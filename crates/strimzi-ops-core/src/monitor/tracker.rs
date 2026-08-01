use serde_json::Value;
use std::collections::HashMap;

/// In-memory snapshot progress for one connector.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotState {
    pub status: String,
    pub progress: u64,
    pub start_time: Option<Value>,
    pub end_time: Option<Value>,
    pub notification: Value,
}

/// Tracks snapshot lifecycle from Debezium notification messages.
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
        let Some(connector) = notification
            .get("aggregateType")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };

        match notification_type {
            "STARTED" => self.handle_started(&connector, notification),
            "IN_PROGRESS" => self.handle_progress(&connector, notification),
            "COMPLETED" => self.handle_completed(&connector, notification),
            "ABORTED" => self.handle_aborted(&connector, notification),
            _ => {}
        }
    }

    pub fn get_snapshot_status(&self, connector: &str) -> Option<&SnapshotState> {
        self.snapshots.get(connector)
    }

    pub fn get_all_snapshots(&self) -> &HashMap<String, SnapshotState> {
        &self.snapshots
    }

    fn handle_started(&mut self, connector: &str, notification: &Value) {
        self.snapshots.insert(
            connector.to_owned(),
            SnapshotState {
                status: "STARTED".to_owned(),
                progress: 0,
                start_time: notification.get("timestamp").cloned(),
                end_time: None,
                notification: notification.clone(),
            },
        );
    }

    fn handle_progress(&mut self, connector: &str, notification: &Value) {
        let Some(state) = self.snapshots.get_mut(connector) else {
            return;
        };
        let progress = notification
            .pointer("/data/progress")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        "IN_PROGRESS".clone_into(&mut state.status);
        state.progress = progress;
        state.notification = notification.clone();
    }

    fn handle_completed(&mut self, connector: &str, notification: &Value) {
        let Some(state) = self.snapshots.get_mut(connector) else {
            return;
        };
        "COMPLETED".clone_into(&mut state.status);
        state.progress = 100;
        state.end_time = notification.get("timestamp").cloned();
        state.notification = notification.clone();
    }

    fn handle_aborted(&mut self, connector: &str, notification: &Value) {
        let Some(state) = self.snapshots.get_mut(connector) else {
            return;
        };
        "ABORTED".clone_into(&mut state.status);
        state.end_time = notification.get("timestamp").cloned();
        state.notification = notification.clone();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn tracks_snapshot_lifecycle() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&json!({
            "type": "STARTED",
            "aggregateType": "demo",
            "timestamp": 1
        }));
        tracker.process_notification(&json!({
            "type": "IN_PROGRESS",
            "aggregateType": "demo",
            "data": { "progress": 40 }
        }));
        tracker.process_notification(&json!({
            "type": "COMPLETED",
            "aggregateType": "demo",
            "timestamp": 2
        }));

        let state = tracker.get_snapshot_status("demo").expect("state");
        assert_eq!(state.status, "COMPLETED");
        assert_eq!(state.progress, 100);
        assert_eq!(state.start_time, Some(json!(1)));
        assert_eq!(state.end_time, Some(json!(2)));
    }

    #[test]
    fn ignores_notifications_without_connector() {
        let mut tracker = SnapshotTracker::new();
        tracker.process_notification(&json!({ "type": "STARTED" }));
        assert!(tracker.get_all_snapshots().is_empty());
    }
}
