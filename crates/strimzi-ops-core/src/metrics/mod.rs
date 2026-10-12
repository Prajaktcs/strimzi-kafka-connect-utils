//! Bounded, per-worker Kafka Connect and Debezium Prometheus collection.
//!
//! Targets are reloaded from `[[workers]]` TOML entries on every collection. Each
//! entry has an `id`, an HTTPS `url`, and optionally a private `bearer_token` and
//! PEM `ca_file` (relative to the targets file). HTTP is rejected, including
//! unauthenticated loopback endpoints; certificate and hostname checks stay enabled.
//! Collection and client destruction must run on a blocking worker thread.

mod collector;
mod failures;
mod parser;
mod streaming;
mod targets;
mod throughput;
mod types;

pub use collector::{MetricsCollector, COLLECTION_INTERVAL, REQUEST_TIMEOUT, STALE_AFTER};
pub use failures::{FailureCounter, FailuresReport, TaskFailures};
pub use parser::parse_prometheus;
pub use streaming::{debezium_streaming, DebeziumStreaming};
pub use throughput::{
    connector_throughput, ConnectorThroughput, TaskThroughput, ThroughputStage, ThroughputState,
    ThroughputValue,
};
pub use types::{MetricLookup, MetricSample, MetricState, MetricsSnapshot, WorkerMetrics};
