//! Queue and backpressure projections over collected metric samples.
//!
//! Debezium rows expose the connector's event queue per connector/task/server/
//! context identity: total and remaining record capacity, derived utilization,
//! and byte-size measurements when the connector configures byte limits.
//! Kafka Connect rows expose per-task active-record counts for source and sink
//! tasks; sink rows also carry the maximum Kafka consumer lag, which is never
//! presented as active records.
//!
//! Queue capacities and active-record counts are not summed across tasks:
//! utilization compares one queue with itself, and active records are a
//! point-in-time pipeline depth whose task boundaries must stay visible.
//! Grouping by connector is a display concern only.

use std::collections::BTreeMap;

use super::series::Series;
use super::{MetricLookup, MetricState, MetricsSnapshot, WorkerMetrics};

const DEBEZIUM_PREFIX: &str = "debezium_";
// Debezium queue attributes exist in both the snapshot and streaming contexts.
const QUEUE_SUFFIXES: [&str; 4] = [
    "_queue_total_capacity",
    "_queue_remaining_capacity",
    "_max_queue_size_in_bytes",
    "_current_queue_size_in_bytes",
];
const QUEUE_TOTAL_CAPACITY: usize = 0;
const QUEUE_REMAINING_CAPACITY: usize = 1;
const QUEUE_MAX_BYTES: usize = 2;
const QUEUE_CURRENT_BYTES: usize = 3;

const SOURCE_TASK_PREFIX: &str = "kafka_connect_source_task_";
const SINK_TASK_PREFIX: &str = "kafka_connect_sink_task_";
const SOURCE_SUFFIXES: [&str; 3] = [
    "source_record_active_count",
    "source_record_active_count_max",
    "source_record_active_count_avg",
];
const SINK_SUFFIXES: [&str; 4] = [
    "sink_record_active_count",
    "sink_record_active_count_max",
    "sink_record_active_count_avg",
    "sink_record_lag_max",
];

/// Whether a backpressure-alertable Kafka Connect row reads from the database
/// log or from Kafka.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskKind {
    Source,
    Sink,
}

impl TaskKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Sink => "sink",
        }
    }
}

/// Debezium queue readings for one connector/task/server/context identity.
///
/// Utilization is derived only from compatible values of the same row. Byte
/// readings are exactly what the exporter sends: without
/// `max.queue.size.in.bytes` a real exported zero or a missing family is
/// kept as such; only the derived byte utilization additionally requires a
/// positive maximum. Unavailable and collection-error readings never contain
/// a value. Stale workers keep their last good values with stale state.
#[derive(Debug, Clone, PartialEq)]
pub struct DebeziumQueue {
    pub connector: String,
    pub task: String,
    pub server: String,
    pub context: String,
    pub worker: String,
    pub domain: String,
    pub total_capacity: MetricLookup,
    pub remaining_capacity: MetricLookup,
    /// `(total - remaining) / total`; unavailable for missing, zero, or
    /// inconsistent capacity instead of an invented fraction.
    pub utilization: MetricLookup,
    pub max_queue_bytes: MetricLookup,
    pub current_queue_bytes: MetricLookup,
    /// `current / max`; unavailable unless both exist and max exceeds zero.
    pub bytes_utilization: MetricLookup,
}

/// Active-record readings for one Kafka Connect source or sink task.
///
/// Active records are records the task holds in its pipeline without having
/// fully committed them downstream. They are a pipeline depth, not Kafka
/// consumer lag. The lag maximum belongs only to sink tasks, which consume
/// Kafka; source readings keep it structurally unavailable.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectBackpressure {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub kind: TaskKind,
    pub active_records: MetricLookup,
    /// Windowed maximum of the gauge (Kafka's sampled `Max` stat, which can
    /// decay as samples expire), not a lifetime or current depth.
    pub active_records_max: MetricLookup,
    /// Windowed average of the gauge (Kafka's sampled `Avg` stat), not a
    /// backlog.
    pub active_records_avg: MetricLookup,
    /// Kafka consumer-group lag in records; sink tasks only.
    pub consumer_lag_max: MetricLookup,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct DebeziumIdentity<'a> {
    connector: &'a str,
    task: &'a str,
    domain: &'a str,
    server: &'a str,
    context: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct ConnectIdentity<'a> {
    kind: TaskKind,
    connector: &'a str,
    task: &'a str,
}

fn absent(state: MetricState) -> MetricLookup {
    MetricLookup { state, value: None }
}

/// The most severe readability of two measurements feeding one derived value.
fn most_severe(first: MetricState, second: MetricState) -> MetricState {
    match first {
        MetricState::CollectionError => MetricState::CollectionError,
        MetricState::Unavailable => MetricState::Unavailable,
        MetricState::Stale => match second {
            MetricState::Available | MetricState::Stale => MetricState::Stale,
            state => state,
        },
        MetricState::Available => second,
    }
}

/// Utilization from a capacity and an occupancy lookup over the same row.
///
/// Both inputs must carry values from the same reading generation. Zero usable
/// capacity, negative occupancy, occupancy above capacity, or a nonfinite
/// fraction yields unavailable instead of a clamped, averaged, or zero value.
fn utilization(capacity: MetricLookup, occupancy: MetricLookup) -> MetricLookup {
    let (Some(capacity_value), Some(occupancy_value)) = (capacity.value, occupancy.value) else {
        return absent(most_severe(capacity.state, occupancy.state));
    };
    if capacity_value <= 0.0 || occupancy_value < 0.0 || occupancy_value > capacity_value {
        return absent(MetricState::Unavailable);
    }
    let share = occupancy_value / capacity_value;
    let value = share.is_finite().then_some(share);
    MetricLookup {
        state: match value {
            Some(_) => most_severe(capacity.state, occupancy.state),
            None => MetricState::Unavailable,
        },
        value,
    }
}

/// The queued share of a record-capacity pair, or the byte pair directly.
///
/// Record capacity reports remaining free capacity, so in-use is derived as
/// `total - remaining`; incoherent pairs stay unknown.
fn queued_share(total: MetricLookup, remaining: MetricLookup) -> MetricLookup {
    match (total.value, remaining.value) {
        (Some(total_capacity), Some(remaining_capacity))
            if remaining_capacity <= total_capacity =>
        {
            MetricLookup {
                state: most_severe(total.state, remaining.state),
                value: Some(total_capacity - remaining_capacity),
            }
        }
        (Some(total_capacity), Some(remaining_capacity)) if remaining_capacity > total_capacity => {
            absent(MetricState::Unavailable)
        }
        _ => absent(most_severe(total.state, remaining.state)),
    }
}

struct QueueInventory<'a> {
    worker: &'a WorkerMetrics,
    series: [Series<'a>; QUEUE_SUFFIXES.len()],
}

struct ActiveInventory<'a> {
    worker: &'a WorkerMetrics,
    series: [Series<'a>; SOURCE_SUFFIXES.len()],
    lag: Series<'a>,
}

/// Project Debezium queue capacity, utilization, and byte measurements without
/// guessing connector identity, combining contexts, or summing queues.
///
/// Each row is one connector/task/domain/server/context identity as exported by
/// one worker. Exactly the connector, task, server, and context labels are
/// required; extra dimensions (for example custom metric tags), duplicate
/// series, duplicate worker claims, and negative or nonfinite values produce
/// unavailable readings that never imitate zero. Utilization is derived per row
/// so capacity semantics cannot leak across tasks. The snapshot and streaming
/// contexts expose the same underlying queue, so both keep separate labeled
/// rows instead of silently choosing a winner. Output is ordered by connector,
/// task, domain, server, context, and worker.
pub fn debezium_queues(snapshot: &MetricsSnapshot) -> Vec<DebeziumQueue> {
    let mut identities: BTreeMap<
        DebeziumIdentity<'_>,
        BTreeMap<(&str, usize), QueueInventory<'_>>,
    > = BTreeMap::new();
    for (worker_index, worker) in snapshot.workers.iter().enumerate() {
        for sample in worker.samples.iter() {
            let Some(name) = sample.name.strip_prefix(DEBEZIUM_PREFIX) else {
                continue;
            };
            let Some((index, domain)) =
                QUEUE_SUFFIXES
                    .iter()
                    .enumerate()
                    .find_map(|(index, suffix)| {
                        name.strip_suffix(*suffix)
                            .filter(|domain| !domain.is_empty())
                            .map(|domain| (index, domain))
                    })
            else {
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
            let identity = DebeziumIdentity {
                connector,
                task,
                domain,
                server,
                context,
            };
            // Retain the index so repeated worker names cannot elect a winner.
            let inventory = identities
                .entry(identity)
                .or_default()
                .entry((&worker.worker, worker_index))
                .or_insert_with(|| QueueInventory {
                    worker,
                    series: [Series::Missing; QUEUE_SUFFIXES.len()],
                });
            inventory.series[index].observe(sample);
        }
    }
    identities
        .into_iter()
        .flat_map(|(identity, workers)| {
            let ambiguous = workers.len() > 1;
            workers
                .into_values()
                .map(move |inventory| queue_row(identity, ambiguous, inventory))
        })
        .collect()
}

/// One projected queue row; ambiguous ownership keeps identity without values.
fn queue_row(
    identity: DebeziumIdentity<'_>,
    ambiguous: bool,
    inventory: QueueInventory<'_>,
) -> DebeziumQueue {
    let QueueInventory { worker, series } = inventory;
    let lookup = |series: Series<'_>| {
        if ambiguous {
            absent(MetricState::Unavailable)
        } else {
            series.lookup(worker, false, 4)
        }
    };
    let total_capacity = lookup(series[QUEUE_TOTAL_CAPACITY]);
    let remaining_capacity = lookup(series[QUEUE_REMAINING_CAPACITY]);
    let queued = queued_share(total_capacity, remaining_capacity);
    let max_queue_bytes = lookup(series[QUEUE_MAX_BYTES]);
    let current_queue_bytes = lookup(series[QUEUE_CURRENT_BYTES]);
    DebeziumQueue {
        connector: identity.connector.to_owned(),
        task: identity.task.to_owned(),
        server: identity.server.to_owned(),
        context: identity.context.to_owned(),
        worker: worker.worker.clone(),
        domain: identity.domain.to_owned(),
        total_capacity,
        remaining_capacity,
        utilization: utilization(total_capacity, queued),
        max_queue_bytes,
        current_queue_bytes,
        bytes_utilization: utilization(max_queue_bytes, current_queue_bytes),
    }
}

/// Project Kafka Connect active-record counts for source and sink tasks
/// without combining tasks, workers, or kinds.
///
/// The exact metric prefix selects source or sink readings; every other
/// measurement is ignored rather than being guessed into a row. Exactly the
/// connector and task labels are required. Duplicated series, duplicate worker
/// ownership, extra dimensions, and negative or nonfinite values are
/// unavailable, and collection errors stay visible without values. Output is
/// ordered by kind, connector, task, and worker.
pub fn connect_backpressure(snapshot: &MetricsSnapshot) -> Vec<ConnectBackpressure> {
    let mut identities: BTreeMap<
        ConnectIdentity<'_>,
        BTreeMap<(&str, usize), ActiveInventory<'_>>,
    > = BTreeMap::new();
    for (worker_index, worker) in snapshot.workers.iter().enumerate() {
        for sample in worker.samples.iter() {
            let Some((prefix, index)) = (match (
                sample
                    .name
                    .strip_prefix(SOURCE_TASK_PREFIX)
                    .and_then(|suffix| SOURCE_SUFFIXES.iter().position(|known| *known == suffix))
                    .map(|index| (SOURCE_TASK_PREFIX, index)),
                sample
                    .name
                    .strip_prefix(SINK_TASK_PREFIX)
                    .and_then(|suffix| SINK_SUFFIXES.iter().position(|known| *known == suffix))
                    .map(|index| (SINK_TASK_PREFIX, index)),
            ) {
                (Some(matched), None) | (None, Some(matched)) => Some(matched),
                _ => None,
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
            let (Some(connector), Some(task)) = (label("connector"), label("task")) else {
                continue;
            };
            let identity = ConnectIdentity {
                kind: if prefix == SINK_TASK_PREFIX {
                    TaskKind::Sink
                } else {
                    TaskKind::Source
                },
                connector,
                task,
            };
            let inventory = identities
                .entry(identity)
                .or_default()
                .entry((&worker.worker, worker_index))
                .or_insert_with(|| ActiveInventory {
                    worker,
                    series: [Series::Missing; SOURCE_SUFFIXES.len()],
                    lag: Series::Missing,
                });
            if index < SOURCE_SUFFIXES.len() {
                inventory.series[index].observe(sample);
            } else {
                inventory.lag.observe(sample);
            }
        }
    }
    identities
        .into_iter()
        .flat_map(|(identity, workers)| {
            let ambiguous = workers.len() > 1;
            let ConnectIdentity {
                kind,
                connector,
                task,
            } = identity;
            workers.into_values().map(move |inventory| {
                let ActiveInventory {
                    worker,
                    series,
                    lag,
                } = inventory;
                let lookup = |series: Series<'_>| {
                    if ambiguous {
                        absent(MetricState::Unavailable)
                    } else {
                        series.lookup(worker, false, 2)
                    }
                };
                ConnectBackpressure {
                    connector: connector.to_owned(),
                    task: task.to_owned(),
                    worker: worker.worker.clone(),
                    kind,
                    active_records: lookup(series[0]),
                    active_records_max: lookup(series[1]),
                    active_records_avg: lookup(series[2]),
                    consumer_lag_max: lookup(lag),
                }
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "backpressure_tests.rs"]
mod tests;
