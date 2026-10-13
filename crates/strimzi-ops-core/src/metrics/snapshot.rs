use std::collections::BTreeMap;

use super::{MetricLookup, MetricSample, MetricState, MetricsSnapshot, WorkerMetrics};

const PREFIX: &str = "debezium_";
// Exact suffixes keep domains containing underscores intact. The final three
// attributes seed identity only, without supplying snapshot measurements.
const SUFFIXES: [&str; 13] = [
    "_snapshot_running",
    "_snapshot_completed",
    "_snapshot_aborted",
    "_snapshot_paused",
    "_snapshot_skipped",
    "_snapshot_duration_in_seconds",
    "_total_table_count",
    "_remaining_table_count",
    "_total_number_of_events_seen_total",
    "_rows_scanned",
    "_connected",
    "_queue_total_capacity",
    "_queue_remaining_capacity",
];
const MEASUREMENT_COUNT: usize = 9;
const ROWS_INDEX: usize = MEASUREMENT_COUNT;
const BOOLEAN_COUNT: usize = 5;

/// A table's historical reported `RowsScanned` map entry, not a current-run delta.
///
/// Debezium 3.7's native `SnapshotMeter` retains this map on `snapshotStarted` and
/// replaces individual entries as reported. Never sum entries or infer progress
/// percentages or ownership by a notification's run ID from these values.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotTableRows {
    pub table: String,
    pub rows: MetricLookup,
}

/// Independent readings for an explicitly tagged Debezium snapshot meter.
///
/// Context is the literal exporter context, not an inferred snapshot kind. The
/// default Debezium 3.7 coordinator shares its context="snapshot" meter with the
/// incremental source, so that context alone does not prove an initial snapshot.
/// A literal "incremental-snapshot" context is preserved if exported separately.
/// Native metrics do not establish identity with any notification's run ID.
/// Duration and table counts describe the exporter's retained lifecycle values.
/// Events are historical cumulative totals; table rows are historical map values.
/// Neither is a per-new-snapshot count, delta, or percentage.
#[derive(Debug, Clone, PartialEq)]
pub struct DebeziumSnapshot {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub domain: String,
    pub server: String,
    pub context: String,
    pub running: MetricLookup,
    pub completed: MetricLookup,
    pub aborted: MetricLookup,
    pub paused: MetricLookup,
    pub skipped: MetricLookup,
    pub duration_secs: MetricLookup,
    pub total_tables: MetricLookup,
    pub remaining_tables: MetricLookup,
    pub events_total: MetricLookup,
    pub rows: Vec<SnapshotTableRows>,
    notification_scope_unique: bool,
}

impl DebeziumSnapshot {
    /// Interpret only a complete, fresh, unambiguous set of boolean flags.
    ///
    /// Native paused means running=false. Skipped is additive in the native
    /// meter; coexistence with another true flag is conservatively ambiguous,
    /// not a claim that the exporter or snapshot kind is unsupported.
    pub fn lifecycle(&self) -> Option<&'static str> {
        let flags = [
            self.running,
            self.completed,
            self.aborted,
            self.paused,
            self.skipped,
        ];
        if flags.iter().any(|flag| {
            flag.state != MetricState::Available || !matches!(flag.value, Some(0.0 | 1.0))
        }) {
            return None;
        }
        let mut active = None;
        for (flag, status) in
            flags
                .into_iter()
                .zip(["RUNNING", "COMPLETED", "ABORTED", "PAUSED", "SKIPPED"])
        {
            if flag.value == Some(1.0) {
                if active.is_some() {
                    return None;
                }
                active = Some(status);
            }
        }
        Some(active.unwrap_or("IDLE"))
    }

    /// Return a notification scope only when the exporter explicitly separates
    /// incremental snapshots. The shared native snapshot meter proves no kind.
    pub fn notification_context(&self) -> Option<&str> {
        (self.context == "incremental-snapshot").then_some(self.context.as_str())
    }

    /// Compare observations after matching the notification's logical source
    /// (`connector_name`) to the exported server, not to the Connect REST name.
    ///
    /// Agreement does not establish that exporter and notification refer to the
    /// same run. Missing or stale metrics never replace notification evidence.
    pub fn comparison(&self, notification_status: Option<&str>) -> &'static str {
        if self.context == "snapshot" {
            return "exporter shares initial/incremental context; run association unavailable";
        }
        if !self.notification_scope_unique {
            return "notification source association ambiguous or unavailable; events retained";
        }
        let Some(notification_status) = notification_status else {
            return "no notification observed";
        };
        let Some(lifecycle) = self.lifecycle() else {
            return "metrics unavailable or stale; notification retained";
        };
        let expected = match notification_status {
            "STARTED" | "IN_PROGRESS" | "RESUMED" => "RUNNING",
            status => status,
        };
        if lifecycle == expected {
            "consistent observations; same run not established"
        } else {
            "different observations; same run not established"
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Identity<'a> {
    connector: &'a str,
    task: &'a str,
    domain: &'a str,
    server: &'a str,
    context: &'a str,
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

    fn lookup(self, worker: &WorkerMetrics, boolean: bool, label_count: usize) -> MetricLookup {
        if matches!(self, Self::Duplicate) {
            return absent(MetricState::Unavailable);
        }
        if worker.state == MetricState::CollectionError {
            return absent(MetricState::CollectionError);
        }
        match self {
            Self::One(sample)
                if sample.labels.len() == label_count
                    && sample.value.is_finite()
                    && sample.value >= 0.0
                    && (!boolean || matches!(sample.value, 0.0 | 1.0))
                    && matches!(worker.state, MetricState::Available | MetricState::Stale) =>
            {
                MetricLookup {
                    state: worker.state,
                    value: Some(sample.value),
                }
            }
            _ => absent(MetricState::Unavailable),
        }
    }
}

struct WorkerInventory<'a> {
    worker: &'a WorkerMetrics,
    measurements: [Series<'a>; MEASUREMENT_COUNT],
    rows: BTreeMap<&'a str, Series<'a>>,
}

/// Project exact exported snapshot series, without aggregation or guessed IDs.
///
/// All four nonempty connector/task/server/context labels are required; rows
/// additionally require a nonempty table label. Only "snapshot" and
/// "incremental-snapshot" supply readings. Other non-streaming contexts retain
/// their explicit identities with unavailable values; streaming is excluded.
/// Known event/connected/queue suffixes can seed an unavailable row only when
/// explicit non-streaming context is present; streaming alone cannot invent one.
/// Extra dimensions, duplicate series, ambiguous worker ownership, nonfinite or
/// negative values, and nonboolean flags are unavailable, never synthetic zero.
/// Stale valid readings retain stale values; unavailable/error readings have no
/// numeric values. Output is ordered by connector/task/domain/server/context/
/// worker, with rows ordered by table. Context and table identity are never lost.
pub fn debezium_snapshots(snapshot: &MetricsSnapshot) -> Vec<DebeziumSnapshot> {
    let mut identities: BTreeMap<Identity<'_>, BTreeMap<(&str, usize), WorkerInventory<'_>>> =
        BTreeMap::new();
    for (worker_index, worker) in snapshot.workers.iter().enumerate() {
        for sample in worker.samples.iter() {
            let Some((index, identity)) = sample_identity(sample) else {
                continue;
            };
            // Keep index as well as worker name: repeated names cannot win.
            let inventory = identities
                .entry(identity)
                .or_default()
                .entry((&worker.worker, worker_index))
                .or_insert_with(|| WorkerInventory {
                    worker,
                    measurements: [Series::Missing; MEASUREMENT_COUNT],
                    rows: BTreeMap::new(),
                });
            if index < MEASUREMENT_COUNT {
                inventory.measurements[index].observe(sample);
            } else if index == ROWS_INDEX {
                if let Some(table) = label(sample, "table") {
                    inventory.rows.entry(table).or_default().observe(sample);
                }
            }
        }
    }
    let mut notification_scopes = BTreeMap::new();
    for (identity, workers) in &identities {
        *notification_scopes
            .entry((identity.server, identity.context))
            .or_insert(0usize) += workers.len();
    }
    identities
        .into_iter()
        .flat_map(|(identity, workers)| {
            let usable_context = matches!(identity.context, "snapshot" | "incremental-snapshot");
            let ambiguous = workers.len() > 1;
            let notification_scope_unique =
                notification_scopes[&(identity.server, identity.context)] == 1;
            workers.into_values().map(move |inventory| {
                let lookup = |series: Series<'_>, boolean, label_count| {
                    if ambiguous || !usable_context {
                        absent(MetricState::Unavailable)
                    } else {
                        series.lookup(inventory.worker, boolean, label_count)
                    }
                };
                let measurement =
                    |index| lookup(inventory.measurements[index], index < BOOLEAN_COUNT, 4);
                DebeziumSnapshot {
                    connector: identity.connector.to_owned(),
                    task: identity.task.to_owned(),
                    worker: inventory.worker.worker.clone(),
                    domain: identity.domain.to_owned(),
                    server: identity.server.to_owned(),
                    context: identity.context.to_owned(),
                    notification_scope_unique,
                    running: measurement(0),
                    completed: measurement(1),
                    aborted: measurement(2),
                    paused: measurement(3),
                    skipped: measurement(4),
                    duration_secs: measurement(5),
                    total_tables: measurement(6),
                    remaining_tables: measurement(7),
                    events_total: measurement(8),
                    rows: inventory
                        .rows
                        .into_iter()
                        .map(|(table, series)| SnapshotTableRows {
                            table: table.to_owned(),
                            rows: lookup(series, false, 5),
                        })
                        .collect(),
                }
            })
        })
        .collect()
}

fn label<'a>(sample: &'a MetricSample, name: &str) -> Option<&'a str> {
    sample
        .labels
        .get(name)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

fn sample_identity(sample: &MetricSample) -> Option<(usize, Identity<'_>)> {
    let name = sample.name.strip_prefix(PREFIX)?;
    let (index, domain) = SUFFIXES.iter().enumerate().find_map(|(index, suffix)| {
        name.strip_suffix(*suffix)
            .filter(|domain| !domain.is_empty())
            .map(|domain| (index, domain))
    })?;
    let context = label(sample, "context")?;
    if context == "streaming" {
        return None;
    }
    Some((
        index,
        Identity {
            connector: label(sample, "connector")?,
            task: label(sample, "task")?,
            domain,
            server: label(sample, "server")?,
            context,
        },
    ))
}

fn absent(state: MetricState) -> MetricLookup {
    MetricLookup { state, value: None }
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
