use askama::Template;
use strimzi_ops_core::metrics::{debezium_snapshots, MetricLookup, MetricState, STALE_AFTER};
use strimzi_ops_core::SnapshotTracker;

use super::{expire_lookup, format_streaming_measurement};
use crate::metrics::CachedMetrics;

const DISPLAY_LIMIT: usize = 200;

pub struct SnapshotMetricView {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub domain: String,
    pub server: String,
    pub context: String,
    pub age: String,
    pub running: String,
    pub completed: String,
    pub aborted: String,
    pub paused: String,
    pub skipped: String,
    pub duration: String,
    pub total_tables: String,
    pub remaining_tables: String,
    pub events: String,
    pub comparison: &'static str,
    pub tables: Vec<SnapshotTableView>,
}

pub struct SnapshotTableView {
    pub table: String,
    pub rows: String,
}

pub struct SnapshotMetricsPanel {
    pub state: MetricState,
    pub rows: Vec<SnapshotMetricView>,
    pub omitted: usize,
    pub tables_omitted: usize,
}

/// Render independent exporter observations, never attach retained counters to a
/// notification's run ID. Both sources lack enough information to prove that link.
pub(crate) fn snapshot_metrics(
    cached: &CachedMetrics,
    tracker: Option<&SnapshotTracker>,
) -> SnapshotMetricsPanel {
    let elapsed = cached.received.elapsed();
    let expired = elapsed >= STALE_AFTER;
    let projections = debezium_snapshots(&cached.snapshot);
    let omitted = projections.len().saturating_sub(DISPLAY_LIMIT);
    let mut tables_remaining = DISPLAY_LIMIT;
    let mut tables_omitted = 0;
    let rows = projections
        .into_iter()
        .take(DISPLAY_LIMIT)
        .map(|mut row| {
            if expired {
                for reading in [
                    &mut row.running,
                    &mut row.completed,
                    &mut row.aborted,
                    &mut row.paused,
                    &mut row.skipped,
                ] {
                    *reading = expire_lookup(*reading, true);
                }
            }
            let notification = row.notification_context().and_then(|context| {
                tracker.and_then(|tracker| tracker.get_snapshot_status(&row.server, context))
            });
            let comparison = row.comparison(notification.map(|state| state.status.as_str()));
            let age = cached
                .snapshot
                .workers
                .iter()
                .find(|worker| worker.worker == row.worker)
                .and_then(|worker| worker.age_secs)
                .map_or_else(
                    || "never".to_owned(),
                    |age| format!("{} s", age.saturating_add(elapsed.as_secs())),
                );
            tables_omitted += row.rows.len().saturating_sub(tables_remaining);
            let tables: Vec<_> = row
                .rows
                .into_iter()
                .take(tables_remaining)
                .map(|table| SnapshotTableView {
                    table: table.table,
                    rows: format_streaming_measurement(table.rows, expired, 0),
                })
                .collect();
            tables_remaining -= tables.len();
            SnapshotMetricView {
                connector: row.connector,
                task: row.task,
                worker: row.worker,
                domain: row.domain,
                server: row.server,
                context: row.context,
                age,
                running: boolean(row.running, expired),
                completed: boolean(row.completed, expired),
                aborted: boolean(row.aborted, expired),
                paused: boolean(row.paused, expired),
                skipped: boolean(row.skipped, expired),
                duration: format_streaming_measurement(row.duration_secs, expired, 0),
                total_tables: format_streaming_measurement(row.total_tables, expired, 0),
                remaining_tables: format_streaming_measurement(row.remaining_tables, expired, 0),
                events: format_streaming_measurement(row.events_total, expired, 0),
                comparison,
                tables,
            }
        })
        .collect();
    SnapshotMetricsPanel {
        state: if expired && cached.snapshot.state == MetricState::Available {
            MetricState::Stale
        } else {
            cached.snapshot.state
        },
        rows,
        omitted,
        tables_omitted,
    }
}

fn boolean(reading: MetricLookup, expired: bool) -> String {
    let reading = expire_lookup(reading, expired);
    match (reading.value, reading.state) {
        (Some(value), state @ (MetricState::Available | MetricState::Stale)) => {
            let value = match value {
                1.0 => "yes",
                0.0 => "no",
                _ => return MetricState::Unavailable.to_string(),
            };
            if state == MetricState::Stale {
                format!("last observed: {value} (stale)")
            } else {
                value.to_owned()
            }
        }
        (_, state) => state.to_string(),
    }
}

pub struct SnapshotNotificationView<'a> {
    pub connector: &'a str,
    pub context: &'a str,
    pub status: &'a str,
    pub id: &'a str,
    pub timestamp: Option<&'a serde_json::Value>,
    pub events: &'a [serde_json::Value],
}

#[derive(Template)]
#[template(path = "snapshot_cards.html")]
pub struct SnapshotCards<'a> {
    pub snapshots: Vec<SnapshotNotificationView<'a>>,
    pub snapshot_metrics: SnapshotMetricsPanel,
    pub notification_error: Option<&'a str>,
}

impl<'a> SnapshotCards<'a> {
    pub(crate) fn new(
        tracker: &'a SnapshotTracker,
        cached: &CachedMetrics,
        notification_error: Option<&'a str>,
    ) -> Self {
        let mut snapshots: Vec<_> = tracker
            .get_all_snapshots()
            .values()
            .map(|state| SnapshotNotificationView {
                connector: &state.connector,
                context: &state.context,
                status: &state.status,
                id: state.id.as_deref().unwrap_or("not reported"),
                timestamp: state.observed_time.as_ref(),
                events: &state.events,
            })
            .collect();
        snapshots.sort_unstable_by(|a, b| (a.connector, a.context).cmp(&(b.connector, b.context)));
        Self {
            snapshots,
            snapshot_metrics: snapshot_metrics(cached, Some(tracker)),
            notification_error,
        }
    }
}

#[cfg(test)]
#[path = "snapshots_tests.rs"]
mod tests;
