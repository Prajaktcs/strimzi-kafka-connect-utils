use std::fmt::Write;
use std::sync::Arc;
use std::time::Instant;

use serde_json::json;
use strimzi_ops_core::metrics::{parse_prometheus, MetricsSnapshot, WorkerMetrics};

use super::*;

#[test]
fn expires_cached_flags_before_comparing_notification_state() {
    let labels = "connector=\"source\",task=\"0\",server=\"db\",context=\"incremental-snapshot\"";
    let mut text = String::new();
    for (name, value) in [
        ("snapshot_running", 1),
        ("snapshot_completed", 0),
        ("snapshot_aborted", 0),
        ("snapshot_paused", 0),
        ("snapshot_skipped", 0),
    ] {
        writeln!(text, "debezium_postgres_{name}{{{labels}}} {value}").unwrap();
    }
    let mut cached = CachedMetrics {
        snapshot: Arc::new(MetricsSnapshot {
            state: MetricState::Available,
            workers: vec![WorkerMetrics {
                worker: "worker".to_owned(),
                state: MetricState::Available,
                samples: Arc::new(parse_prometheus(&text).unwrap()),
                age_secs: Some(0),
                error: None,
            }],
        }),
        received: Instant::now(),
        previous: None,
    };
    let mut tracker = SnapshotTracker::new();
    tracker.process_notification(&json!({
        "type": "STARTED",
        "aggregate_type": "Incremental Snapshot",
        "additional_data": { "connector_name": "db" },
        "timestamp": "1"
    }));
    let fresh = snapshot_metrics(&cached, Some(&tracker));
    assert_eq!(fresh.state, MetricState::Available);
    assert_eq!(
        fresh.rows[0].comparison,
        "consistent observations; same run not established"
    );
    cached.received -= STALE_AFTER;
    let expired = snapshot_metrics(&cached, Some(&tracker));
    assert_eq!(expired.state, MetricState::Stale);
    assert!(expired.rows[0].running.contains("stale"));
    assert_eq!(
        expired.rows[0].comparison,
        "metrics unavailable or stale; notification retained"
    );
    assert_eq!(
        tracker
            .get_snapshot_status("db", "incremental-snapshot")
            .unwrap()
            .status,
        "STARTED"
    );
}
