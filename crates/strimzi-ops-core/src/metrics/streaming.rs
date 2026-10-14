use std::collections::BTreeMap;

use super::series::Series;
use super::{MetricLookup, MetricState, MetricsSnapshot, WorkerMetrics};

const PREFIX: &str = "debezium_";
// The first six suffixes are the projected measurements; the rest only seed
// identities. Exact suffixes preserve domains containing underscores.
const SUFFIXES: [&str; 11] = [
    "_connected",
    "_milli_seconds_behind_source",
    "_total_number_of_events_seen_total",
    "_total_number_of_create_events_seen_total",
    "_total_number_of_update_events_seen_total",
    "_total_number_of_delete_events_seen_total",
    "_snapshot_completed",
    "_snapshot_running",
    "_snapshot_skipped",
    "_queue_total_capacity",
    "_queue_remaining_capacity",
];
const MEASUREMENT_COUNT: usize = 6;

/// Independent Debezium streaming readings for one exported task identity.
///
/// Availability is per metric, not a capability claim about the database plugin.
/// Counters retain the task's exported lifetime totals without aggregation or
/// subtraction. Unavailable and collection-error readings never contain values.
#[derive(Debug, Clone, PartialEq)]
pub struct DebeziumStreaming {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub domain: String,
    pub server: String,
    pub connected: MetricLookup,
    pub delay_ms: MetricLookup,
    pub events_total: MetricLookup,
    pub creates_total: MetricLookup,
    pub updates_total: MetricLookup,
    pub deletes_total: MetricLookup,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Identity<'a> {
    connector: &'a str,
    task: &'a str,
    domain: &'a str,
    server: &'a str,
}

struct WorkerInventory<'a> {
    worker: &'a WorkerMetrics,
    measurements: [Series<'a>; MEASUREMENT_COUNT],
}

/// Project tagged Debezium streaming measurements without guessing connector
/// identity or combining tasks, workers, domains, servers, or contexts.
///
/// Exact metric suffixes identify the domain: connected, milliseconds behind
/// source, the four event counters, snapshot completed/running/skipped, and queue
/// total/remaining capacity. The snapshot and queue metrics only seed identities;
/// unknown attributes alone cannot safely identify a domain and are ignored.
/// All four nonempty connector/task/server/context labels are required. Snapshot
/// samples seed missing streaming readings but never supply their values.
///
/// Only context="streaming" supplies readings. Extra dimensions, duplicate series
/// or worker ownership, negative/nonfinite values, and nonboolean connected
/// readings are unavailable. In particular, negative source delay (including
/// Debezium's -1 sentinel or clock skew) is never converted to a healthy zero.
/// Stale valid measurements retain their values and stale state. Output is
/// ordered by connector, task, domain, server, and worker.
pub fn debezium_streaming(snapshot: &MetricsSnapshot) -> Vec<DebeziumStreaming> {
    let mut identities: BTreeMap<Identity<'_>, BTreeMap<(&str, usize), WorkerInventory<'_>>> =
        BTreeMap::new();
    for (worker_index, worker) in snapshot.workers.iter().enumerate() {
        for sample in worker.samples.iter() {
            let Some(name) = sample.name.strip_prefix(PREFIX) else {
                continue;
            };
            let Some((index, domain)) = SUFFIXES.iter().enumerate().find_map(|(index, suffix)| {
                name.strip_suffix(*suffix)
                    .filter(|domain| !domain.is_empty())
                    .map(|domain| (index, domain))
            }) else {
                continue;
            };
            let label = |name: &str| {
                sample
                    .labels
                    .get(name)
                    .map(String::as_str)
                    .filter(|value| !value.is_empty())
            };
            let (Some(connector), Some(task), Some(server), Some(context)) = (
                label("connector"),
                label("task"),
                label("server"),
                label("context"),
            ) else {
                continue;
            };
            let identity = Identity {
                connector,
                task,
                domain,
                server,
            };
            // Retain the index so repeated worker names cannot choose a winner.
            let inventory = identities
                .entry(identity)
                .or_default()
                .entry((&worker.worker, worker_index))
                .or_insert_with(|| WorkerInventory {
                    worker,
                    measurements: [Series::Missing; MEASUREMENT_COUNT],
                });
            if context == "streaming" && index < MEASUREMENT_COUNT {
                inventory.measurements[index].observe(sample);
            }
        }
    }
    identities
        .into_iter()
        .flat_map(|(identity, workers)| {
            let ambiguous = workers.len() > 1;
            workers.into_values().map(move |inventory| {
                let measurements = inventory.measurements;
                let lookup = |index: usize| {
                    if ambiguous {
                        absent(MetricState::Unavailable)
                    } else {
                        measurements[index].lookup(inventory.worker, index == 0, 4)
                    }
                };
                DebeziumStreaming {
                    connector: identity.connector.to_owned(),
                    task: identity.task.to_owned(),
                    worker: inventory.worker.worker.clone(),
                    domain: identity.domain.to_owned(),
                    server: identity.server.to_owned(),
                    connected: lookup(0),
                    delay_ms: lookup(1),
                    events_total: lookup(2),
                    creates_total: lookup(3),
                    updates_total: lookup(4),
                    deletes_total: lookup(5),
                }
            })
        })
        .collect()
}

fn absent(state: MetricState) -> MetricLookup {
    MetricLookup { state, value: None }
}

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod tests;
