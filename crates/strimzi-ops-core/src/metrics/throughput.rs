use std::collections::BTreeMap;
use std::fmt;

use super::{MetricSample, MetricState, MetricsSnapshot, WorkerMetrics};

const SOURCE_PREFIX: &str = "kafka_connect_source_task_";
const SINK_PREFIX: &str = "kafka_connect_sink_task_";
const CONNECTOR_STATUS: &str = "kafka_connect_connector_status";
const TASK_STATUS: &str = "kafka_connect_connector_task_status";
const MEASUREMENTS: [&str; 6] = [
    "kafka_connect_source_task_source_record_write_rate",
    "kafka_connect_source_task_source_record_write_total",
    "kafka_connect_sink_task_sink_record_read_rate",
    "kafka_connect_sink_task_sink_record_read_total",
    "kafka_connect_sink_task_sink_record_send_rate",
    "kafka_connect_sink_task_sink_record_send_total",
];
const STAGES: [StageMetrics; 3] = [
    StageMetrics {
        label: "Source records written",
        rate: 0,
        total: 1,
    },
    StageMetrics {
        label: "Sink records read",
        rate: 2,
        total: 3,
    },
    StageMetrics {
        label: "Sink records sent",
        rate: 4,
        total: 5,
    },
];

/// Whether a throughput measurement is safe to present as current.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThroughputState {
    Available,
    /// The required task metric is absent, invalid, or has unsupported dimensions.
    Unsupported,
    Stale,
    Unreachable,
    /// Multiple series or workers claim the same measurement or task identity.
    Ambiguous,
}

impl fmt::Display for ThroughputState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Available => "available",
            Self::Unsupported => "unsupported",
            Self::Stale => "stale",
            Self::Unreachable => "unreachable",
            Self::Ambiguous => "ambiguous",
        })
    }
}

/// A real measurement, not a synthetic zero. Only available measurements and
/// explicitly stale, retained task measurements can contain a value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThroughputValue {
    pub state: ThroughputState,
    pub value: Option<f64>,
}

impl ThroughputValue {
    /// Expire a cached available value without discarding its real reading.
    pub fn expire(&mut self) {
        if self.state == ThroughputState::Available {
            self.state = ThroughputState::Stale;
        }
    }

    fn absent(state: ThroughputState) -> Self {
        Self { state, value: None }
    }
}

/// A native rolling records/second gauge and a current task-lifetime counter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThroughputStage {
    pub label: &'static str,
    pub rate: ThroughputValue,
    pub total: ThroughputValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskThroughput {
    pub task: String,
    pub worker: String,
    pub stages: Vec<ThroughputStage>,
}

/// Connector stages sum all known tasks, never worker or connector metrics.
/// Totals describe the current tasks' lifetimes, not connector lifetime history.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorThroughput {
    pub connector: String,
    pub stages: Vec<ThroughputStage>,
    pub tasks: Vec<TaskThroughput>,
}

struct StageMetrics {
    label: &'static str,
    rate: usize,
    total: usize,
}

#[derive(Clone, Copy, Default)]
enum Series<'a> {
    #[default]
    Missing,
    One(&'a MetricSample),
    Duplicate,
}

impl<'a> Series<'a> {
    fn observe(&mut self, sample: &'a MetricSample) {
        *self = match self {
            Self::Missing => Self::One(sample),
            Self::One(_) | Self::Duplicate => Self::Duplicate,
        };
    }

    fn value(self, worker: &WorkerMetrics) -> ThroughputValue {
        if matches!(self, Self::Duplicate) {
            return ThroughputValue::absent(ThroughputState::Ambiguous);
        }
        if worker.state == MetricState::CollectionError {
            return ThroughputValue::absent(ThroughputState::Unreachable);
        }
        match self {
            Self::One(sample)
                if sample.labels.len() == 2
                    && sample.value.is_finite()
                    && sample.value >= 0.0
                    && matches!(worker.state, MetricState::Available | MetricState::Stale) =>
            {
                ThroughputValue {
                    state: worker_state(worker.state),
                    value: Some(sample.value),
                }
            }
            _ => ThroughputValue::absent(ThroughputState::Unsupported),
        }
    }
}

struct TaskInventory<'a> {
    worker: &'a WorkerMetrics,
    measurements: [Series<'a>; 6],
}

#[derive(Default)]
struct ConnectorInventory<'a> {
    source: bool,
    sink: bool,
    // Include the worker index so even malformed snapshots with repeated worker
    // names cannot silently pick one of their readings.
    tasks: BTreeMap<(&'a str, &'a str, usize), TaskInventory<'a>>,
}

/// Project common Kafka Connect task metrics without consuming Kafka payloads.
///
/// Rates are the exporter's native rolling gauges, never counter deltas. Task
/// restarts therefore cannot introduce subtraction spikes or negative rates.
/// Missing task readings invalidate a sum, and duplicate connector/task claims
/// across workers invalidate all measurements of that task. Multiple series for
/// a measurement are ambiguous even when distinguished by extra labels; a lone
/// extra-dimension series is unsupported rather than being silently collapsed.
///
/// Any stale or unreachable worker may conceal additional tasks, so connector
/// aggregates have no value until membership is trustworthy again. Healthy task
/// readings remain usable; stale real task readings retain their explicit state.
/// Connector and task status metrics seed identities with missing throughput.
/// Source/sink prefixes select the relevant stages; unknown types expose all
/// three unsupported stages. Output is ordered by connector, task, and worker.
pub fn connector_throughput(snapshot: &MetricsSnapshot) -> Vec<ConnectorThroughput> {
    let mut connectors: BTreeMap<&str, ConnectorInventory<'_>> = BTreeMap::new();
    let mut membership = ThroughputState::Available;
    for (worker_index, worker) in snapshot.workers.iter().enumerate() {
        membership = most_severe(membership, worker_state(worker.state));
        for sample in worker.samples.iter() {
            let source = sample.name.starts_with(SOURCE_PREFIX);
            let sink = sample.name.starts_with(SINK_PREFIX);
            if !source && !sink && sample.name != CONNECTOR_STATUS && sample.name != TASK_STATUS {
                continue;
            }
            let Some(connector) = sample
                .labels
                .get("connector")
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let inventory = connectors.entry(connector).or_default();
            inventory.source |= source;
            inventory.sink |= sink;
            if sample.name == CONNECTOR_STATUS {
                continue;
            }
            let Some(task) = sample.labels.get("task").filter(|task| !task.is_empty()) else {
                continue;
            };
            let task = inventory
                .tasks
                .entry((task, &worker.worker, worker_index))
                .or_insert_with(|| TaskInventory {
                    worker,
                    measurements: [Series::Missing; 6],
                });
            if let Some(index) = MEASUREMENTS.iter().position(|name| *name == sample.name) {
                task.measurements[index].observe(sample);
            }
        }
    }
    connectors
        .into_iter()
        .map(|(connector, inventory)| project_connector(connector, inventory, membership))
        .collect()
}

fn project_connector(
    connector: &str,
    inventory: ConnectorInventory<'_>,
    membership: ThroughputState,
) -> ConnectorThroughput {
    let stages = match (inventory.source, inventory.sink) {
        (true, false) => &STAGES[..1],
        (false, true) => &STAGES[1..],
        _ => &STAGES[..],
    };
    let mut tasks = Vec::with_capacity(inventory.tasks.len());
    let mut entries = inventory.tasks.into_iter().peekable();
    let mut previous_task = None;
    while let Some(((task, worker, _), readings)) = entries.next() {
        let duplicate_task =
            previous_task == Some(task) || entries.peek().is_some_and(|(key, _)| key.0 == task);
        previous_task = Some(task);
        tasks.push(TaskThroughput {
            task: task.to_owned(),
            worker: worker.to_owned(),
            stages: stages
                .iter()
                .map(|stage| {
                    let reading = |index: usize| {
                        if duplicate_task {
                            ThroughputValue::absent(ThroughputState::Ambiguous)
                        } else {
                            readings.measurements[index].value(readings.worker)
                        }
                    };
                    ThroughputStage {
                        label: stage.label,
                        rate: reading(stage.rate),
                        total: reading(stage.total),
                    }
                })
                .collect(),
        });
    }
    let stages = stages
        .iter()
        .enumerate()
        .map(|(index, stage)| ThroughputStage {
            label: stage.label,
            rate: sum_values(tasks.iter().map(|task| task.stages[index].rate), membership),
            total: sum_values(
                tasks.iter().map(|task| task.stages[index].total),
                membership,
            ),
        })
        .collect();
    ConnectorThroughput {
        connector: connector.to_owned(),
        stages,
        tasks,
    }
}

fn sum_values(
    values: impl Iterator<Item = ThroughputValue>,
    membership: ThroughputState,
) -> ThroughputValue {
    let mut state = membership;
    let mut sum = (membership == ThroughputState::Available).then_some(0.0);
    let mut has_tasks = false;
    for reading in values {
        has_tasks = true;
        state = most_severe(state, reading.state);
        sum = match (sum, reading.value, reading.state) {
            (Some(sum), Some(value), ThroughputState::Available) => {
                let sum = sum + value;
                if sum.is_finite() {
                    Some(sum)
                } else {
                    state = most_severe(state, ThroughputState::Unsupported);
                    None
                }
            }
            _ => None,
        };
    }
    if !has_tasks {
        state = most_severe(state, ThroughputState::Unsupported);
        sum = None;
    }
    ThroughputValue {
        state,
        value: if state == ThroughputState::Available {
            sum
        } else {
            None
        },
    }
}

fn worker_state(state: MetricState) -> ThroughputState {
    match state {
        MetricState::Available => ThroughputState::Available,
        MetricState::Unavailable => ThroughputState::Unsupported,
        MetricState::Stale => ThroughputState::Stale,
        MetricState::CollectionError => ThroughputState::Unreachable,
    }
}

fn most_severe(first: ThroughputState, second: ThroughputState) -> ThroughputState {
    let priority = |state| match state {
        ThroughputState::Available => 0,
        ThroughputState::Unsupported => 1,
        ThroughputState::Stale => 2,
        ThroughputState::Unreachable => 3,
        ThroughputState::Ambiguous => 4,
    };
    if priority(first) >= priority(second) {
        first
    } else {
        second
    }
}

#[cfg(test)]
#[path = "throughput_tests.rs"]
mod tests;
