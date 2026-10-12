use serde_json::Value;
use std::collections::HashMap;

const MAX_EVENTS: usize = 20;

/// Last observed snapshot lifecycle for one logical source and notification context.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotState {
    /// Native `additional_data.connector_name`: the logical source name
    /// (`topic.prefix` for `PostgreSQL`), not necessarily the Connect REST name.
    pub connector: String,
    pub context: String,
    /// Last notification ID, not necessarily a snapshot-run identifier.
    pub id: Option<String>,
    /// Lifecycle status; UNKNOWN when only a table event has been observed.
    pub status: String,
    /// Percentage only when explicitly supplied in `additional_data.progress`.
    pub progress: Option<u64>,
    pub start_time: Option<Value>,
    pub end_time: Option<Value>,
    /// Last available notification timestamp, in its original wire representation.
    pub observed_time: Option<Value>,
    pub notification: Value,
    /// Recent accepted wire payloads, including lifecycle and table events.
    pub events: Vec<Value>,
}

/// Tracks snapshot lifecycle from native Debezium notification messages.
#[derive(Debug, Default)]
pub struct SnapshotTracker {
    snapshots: HashMap<String, SnapshotState>,
}

impl SnapshotTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process_notification(&mut self, notification: &Value) {
        let Some(notification_type) = notification.get("type").and_then(Value::as_str) else {
            return;
        };
        if !matches!(
            notification_type,
            "STARTED"
                | "IN_PROGRESS"
                | "COMPLETED"
                | "ABORTED"
                | "PAUSED"
                | "RESUMED"
                | "SKIPPED"
                | "TABLE_SCAN_COMPLETED"
                | "DATA_COLLECTIONS_RESOLVED"
                | "TABLE_CHUNK_IN_PROGRESS"
                | "TABLE_CHUNK_COMPLETED"
        ) {
            return;
        }
        let Some(connector) = notification
            .pointer("/additional_data/connector_name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty())
        else {
            return;
        };
        let context = match notification.get("aggregate_type").and_then(Value::as_str) {
            Some("Initial Snapshot") => "initial-snapshot",
            Some("Incremental Snapshot") => "incremental-snapshot",
            _ => return,
        };

        let key = format!("{connector} ({context})");
        let state = self.snapshots.entry(key).or_insert_with(|| SnapshotState {
            connector: connector.to_owned(),
            context: context.to_owned(),
            id: None,
            status: "UNKNOWN".to_owned(),
            progress: None,
            start_time: None,
            end_time: None,
            observed_time: None,
            notification: Value::Null,
            events: Vec::new(),
        });

        if let (Some(incoming), Some(observed)) = (
            notification.get("timestamp").and_then(unsigned_integer),
            state.observed_time.as_ref().and_then(unsigned_integer),
        ) {
            if incoming < observed {
                return;
            }
        }
        // IDs identify individual notifications for initial snapshots. Do not use
        // an ID mismatch as evidence of a different run; deduplicate raw replays.
        if state.events.iter().any(|event| event == notification) {
            return;
        }

        if notification_type == "STARTED" {
            state.start_time = notification.get("timestamp").cloned();
            state.end_time = None;
            state.progress = None;
            state.observed_time = None;
        }
        if !matches!(
            notification_type,
            "TABLE_SCAN_COMPLETED"
                | "DATA_COLLECTIONS_RESOLVED"
                | "TABLE_CHUNK_IN_PROGRESS"
                | "TABLE_CHUNK_COMPLETED"
        ) {
            notification_type.clone_into(&mut state.status);
        }
        if matches!(notification_type, "COMPLETED" | "ABORTED" | "SKIPPED") {
            state.end_time = notification.get("timestamp").cloned();
        }
        if let Some(progress) = notification
            .pointer("/additional_data/progress")
            .and_then(unsigned_integer)
            .filter(|progress| *progress <= 100)
        {
            state.progress = Some(progress);
        }
        state.id = notification
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(timestamp) = notification.get("timestamp") {
            state.observed_time = Some(timestamp.clone());
        }
        state.notification = notification.clone();
        if state.events.len() == MAX_EVENTS {
            state.events.remove(0);
        }
        state.events.push(notification.clone());
    }

    pub fn get_snapshot_status(&self, connector: &str, context: &str) -> Option<&SnapshotState> {
        self.snapshots
            .values()
            .find(|state| state.connector == connector && state.context == context)
    }

    /// Keys are readable labels; use each state's connector/context for identity.
    pub fn get_all_snapshots(&self) -> &HashMap<String, SnapshotState> {
        &self.snapshots
    }
}

fn unsigned_integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|number| number.parse().ok()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn notification(aggregate: &str, kind: &str, id: &str, timestamp: u64) -> Value {
        json!({
            "id": id,
            "aggregate_type": aggregate,
            "type": kind,
            "additional_data": { "connector_name": "demo" },
            "timestamp": timestamp.to_string()
        })
    }

    #[test]
    fn tracks_native_snapshot_lifecycle_without_fabricated_percentage() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&notification("Initial Snapshot", "STARTED", "start", 1));
        tracker.process_notification(&notification(
            "Initial Snapshot",
            "IN_PROGRESS",
            "progress",
            2,
        ));
        let completed = notification("Initial Snapshot", "COMPLETED", "complete", 3);
        tracker.process_notification(&completed);

        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.connector, "demo");
        assert_eq!(state.context, "initial-snapshot");
        assert_eq!(state.id.as_deref(), Some("complete"));
        assert_eq!(state.status, "COMPLETED");
        assert_eq!(state.progress, None);
        assert_eq!(state.start_time, Some(json!("1")));
        assert_eq!(state.end_time, Some(json!("3")));
        assert_eq!(state.observed_time, Some(json!("3")));
        assert_eq!(state.notification, completed);
        assert_eq!(state.events.len(), 3);
    }

    #[test]
    fn tracks_initial_and_incremental_contexts_independently() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&notification("Initial Snapshot", "COMPLETED", "initial", 1));
        tracker.process_notification(&notification("Incremental Snapshot", "PAUSED", "signal", 2));
        assert_eq!(tracker.get_all_snapshots().len(), 2);
        assert_eq!(
            tracker
                .get_snapshot_status("demo", "initial-snapshot")
                .expect("initial")
                .status,
            "COMPLETED"
        );
        assert_eq!(
            tracker
                .get_snapshot_status("demo", "incremental-snapshot")
                .expect("incremental")
                .status,
            "PAUSED"
        );
    }

    #[test]
    fn records_terminal_notifications_without_started() {
        for kind in ["COMPLETED", "ABORTED", "SKIPPED"] {
            let mut tracker = SnapshotTracker::new();
            let terminal = notification("Initial Snapshot", kind, "terminal", 10);
            tracker.process_notification(&terminal);
            let state = tracker
                .get_snapshot_status("demo", "initial-snapshot")
                .expect("state");
            assert_eq!(state.status, kind);
            assert_eq!(state.start_time, None);
            assert_eq!(state.end_time, Some(json!("10")));
            assert_eq!(state.progress, None);
            assert_eq!(state.notification, terminal);
        }
    }

    #[test]
    fn preserves_lifecycle_status_for_table_scan_events() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&notification("Incremental Snapshot", "PAUSED", "pause", 1));
        let mut table = notification("Incremental Snapshot", "TABLE_SCAN_COMPLETED", "table", 2);
        table["additional_data"]["total_rows_scanned"] = json!("250");
        table["additional_data"]["status"] = json!("SUCCEEDED");
        tracker.process_notification(&table);
        let state = tracker
            .get_snapshot_status("demo", "incremental-snapshot")
            .expect("state");
        assert_eq!(state.status, "PAUSED");
        assert_eq!(state.progress, None);
        assert_eq!(state.notification, table);
        assert_eq!(state.events.len(), 2);
        tracker.process_notification(&notification(
            "Incremental Snapshot",
            "RESUMED",
            "resume",
            3,
        ));
        assert_eq!(
            tracker
                .get_snapshot_status("demo", "incremental-snapshot")
                .expect("state")
                .status,
            "RESUMED"
        );
    }

    #[test]
    fn retains_table_event_without_inventing_a_lifecycle() {
        let mut tracker = SnapshotTracker::new();
        let table = notification("Initial Snapshot", "TABLE_SCAN_COMPLETED", "table", 1);
        tracker.process_notification(&table);
        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.status, "UNKNOWN");
        assert_eq!(state.start_time, None);
        assert_eq!(state.notification, table);
    }

    #[test]
    fn fresh_started_clears_previous_run_and_rejects_stale_events() {
        let mut tracker = SnapshotTracker::new();
        let mut progress = notification("Initial Snapshot", "IN_PROGRESS", "old-progress", 10);
        progress["additional_data"]["progress"] = json!("40");
        tracker.process_notification(&progress);
        tracker.process_notification(&notification(
            "Initial Snapshot",
            "COMPLETED",
            "old-end",
            20,
        ));
        let started = notification("Initial Snapshot", "STARTED", "new-start", 30);
        tracker.process_notification(&started);
        tracker.process_notification(&notification("Initial Snapshot", "ABORTED", "old-end", 25));
        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.status, "STARTED");
        assert_eq!(state.progress, None);
        assert_eq!(state.start_time, Some(json!("30")));
        assert_eq!(state.end_time, None);
        assert_eq!(state.notification, started);
        assert_eq!(state.events.len(), 3);

        let mut newer = notification("Initial Snapshot", "IN_PROGRESS", "new-progress", 31);
        newer["timestamp"] = json!(31);
        newer["additional_data"]["progress"] = json!(55);
        tracker.process_notification(&newer);
        tracker.process_notification(&started);
        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.status, "IN_PROGRESS");
        assert_eq!(state.progress, Some(55));
        assert_eq!(state.observed_time, Some(json!(31)));
    }

    #[test]
    fn deduplicates_identical_notifications_and_bounds_history() {
        let mut tracker = SnapshotTracker::new();
        let started = notification("Initial Snapshot", "STARTED", "start", 1);
        tracker.process_notification(&started);
        tracker.process_notification(&started);
        assert_eq!(
            tracker
                .get_snapshot_status("demo", "initial-snapshot")
                .expect("state")
                .events
                .len(),
            1
        );
        for timestamp in 2..=30 {
            tracker.process_notification(&notification(
                "Initial Snapshot",
                "IN_PROGRESS",
                "progress",
                timestamp,
            ));
        }
        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.events.len(), MAX_EVENTS);
        assert_eq!(state.events[0]["timestamp"], json!("11"));
        assert_eq!(state.events[MAX_EVENTS - 1]["timestamp"], json!("30"));
    }

    #[test]
    fn estimated_rows_and_chunk_counts_are_not_percentage() {
        let mut tracker = SnapshotTracker::new();
        let mut progress = notification("Incremental Snapshot", "IN_PROGRESS", "signal", 1);
        progress["additional_data"]["total_rows"] = json!("100");
        progress["additional_data"]["total_chunks"] = json!("10");
        progress["additional_data"]["chunk_index"] = json!("5");
        tracker.process_notification(&progress);
        let state = tracker
            .get_snapshot_status("demo", "incremental-snapshot")
            .expect("state");
        assert_eq!(state.progress, None);
        assert_eq!(state.notification, progress);
    }

    #[test]
    fn duplicate_started_at_same_timestamp_does_not_reset_progress() {
        let mut tracker = SnapshotTracker::new();
        let started = notification("Initial Snapshot", "STARTED", "start", 1);
        tracker.process_notification(&started);
        let mut progress = notification("Initial Snapshot", "IN_PROGRESS", "progress", 1);
        progress["additional_data"]["progress"] = json!("40");
        tracker.process_notification(&progress);
        tracker.process_notification(&started);
        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.status, "IN_PROGRESS");
        assert_eq!(state.progress, Some(40));
        assert_eq!(state.events.len(), 2);
    }

    #[test]
    fn missing_timestamp_does_not_hide_event_or_clear_last_observed_time() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&notification("Initial Snapshot", "STARTED", "start", 1));
        let mut completed = notification("Initial Snapshot", "COMPLETED", "end", 2);
        completed
            .as_object_mut()
            .expect("object")
            .remove("timestamp");
        tracker.process_notification(&completed);
        let state = tracker
            .get_snapshot_status("demo", "initial-snapshot")
            .expect("state");
        assert_eq!(state.status, "COMPLETED");
        assert_eq!(state.observed_time, Some(json!("1")));
        assert_eq!(state.end_time, None);
        assert_eq!(state.notification, completed);
    }

    #[test]
    fn ignores_missing_identity_and_obsolete_camel_case() {
        let mut tracker = SnapshotTracker::new();
        for invalid in [
            json!({ "type": "STARTED", "aggregateType": "demo", "timestamp": 1 }),
            json!({ "type": "STARTED", "aggregate_type": "Initial Snapshot" }),
            json!({ "type": "STARTED", "additional_data": { "connector_name": "demo" } }),
            json!({ "type": "STARTED", "aggregate_type": "Other", "additional_data": { "connector_name": "demo" } }),
            json!({ "type": "STARTED", "aggregate_type": "Initial Snapshot", "additional_data": { "connector_name": " " } }),
        ] {
            tracker.process_notification(&invalid);
        }
        assert!(tracker.get_all_snapshots().is_empty());
    }

    #[test]
    fn retains_native_collection_and_chunk_events_without_replacing_lifecycle() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&notification("Initial Snapshot", "STARTED", "start", 1));
        for (timestamp, kind) in [
            (2, "DATA_COLLECTIONS_RESOLVED"),
            (3, "TABLE_CHUNK_IN_PROGRESS"),
            (4, "TABLE_CHUNK_COMPLETED"),
        ] {
            let event = notification("Initial Snapshot", kind, kind, timestamp);
            tracker.process_notification(&event);
            let state = tracker
                .get_snapshot_status("demo", "initial-snapshot")
                .unwrap();
            assert_eq!(state.status, "STARTED");
            assert_eq!(state.notification, event);
            assert_eq!(state.events.last(), Some(&event));
            assert_eq!(state.progress, None);
        }
    }
}
