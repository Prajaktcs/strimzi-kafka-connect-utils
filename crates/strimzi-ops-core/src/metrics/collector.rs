use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;

use super::parser::{parse_prometheus, MAX_RESPONSE_BYTES};
use super::targets::{read_roots, read_targets, Target, TargetsError};
use super::{MetricSample, MetricState, MetricsSnapshot, WorkerMetrics};

pub const COLLECTION_INTERVAL: Duration = Duration::from_secs(10);
pub const STALE_AFTER: Duration = Duration::from_secs(30);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

type ScrapeResult = Result<(Vec<MetricSample>, Instant), &'static str>;

/// Server-side cache and independent, bounded collection for every configured
/// worker. At most 32 requests run concurrently, each limited to three seconds
/// and two MiB. The target file itself is limited to 128 KiB.
/// A response may contain at most 10,000 samples and 32,768 label entries, so
/// small-byte, high-cardinality responses cannot inflate decoding memory.
///
/// `new` does not construct a blocking client. Call `collect` only on a blocking
/// thread; clients are constructed and destroyed within each collection round,
/// shared only by workers using the same roots. CA files are reread every round.
/// This cache may safely move back to an async polling owner between calls.
/// Endpoints, credentials, and collection times are private and excluded from
/// `Debug` output and public snapshots.
pub struct MetricsCollector {
    targets_file: Option<PathBuf>,
    workers: BTreeMap<String, StoredWorker>,
    empty_state: MetricState,
}

impl fmt::Debug for MetricsCollector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MetricsCollector")
            .field("workers", &self.workers.len())
            .finish_non_exhaustive()
    }
}

impl MetricsCollector {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            targets_file: path,
            workers: BTreeMap::new(),
            empty_state: MetricState::Unavailable,
        }
    }

    /// Reread membership, scrape workers independently, and return safe data.
    /// A rejected target file retains all last-known worker identities; a valid
    /// file replaces membership, including intentional removals.
    pub fn collect(&mut self) -> MetricsSnapshot {
        let Some(path) = self.targets_file.as_deref() else {
            return self.snapshot();
        };
        let targets = match read_targets(path) {
            Ok(targets) => targets,
            Err(TargetsError::Missing) => {
                self.fail_all("cannot read metrics targets", MetricState::CollectionError);
                return self.snapshot();
            }
            Err(TargetsError::Invalid(reason)) => {
                self.fail_all(reason, MetricState::CollectionError);
                return self.snapshot();
            }
        };
        self.workers
            .retain(|id, _| targets.iter().any(|target| target.id == *id));
        for target in &targets {
            self.workers.entry(target.id.clone()).or_default();
        }
        self.empty_state = MetricState::Unavailable;
        if targets.is_empty() {
            return self.snapshot();
        }
        for (id, result) in scrape_targets(&targets) {
            if let Some(worker) = self.workers.get_mut(id) {
                match result {
                    Ok((samples, collected_at)) => worker.succeed(samples, collected_at),
                    Err(reason) => worker.fail(reason),
                }
            }
        }
        self.snapshot()
    }

    /// Compute current ages and expiration without performing I/O or scraping.
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> MetricsSnapshot {
        let workers: Vec<_> = self
            .workers
            .iter()
            .map(|(id, worker)| worker.snapshot(id, now))
            .collect();
        let state = workers
            .iter()
            .map(|worker| worker.state)
            .max_by_key(|state| match state {
                MetricState::Unavailable => 0,
                MetricState::Available => 1,
                MetricState::Stale => 2,
                MetricState::CollectionError => 3,
            })
            .unwrap_or(self.empty_state);
        MetricsSnapshot { workers, state }
    }

    fn fail_all(&mut self, reason: &'static str, empty_state: MetricState) {
        self.empty_state = empty_state;
        for worker in self.workers.values_mut() {
            worker.fail(reason);
        }
    }
}

struct StoredWorker {
    state: MetricState,
    samples: Arc<Vec<MetricSample>>,
    last_good: Option<Instant>,
    error: Option<&'static str>,
}

impl Default for StoredWorker {
    fn default() -> Self {
        Self {
            state: MetricState::Unavailable,
            samples: Arc::default(),
            last_good: None,
            error: None,
        }
    }
}

impl StoredWorker {
    fn succeed(&mut self, samples: Vec<MetricSample>, now: Instant) {
        if samples.iter().any(|sample| sample.value.is_finite()) {
            self.state = MetricState::Available;
            self.last_good = Some(now);
        } else {
            self.state = MetricState::Unavailable;
            self.last_good = None;
        }
        self.samples = Arc::new(samples);
        self.error = None;
    }

    fn fail(&mut self, reason: &'static str) {
        self.state = if self.last_good.is_some() {
            MetricState::Stale
        } else {
            MetricState::CollectionError
        };
        self.error = Some(reason);
    }

    fn snapshot(&self, id: &str, now: Instant) -> WorkerMetrics {
        let age = self
            .last_good
            .map(|last_good| now.saturating_duration_since(last_good));
        let state =
            if self.state == MetricState::Available && age.is_some_and(|age| age >= STALE_AFTER) {
                MetricState::Stale
            } else {
                self.state
            };
        WorkerMetrics {
            worker: id.to_owned(),
            state,
            samples: Arc::clone(&self.samples),
            age_secs: age.map(|age| age.as_secs()),
            error: self.error.map(str::to_owned),
        }
    }
}

fn scrape_targets(targets: &[Target]) -> Vec<(&str, ScrapeResult)> {
    // A client must never acquire another target's custom trust roots. Reuse
    // only exact CA-file configurations, and drop all clients before returning.
    let mut clients = BTreeMap::new();
    for target in targets {
        clients
            .entry(target.ca_file.as_deref())
            .or_insert_with(|| metrics_client(target));
    }
    thread::scope(|scope| {
        let mut tasks = Vec::with_capacity(targets.len());
        let mut outcomes = Vec::with_capacity(targets.len());
        for target in targets {
            let http = match &clients[&target.ca_file.as_deref()] {
                Ok(http) => http,
                Err(reason) => {
                    outcomes.push((target.id.as_str(), Err(*reason)));
                    continue;
                }
            };
            match thread::Builder::new().spawn_scoped(scope, move || scrape(http, target)) {
                Ok(task) => tasks.push((target.id.as_str(), task)),
                Err(_) => {
                    outcomes.push((target.id.as_str(), Err("cannot start metrics collection")));
                }
            }
        }
        for (id, task) in tasks {
            outcomes.push((
                id,
                task.join().unwrap_or(Err("cannot collect worker metrics")),
            ));
        }
        outcomes
    })
}

fn metrics_client(target: &Target) -> Result<Client, &'static str> {
    let roots = target
        .ca_file
        .as_deref()
        .map_or_else(|| Ok(Vec::new()), read_roots)?;
    let mut builder = Client::builder()
        .tls_built_in_root_certs(target.ca_file.is_none())
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    for root in roots {
        builder = builder.add_root_certificate(root);
    }
    builder
        .build()
        .map_err(|_| "cannot initialize metrics collection")
}

fn scrape(http: &Client, target: &Target) -> ScrapeResult {
    let mut request = http.get(target.url.clone());
    if let Some(authorization) = &target.authorization {
        request = request.header(AUTHORIZATION, authorization.clone());
    }
    let response = request.send().map_err(|error| {
        if error.is_timeout() {
            "cannot fetch metrics (request timed out)"
        } else {
            "cannot fetch metrics"
        }
    })?;
    if !response.status().is_success() {
        return Err(if response.status().is_redirection() {
            "cannot fetch metrics (redirects are disabled)"
        } else if matches!(response.status().as_u16(), 401 | 403) {
            "cannot authenticate metrics request"
        } else {
            "cannot fetch metrics (HTTP error)"
        });
    }
    let limit = u64::try_from(MAX_RESPONSE_BYTES).unwrap_or(u64::MAX - 1);
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err("metrics response exceeds size limit");
    }
    let mut bytes = Vec::new();
    response
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read metrics response")?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("metrics response exceeds size limit");
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| "cannot decode metrics response")?;
    let mut samples = parse_prometheus(text).map_err(|_| "cannot parse metrics response")?;
    // Do not publish a malicious exporter's reflection of private configuration.
    // Token validation makes ordinary names and values unable to contain a
    // token, so only deliberate reflection can match here.
    if let Some(token) = target
        .authorization
        .as_ref()
        .and_then(|authorization| authorization.to_str().ok())
        .and_then(|authorization| authorization.strip_prefix("Bearer "))
    {
        let reflected = samples.iter().any(|sample| {
            sample.name.contains(token)
                || sample
                    .labels
                    .iter()
                    .any(|(name, value)| name.contains(token) || value.contains(token))
        });
        if reflected {
            return Err("cannot publish credential-bearing metrics response");
        }
    }
    // Do not publish a hostile exporter's reflection of the private endpoint.
    if text.contains(target.url.as_str()) {
        return Err("cannot publish credential-bearing metrics response");
    }
    if samples.iter().any(|sample| {
        sample.name == "jmx_scrape_error" && (!sample.value.is_finite() || sample.value.abs() > 0.0)
    }) {
        return Err("cannot collect metrics from worker exporter");
    }
    samples.retain(|sample| {
        sample.name.starts_with("kafka_connect_") || sample.name.starts_with("debezium_")
    });
    Ok((samples, Instant::now()))
}

#[cfg(test)]
#[path = "collector_tests.rs"]
mod tests;
