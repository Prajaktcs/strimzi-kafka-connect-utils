//! Bounded, per-worker Kafka Connect and Debezium Prometheus collection.
//!
//! Targets are reloaded from `[[workers]]` TOML entries on every collection. Each
//! entry has an `id`, an HTTP(S) `url`, and optionally a private `bearer_token`.
//! Collection and client destruction must run on a blocking worker thread.

mod collector;
mod parser;
mod targets;
mod types;

pub use collector::{MetricsCollector, COLLECTION_INTERVAL, REQUEST_TIMEOUT, STALE_AFTER};
pub use parser::parse_prometheus;
pub use types::{MetricLookup, MetricSample, MetricState, MetricsSnapshot, WorkerMetrics};
