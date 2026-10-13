//! Safe, read-only Kafka operations. There is never an assignment/subscription:
//! `OffsetFetch` does not require group membership. `BaseConsumer`'s destructor may
//! internally drain close callbacks, but has no payloads or offsets to consume/
//! commit, and no joined group to leave. Socket/session waits are finite.
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use rdkafka::client::ClientContext;
use rdkafka::config::{ClientConfig, RDKafkaLogLevel};
use rdkafka::consumer::{BaseConsumer, Consumer, ConsumerContext};
use rdkafka::error::KafkaError;
use rdkafka::metadata::Metadata;
use rdkafka::{Offset, TopicPartitionList};

use super::sink_lag::{
    aggregate, partition_reading, safe_identifier, ConnectorLag, Mapping, MAX_PARTITIONS,
};
use super::{MetricState, REQUEST_TIMEOUT};

type TopicMetadata = BTreeMap<String, Result<Vec<i32>, &'static str>>;
const MAX_METADATA_TOPICS: usize = 4096;
const MAX_METADATA_PARTITIONS: usize = 16_384;
const DEADLINE_ERROR: &str = "sink lag collection deadline exceeded";

pub(super) fn collect(
    mappings: &BTreeMap<String, Mapping>,
    bootstrap: &str,
    cluster: &str,
    deadline: Instant,
) -> Vec<ConnectorLag> {
    let metadata = mappings
        .values()
        .find(|mapping| mapping.error.is_none())
        .and_then(|mapping| mapping.group.as_deref())
        .map(|group| {
            remaining(deadline)?;
            let client = offset_client(bootstrap, group)?;
            let metadata = client
                .fetch_metadata(None, remaining(deadline)?)
                .map_err(|_| "cannot fetch Kafka topic metadata")?;
            let observed_cluster = client
                .client()
                .fetch_cluster_id(remaining(deadline)?)
                .ok_or("Kafka cluster identity is unavailable")?;
            if observed_cluster != cluster {
                return Err("collector Kafka cluster differs from Connect cluster");
            }
            decode_metadata(&metadata)
        });
    let mut used_partitions = 0;
    mappings
        .iter()
        .map(|(name, mapping)| {
            let mut reading = mapping.reading(name);
            if mapping.error.is_some() {
                return reading;
            }
            let result = match &metadata {
                Some(Ok(metadata)) => collect_connector(
                    &mut reading,
                    mapping,
                    metadata,
                    bootstrap,
                    deadline,
                    &mut used_partitions,
                ),
                Some(Err(reason)) => Err(*reason),
                None => Err("sink consumer group mapping is unavailable"),
            };
            if let Err(reason) = result {
                reading.state = MetricState::CollectionError;
                reading.total_lag = None;
                reading.error = Some(reason);
            }
            reading
        })
        .collect()
}

fn collect_connector(
    reading: &mut ConnectorLag,
    mapping: &Mapping,
    metadata: &TopicMetadata,
    bootstrap: &str,
    deadline: Instant,
    used_partitions: &mut usize,
) -> Result<(), &'static str> {
    let topics = mapping.topics(metadata.keys())?;
    if topics.is_empty() {
        reading.error = Some("sink subscription matches no known topics");
        return Ok(());
    }
    // Resolve the complete subscribed set before offsets; missing metadata must
    // never produce an apparently complete aggregate of a partial partition set.
    let mut partitions = Vec::new();
    for topic in topics {
        if !safe_identifier(&topic) {
            return Err("cannot publish unsafe topic identity");
        }
        let Some(ids) = metadata.get(&topic) else {
            reading.error = Some("configured sink topic is missing from Kafka metadata");
            return Ok(());
        };
        let ids = ids.as_ref().map_err(|reason| *reason)?;
        if ids.is_empty() {
            reading.error = Some("configured sink topic has no known partitions");
            return Ok(());
        }
        if partitions.len() + ids.len() > MAX_PARTITIONS
            || *used_partitions + partitions.len() + ids.len() > MAX_PARTITIONS
        {
            return Err("sink lag collection exceeds partition limit");
        }
        for id in ids {
            partitions.push((topic.clone(), *id));
        }
    }
    *used_partitions += partitions.len();
    let group = mapping
        .group
        .as_deref()
        .ok_or("sink consumer group mapping is unavailable")?;
    remaining(deadline)?;
    let client = offset_client(bootstrap, group)?;
    let mut request = TopicPartitionList::with_capacity(partitions.len());
    for (topic, partition) in &partitions {
        request.add_partition(topic, *partition);
    }
    // Group-scoped OffsetFetch only: no subscribe/assign/consume/poll/commit calls.
    let offsets = client.committed_offsets(request, remaining(deadline)?);
    for (topic, partition) in partitions {
        let committed = offsets
            .as_ref()
            .ok()
            .and_then(|offsets| offsets.find_partition(&topic, partition))
            .filter(|offset| offset.error().is_ok())
            .and_then(|offset| match offset.offset() {
                Offset::Offset(value) => Some(value),
                _ => None,
            });
        let bounds = remaining(deadline).and_then(|timeout| {
            client
                .fetch_watermarks(&topic, partition, timeout)
                .map_err(|_| "cannot fetch partition log offsets")
        });
        let mut value = partition_reading(topic, partition, committed, bounds);
        if offsets.is_err() {
            value.state = MetricState::CollectionError;
            value.lag = None;
            value.error = Some("cannot fetch sink consumer group committed offsets");
        } else if offsets
            .as_ref()
            .ok()
            .and_then(|offsets| offsets.find_partition(&value.topic, partition))
            .is_some_and(|offset| offset.error().is_err())
        {
            value.state = MetricState::CollectionError;
            value.lag = None;
            value.error = Some("cannot fetch committed partition offset");
        }
        reading.partitions.push(value);
    }
    aggregate(reading);
    Ok(())
}

fn remaining(deadline: Instant) -> Result<Duration, &'static str> {
    remaining_at(deadline, Instant::now())
}

fn remaining_at(deadline: Instant, now: Instant) -> Result<Duration, &'static str> {
    let duration = deadline
        .checked_duration_since(now)
        .ok_or(DEADLINE_ERROR)?
        .min(REQUEST_TIMEOUT);
    if duration < Duration::from_millis(1) {
        return Err(DEADLINE_ERROR);
    }
    Ok(duration)
}

struct QuietContext;

impl ClientContext for QuietContext {
    fn log(&self, _level: RDKafkaLogLevel, _facility: &str, _message: &str) {}

    fn error(&self, _error: KafkaError, _reason: &str) {}

    fn stats_raw(&self, _statistics: &[u8]) {}
}

impl ConsumerContext for QuietContext {}

fn offset_client(bootstrap: &str, group: &str) -> Result<BaseConsumer<QuietContext>, &'static str> {
    ClientConfig::new()
        .set("bootstrap.servers", bootstrap)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .set("enable.auto.offset.store", "false")
        .set("allow.auto.create.topics", "false")
        .set("socket.timeout.ms", "1000")
        .set("socket.connection.setup.timeout.ms", "1000")
        .set("session.timeout.ms", "6000")
        // librdkafka requires receive capacity >= fetch.max.bytes + 512 even
        // for an unassigned client that never issues payload fetches.
        .set("fetch.max.bytes", "1048576")
        .set("receive.message.max.bytes", "2097152")
        .set("log_level", "0")
        .create_with_context(QuietContext)
        .map_err(|_| "cannot initialize sink offset client")
}

fn decode_metadata(metadata: &Metadata) -> Result<TopicMetadata, &'static str> {
    if metadata.topics().len() > MAX_METADATA_TOPICS {
        return Err("Kafka metadata exceeds topic limit");
    }
    let mut decoded = BTreeMap::new();
    let mut total = 0;
    for topic in metadata.topics() {
        total += topic.partitions().len();
        if total > MAX_METADATA_PARTITIONS {
            return Err("Kafka metadata exceeds partition limit");
        }
        let ids = if topic.error().is_some() {
            Err("cannot read subscribed topic metadata")
        } else if topic
            .partitions()
            .iter()
            .any(|partition| partition.id() < 0 || partition.error().is_some())
        {
            Err("cannot read subscribed partition metadata")
        } else {
            let mut ids: Vec<_> = topic
                .partitions()
                .iter()
                .map(rdkafka::metadata::MetadataPartition::id)
                .collect();
            ids.sort_unstable();
            if ids.windows(2).any(|pair| pair[0] == pair[1]) {
                Err("Kafka partition metadata contains duplicate identities")
            } else {
                Ok(ids)
            }
        };
        if decoded.insert(topic.name().to_owned(), ids).is_some() {
            return Err("Kafka metadata contains duplicate topic identities");
        }
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_deadline_never_resets_per_partition() {
        let now = Instant::now();
        assert_eq!(
            remaining_at(now.checked_sub(Duration::from_secs(1)).unwrap(), now),
            Err(DEADLINE_ERROR)
        );
        assert_eq!(remaining_at(now, now), Err(DEADLINE_ERROR));
        assert_eq!(
            remaining_at(now + Duration::from_secs(60), now),
            Ok(REQUEST_TIMEOUT)
        );
        let deadline = now + Duration::from_secs(8);
        assert_eq!(
            remaining_at(deadline, now + Duration::from_secs(7)),
            Ok(Duration::from_secs(1))
        );
    }
}
