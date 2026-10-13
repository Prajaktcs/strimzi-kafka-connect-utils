use std::collections::BTreeMap;
#[cfg(any(feature = "kafka", test))]
use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
#[cfg(feature = "kafka")]
use std::time::Duration;
use std::time::Instant;

#[cfg(any(feature = "kafka", test))]
use regex::{Regex, RegexBuilder};
#[cfg(any(feature = "kafka", test))]
use serde_json::{Map, Value};

#[cfg(feature = "kafka")]
use crate::connect::ConnectClient;

#[cfg(feature = "kafka")]
use super::REQUEST_TIMEOUT;
use super::{MetricState, STALE_AFTER};

#[cfg(feature = "kafka")]
pub(super) const ROUND_TIMEOUT: Duration = Duration::from_secs(8);
#[cfg(any(feature = "kafka", test))]
pub(super) const MAX_CONNECTORS: usize = 128;
#[cfg(any(feature = "kafka", test))]
pub(super) const MAX_PARTITIONS: usize = 1024;

/// Safe offset measurements only: endpoints, connector configuration and Kafka
/// errors are never included. An aggregate exists only for a complete partition set.
#[derive(Debug, Clone)]
pub struct SinkLagSnapshot {
    pub state: MetricState,
    pub connectors: Arc<Vec<ConnectorLag>>,
    pub age_secs: Option<u64>,
    pub error: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct ConnectorLag {
    pub connector: String,
    pub group: Option<String>,
    pub state: MetricState,
    pub total_lag: Option<u64>,
    pub partitions: Vec<PartitionLag>,
    pub error: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct PartitionLag {
    pub topic: String,
    pub partition: i32,
    pub committed: Option<i64>,
    pub end: Option<i64>,
    pub lag: Option<u64>,
    pub state: MetricState,
    pub error: Option<&'static str>,
}

/// Read-only, bounded sink-group collection. Construct and drop all blocking
/// HTTP/Kafka clients within `collect`, which must run on a blocking thread.
/// Makes no subscribe, assign, poll, consume, commit or group-join calls.
/// librdkafka may internally drain close callbacks when destroying an unassigned
/// client; auto commit/store are disabled and there is no subscription or payload.
/// Regex subscriptions support only ASCII literals, `.`, `*`, `+`, `?`, `|`,
/// ordinary parentheses, `^`, `$`, and escaped dots/hyphens, with full-topic
/// matching. Java classes, flags, lookarounds, backreferences, counted/lazy/
/// possessive repetitions and unresolved config-provider expressions are rejected.
pub struct SinkLagCollector {
    connect_url: Option<String>,
    bootstrap_servers: Option<String>,
    #[cfg(any(feature = "kafka", test))]
    mappings: BTreeMap<String, Mapping>,
    measured_at: BTreeMap<String, Instant>,
    cached: SinkLagSnapshot,
}

impl fmt::Debug for SinkLagCollector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SinkLagCollector")
            .field("connectors", &self.cached.connectors.len())
            .field(
                "configured",
                &(self.connect_url.is_some() && self.bootstrap_servers.is_some()),
            )
            .finish_non_exhaustive()
    }
}

impl SinkLagCollector {
    pub fn new(connect_url: Option<String>, bootstrap_servers: Option<String>) -> Self {
        Self {
            connect_url,
            bootstrap_servers,
            #[cfg(any(feature = "kafka", test))]
            mappings: BTreeMap::new(),
            measured_at: BTreeMap::new(),
            cached: SinkLagSnapshot {
                state: MetricState::Unavailable,
                connectors: Arc::default(),
                age_secs: None,
                error: Some("sink lag requires Connect and Kafka bootstrap configuration"),
            },
        }
    }

    pub fn collect(&mut self) -> SinkLagSnapshot {
        #[cfg(not(feature = "kafka"))]
        {
            self.cached.state = MetricState::Unavailable;
            self.cached.error = Some("sink lag is unavailable without Kafka support");
        }
        #[cfg(feature = "kafka")]
        {
            self.collect_kafka();
        }
        self.snapshot()
    }

    #[cfg(feature = "kafka")]
    fn collect_kafka(&mut self) {
        let (Some(connect_url), Some(bootstrap)) = (
            self.connect_url.as_deref(),
            self.bootstrap_servers.as_deref(),
        ) else {
            return;
        };
        if bootstrap.trim().is_empty() {
            self.fail_all("sink lag requires Kafka bootstrap configuration");
            return;
        }
        let deadline = Instant::now() + ROUND_TIMEOUT;
        let discovery = (|| {
            let client = ConnectClient::with_timeout(connect_url, REQUEST_TIMEOUT)
                .map_err(|_| "cannot initialize sink connector discovery")?;
            let info = client
                .get_all_connectors_status()
                .map_err(|_| "cannot discover sink connectors")?;
            let mappings = discover(&info, bootstrap)?;
            let cluster = client
                .get_cluster_info()
                .map_err(|_| "cannot verify Connect Kafka cluster")
                .and_then(|info| {
                    info.kafka_cluster_id
                        .filter(|id| !id.is_empty())
                        .ok_or("Connect Kafka cluster identity is unavailable")
                });
            Ok::<_, &'static str>((mappings, cluster))
        })();
        let (mappings, cluster) = match discovery {
            Ok(result) => result,
            Err(reason) => {
                self.fail_all(reason);
                return;
            }
        };
        let results = match cluster {
            Ok(cluster) => super::sink_lag_kafka::collect(&mappings, bootstrap, &cluster, deadline),
            Err(reason) => mappings
                .iter()
                .map(|(name, mapping)| {
                    let mut reading = mapping.reading(name);
                    if reading.error.is_none() {
                        reading.state = MetricState::CollectionError;
                        reading.error = Some(reason);
                    }
                    reading
                })
                .collect(),
        };
        self.apply(mappings, results, Instant::now());
    }

    pub fn snapshot(&self) -> SinkLagSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> SinkLagSnapshot {
        let mut snapshot = self.cached.clone();
        snapshot.age_secs = self
            .measured_at
            .values()
            .map(|at| now.saturating_duration_since(*at).as_secs())
            .max();
        let expired = |connector: &ConnectorLag| {
            connector.state != MetricState::Stale
                && self
                    .measured_at
                    .get(&connector.connector)
                    .is_some_and(|at| now.saturating_duration_since(*at) >= STALE_AFTER)
        };
        if snapshot.connectors.iter().any(expired) {
            for connector in Arc::make_mut(&mut snapshot.connectors) {
                if expired(connector) {
                    mark_stale(connector);
                }
            }
            snapshot.state = overall_state(&snapshot.connectors);
        }
        snapshot
    }

    #[cfg(any(feature = "kafka", test))]
    fn fail_all(&mut self, reason: &'static str) {
        for connector in Arc::make_mut(&mut self.cached.connectors) {
            if self.measured_at.contains_key(&connector.connector) {
                mark_stale(connector);
            } else {
                connector.state = MetricState::CollectionError;
            }
            connector.error = Some(reason);
        }
        self.cached.state = if self.cached.connectors.is_empty() {
            MetricState::CollectionError
        } else {
            overall_state(&self.cached.connectors)
        };
        self.cached.error = Some(reason);
    }

    #[cfg(any(feature = "kafka", test))]
    fn apply(
        &mut self,
        mappings: BTreeMap<String, Mapping>,
        mut readings: Vec<ConnectorLag>,
        now: Instant,
    ) {
        self.measured_at.retain(|name, _| {
            mappings
                .get(name)
                .is_some_and(|mapping| self.mappings.get(name) == Some(mapping))
        });
        for reading in &mut readings {
            let same_mapping =
                mappings.get(&reading.connector) == self.mappings.get(&reading.connector);
            let previous = self
                .cached
                .connectors
                .iter()
                .find(|old| old.connector == reading.connector && has_offsets(old));
            // A known changed partition set is also a new measurement identity.
            let same_partitions = previous.is_some_and(|old| {
                reading.partitions.is_empty()
                    || old
                        .partitions
                        .iter()
                        .map(|p| (&p.topic, p.partition))
                        .eq(reading.partitions.iter().map(|p| (&p.topic, p.partition)))
            });
            if reading.state == MetricState::CollectionError && same_mapping && same_partitions {
                if let Some(previous) = previous {
                    let reason = reading.error;
                    *reading = previous.clone();
                    mark_stale(reading);
                    reading.error = reason;
                }
            } else if has_offsets(reading) {
                self.measured_at.insert(reading.connector.clone(), now);
            } else {
                self.measured_at.remove(&reading.connector);
            }
        }
        self.mappings = mappings;
        self.cached = SinkLagSnapshot {
            state: overall_state(&readings),
            connectors: Arc::new(readings),
            age_secs: None,
            error: None,
        };
    }
}

#[cfg(any(feature = "kafka", test))]
fn has_offsets(connector: &ConnectorLag) -> bool {
    connector
        .partitions
        .iter()
        .any(|partition| partition.committed.is_some() || partition.end.is_some())
}

fn mark_stale(connector: &mut ConnectorLag) {
    connector.state = MetricState::Stale;
    for partition in &mut connector.partitions {
        if partition.state == MetricState::Available {
            partition.state = MetricState::Stale;
        }
    }
}

fn overall_state(connectors: &[ConnectorLag]) -> MetricState {
    connectors
        .iter()
        .map(|connector| connector.state)
        .max_by_key(|state| match state {
            MetricState::Available => 0,
            MetricState::Unavailable => 1,
            MetricState::Stale => 2,
            MetricState::CollectionError => 3,
        })
        .unwrap_or(MetricState::Unavailable)
}

#[cfg(any(feature = "kafka", test))]
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Subscription {
    Topics(Vec<String>),
    Regex(String),
}

#[cfg(any(feature = "kafka", test))]
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Mapping {
    pub group: Option<String>,
    pub subscription: Option<Subscription>,
    pub error: Option<&'static str>,
}

#[cfg(any(feature = "kafka", test))]
impl Mapping {
    pub(super) fn reading(&self, connector: &str) -> ConnectorLag {
        ConnectorLag {
            connector: connector.to_owned(),
            group: self.group.clone(),
            state: MetricState::Unavailable,
            total_lag: None,
            partitions: Vec::new(),
            error: self.error,
        }
    }
}

#[cfg(any(feature = "kafka", test))]
pub(super) fn safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 249
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && !matches!(value, "." | "..")
}

#[cfg(any(feature = "kafka", test))]
pub(super) fn discover(
    connectors: &Map<String, Value>,
    bootstrap: &str,
) -> Result<BTreeMap<String, Mapping>, &'static str> {
    if connectors.len() > MAX_CONNECTORS {
        return Err("sink connector discovery exceeds connector limit");
    }
    let mut mappings = BTreeMap::new();
    for (name, value) in connectors {
        if !safe_identifier(name) {
            return Err("cannot publish unsafe connector identity");
        }
        let info = value
            .get("info")
            .ok_or("sink connector information is incomplete")?;
        let kind = info
            .get("type")
            .and_then(Value::as_str)
            .ok_or("sink connector type is unavailable")?;
        if kind != "sink" {
            continue;
        }
        let config = info
            .get("config")
            .and_then(Value::as_object)
            .ok_or("sink connector configuration is unavailable")?;
        mappings.insert(name.clone(), resolve_mapping(name, config, bootstrap));
    }
    Ok(mappings)
}

#[cfg(any(feature = "kafka", test))]
fn resolve_mapping(name: &str, config: &Map<String, Value>, bootstrap: &str) -> Mapping {
    let mut mapping = Mapping {
        group: None,
        subscription: None,
        error: None,
    };
    let result = (|| {
        let group = match config.get("consumer.override.group.id") {
            Some(value) => value
                .as_str()
                .ok_or("sink group override is invalid")?
                .to_owned(),
            None => format!("connect-{name}"),
        };
        if !safe_identifier(&group) {
            return Err("sink group identity is invalid or unresolved");
        }
        mapping.group = Some(group);
        if config.keys().any(|key| {
            key.starts_with("consumer.override.sasl.")
                || key.starts_with("consumer.override.ssl.")
                || matches!(
                    key.as_str(),
                    "consumer.override.security.protocol" | "consumer.override.isolation.level"
                )
        }) {
            return Err("sink consumer security or isolation overrides are unsupported");
        }
        if let Some(value) = config.get("consumer.override.bootstrap.servers") {
            let servers = value.as_str().ok_or("sink bootstrap override is invalid")?;
            if servers.contains("${") || canonical_servers(servers) != canonical_servers(bootstrap)
            {
                return Err("sink consumer bootstrap override differs from collector cluster");
            }
        }
        let topics = config
            .get("topics")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty());
        let pattern = config
            .get("topics.regex")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        mapping.subscription = Some(match (topics, pattern) {
            (Some(_), Some(_)) => {
                return Err("sink topics and topics.regex cannot both be configured")
            }
            (Some(topics), None) => {
                let topics: BTreeSet<_> = topics
                    .split(',')
                    .map(str::trim)
                    .map(str::to_owned)
                    .collect();
                if topics.len() > MAX_PARTITIONS
                    || topics.iter().any(|topic| !safe_identifier(topic))
                {
                    return Err("sink topic configuration is invalid or exceeds limits");
                }
                Subscription::Topics(topics.into_iter().collect())
            }
            (None, Some(pattern)) => {
                topic_regex(pattern)?;
                Subscription::Regex(pattern.to_owned())
            }
            (None, None) => return Err("sink topic subscription is unavailable"),
        });
        Ok(())
    })();
    mapping.error = result.err();
    mapping
}

#[cfg(any(feature = "kafka", test))]
fn canonical_servers(servers: &str) -> BTreeSet<String> {
    servers
        .split(',')
        .map(|server| server.trim().to_ascii_lowercase())
        .collect()
}

#[cfg(any(feature = "kafka", test))]
pub(super) fn topic_regex(pattern: &str) -> Result<Regex, &'static str> {
    const REASON: &str = "sink topics.regex uses unsupported or invalid Java regex syntax";
    if pattern.is_empty() || pattern.len() > 1024 || pattern.contains("(?") {
        return Err(REASON);
    }
    let mut bytes = pattern.bytes();
    let mut last_quantifier = false;
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            if !matches!(bytes.next(), Some(b'.' | b'-')) {
                return Err(REASON);
            }
            last_quantifier = false;
        } else {
            let quantifier = b"*+?".contains(&byte);
            if (!byte.is_ascii_alphanumeric() && !b"._-*+?|()^$".contains(&byte))
                || (quantifier && last_quantifier)
            {
                return Err(REASON);
            }
            last_quantifier = quantifier;
        }
    }
    RegexBuilder::new(&format!("\\A(?:{pattern})\\z"))
        .size_limit(64 * 1024)
        .build()
        .map_err(|_| REASON)
}

#[cfg(any(feature = "kafka", test))]
pub(super) fn partition_reading(
    topic: String,
    partition: i32,
    committed: Option<i64>,
    bounds: Result<(i64, i64), &'static str>,
) -> PartitionLag {
    let mut reading = PartitionLag {
        topic,
        partition,
        committed,
        end: bounds.as_ref().ok().map(|(_, high)| *high),
        lag: None,
        state: MetricState::Unavailable,
        error: None,
    };
    reading.error = match bounds {
        Err(reason) => {
            reading.state = MetricState::CollectionError;
            Some(reason)
        }
        Ok((low, high)) => match committed {
            None => Some("consumer group has no committed offset for partition"),
            Some(commit) if low < 0 || high < low || commit < 0 => {
                Some("partition offsets are invalid")
            }
            Some(commit) if commit < low => Some("committed offset is below retained log start"),
            Some(commit) if commit > high => Some("committed offset exceeds sampled log end"),
            Some(commit) => {
                reading.lag = u64::try_from(high - commit).ok();
                reading.state = MetricState::Available;
                None
            }
        },
    };
    reading
}

#[cfg(any(feature = "kafka", test))]
pub(super) fn aggregate(reading: &mut ConnectorLag) {
    if reading.partitions.is_empty() {
        reading.error = Some("sink subscription has no known partitions");
        return;
    }
    reading.state = overall_partition_state(&reading.partitions);
    reading.total_lag = reading
        .partitions
        .iter()
        .try_fold(0_u64, |total, partition| {
            if partition.state != MetricState::Available {
                return None;
            }
            total.checked_add(partition.lag?)
        });
    if reading.total_lag.is_none() {
        reading.error = Some("sink lag aggregate requires every subscribed partition");
        if reading.state == MetricState::Available {
            reading.state = MetricState::Unavailable;
            reading.error = Some("sink lag aggregate exceeds integer range");
        }
    }
}

#[cfg(any(feature = "kafka", test))]
fn overall_partition_state(partitions: &[PartitionLag]) -> MetricState {
    if partitions
        .iter()
        .any(|partition| partition.state == MetricState::CollectionError)
    {
        MetricState::CollectionError
    } else if partitions
        .iter()
        .all(|partition| partition.state == MetricState::Available)
    {
        MetricState::Available
    } else {
        MetricState::Unavailable
    }
}

#[cfg(test)]
#[path = "sink_lag_tests.rs"]
mod tests;
