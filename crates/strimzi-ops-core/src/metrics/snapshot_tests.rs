use std::sync::Arc;

use super::*;
use crate::metrics::parse_prometheus;

const SNAPSHOT: &str =
    r#"connector="postgres-source-connector",task="0",server="lakehouse",context="snapshot""#;
const INCREMENTAL: &str = r#"connector="postgres-source-connector",task="0",server="lakehouse",context="incremental-snapshot""#;
const STREAMING: &str =
    r#"connector="postgres-source-connector",task="0",server="lakehouse",context="streaming""#;

fn worker(name: &str, state: MetricState, text: &str) -> WorkerMetrics {
    WorkerMetrics {
        worker: name.to_owned(),
        state,
        samples: Arc::new(parse_prometheus(text).unwrap()),
        age_secs: match state {
            MetricState::Available => Some(0),
            MetricState::Stale => Some(60),
            _ => None,
        },
        error: (state == MetricState::CollectionError).then(|| "cannot fetch metrics".to_owned()),
    }
}

fn project(workers: Vec<WorkerMetrics>) -> Vec<DebeziumSnapshot> {
    debezium_snapshots(&MetricsSnapshot {
        workers,
        state: MetricState::Available,
    })
}

fn sample(domain: &str, suffix: &str, labels: &str, value: &str) -> String {
    format!("debezium_{domain}{suffix}{{{labels}}} {value}\n")
}

fn all_measurements(labels: &str, values: [&str; MEASUREMENT_COUNT]) -> String {
    SUFFIXES[..MEASUREMENT_COUNT]
        .iter()
        .zip(values)
        .map(|(suffix, value)| sample("postgres", suffix, labels, value))
        .collect()
}

fn table_sample(labels: &str, table: &str, value: &str) -> String {
    sample(
        "postgres",
        SUFFIXES[ROWS_INDEX],
        &format!("{labels},table=\"{table}\""),
        value,
    )
}

fn readings(row: &DebeziumSnapshot) -> [MetricLookup; MEASUREMENT_COUNT] {
    [
        row.running,
        row.completed,
        row.aborted,
        row.paused,
        row.skipped,
        row.duration_secs,
        row.total_tables,
        row.remaining_tables,
        row.events_total,
    ]
}

fn assert_reading(reading: MetricLookup, state: MetricState, value: Option<f64>) {
    assert_eq!(reading, MetricLookup { state, value });
}

#[test]
fn projects_exported_contexts_and_historical_rows_without_streaming_contamination() {
    let text = all_measurements(SNAPSHOT, ["1", "0", "0", "0", "0", "12", "3", "2", "9000"])
        + &table_sample(SNAPSHOT, "public.orders", "8000")
        + &table_sample(SNAPSHOT, "public.customers", "1000")
        + &all_measurements(
            INCREMENTAL,
            ["0", "0", "0", "1", "0", "60", "4", "3", "20000"],
        )
        + &table_sample(INCREMENTAL, "public.orders", "19000")
        + &all_measurements(
            STREAMING,
            ["1", "0", "0", "0", "0", "99", "9", "8", "99999"],
        )
        + &table_sample(STREAMING, "public.orders", "99999");
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    assert_eq!(rows.len(), 2);
    let incremental = &rows[0];
    let shared = &rows[1];
    for row in &rows {
        assert_eq!(row.connector, "postgres-source-connector");
        assert_eq!(row.task, "0");
        assert_eq!(row.worker, "worker-a");
        assert_eq!(row.domain, "postgres");
        assert_eq!(row.server, "lakehouse");
    }
    assert_eq!(incremental.context, "incremental-snapshot");
    assert_eq!(incremental.lifecycle(), Some("PAUSED"));
    assert_eq!(
        incremental.notification_context(),
        Some("incremental-snapshot")
    );
    assert_reading(
        incremental.events_total,
        MetricState::Available,
        Some(20000.0),
    );
    assert_eq!(incremental.rows[0].table, "public.orders");
    assert_reading(
        incremental.rows[0].rows,
        MetricState::Available,
        Some(19000.0),
    );
    assert_eq!(shared.context, "snapshot");
    assert_eq!(shared.notification_context(), None);
    assert_eq!(shared.lifecycle(), Some("RUNNING"));
    for (reading, value) in readings(shared)
        .into_iter()
        .zip([1.0, 0.0, 0.0, 0.0, 0.0, 12.0, 3.0, 2.0, 9000.0])
    {
        assert_reading(reading, MetricState::Available, Some(value));
    }
    assert_eq!(shared.rows.len(), 2);
    assert_eq!(shared.rows[0].table, "public.customers");
    assert_eq!(shared.rows[1].table, "public.orders");
    // Retained row entries need not belong to the running snapshot: no reset,
    // sum, division, or attribution is applied to the native reported values.
    assert_reading(shared.rows[0].rows, MetricState::Available, Some(1000.0));
    assert_reading(shared.rows[1].rows, MetricState::Available, Some(8000.0));
}

#[test]
fn known_non_streaming_context_can_seed_missing_snapshot_attributes() {
    for suffix in &SUFFIXES[ROWS_INDEX + 1..] {
        let rows = project(vec![worker(
            "worker-a",
            MetricState::Available,
            &sample("sql_server", suffix, SNAPSHOT, "1"),
        )]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].domain, "sql_server");
        assert_eq!(rows[0].context, "snapshot");
        assert_eq!(rows[0].rows, [] as [SnapshotTableRows; 0]);
        for reading in readings(&rows[0]) {
            assert_reading(reading, MetricState::Unavailable, None);
        }
    }
    for suffix in SUFFIXES {
        let rows = project(vec![worker(
            "worker-a",
            MetricState::Available,
            &sample("postgres", suffix, STREAMING, "1"),
        )]);
        assert!(rows.is_empty(), "{suffix}");
    }
}

#[test]
fn ignores_incomplete_labels_untagged_samples_and_unknown_suffixes() {
    let text = r#"kafka_connect_source_task_source_record_write_total{connector="common",task="0"} 50
_debezium_postgres_snapshot_running{connector="source",task="0",server="db",context="snapshot"} 1
debezium_postgres_snapshot_running{server="db",context="snapshot"} 1
debezium_postgres_snapshot_running{connector="",task="0",server="db",context="snapshot"} 1
debezium_postgres_snapshot_running{connector="source",server="db",context="snapshot"} 1
debezium_postgres_snapshot_running{connector="source",task="",server="db",context="snapshot"} 1
debezium_postgres_snapshot_running{connector="source",task="0",context="snapshot"} 1
debezium_postgres_snapshot_running{connector="source",task="0",server="",context="snapshot"} 1
debezium_postgres_snapshot_running{connector="source",task="0",server="db"} 1
debezium_postgres_snapshot_running{connector="source",task="0",server="db",context=""} 1
debezium_postgres_arbitrary_attribute{connector="source",task="0",server="db",context="snapshot"} 1
debezium__snapshot_running{connector="source",task="0",server="db",context="snapshot"} 1"#;
    assert_eq!(
        project(vec![worker("worker-a", MetricState::Available, text)]),
        [] as [DebeziumSnapshot; 0]
    );
}

#[test]
fn preserves_unknown_context_without_treating_it_as_snapshot_or_streaming() {
    let labels = SNAPSHOT.replace("snapshot", "custom-phase");
    let text = all_measurements(&labels, ["1"; MEASUREMENT_COUNT])
        + &table_sample(&labels, "public.orders", "100");
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].context, "custom-phase");
    assert_eq!(rows[0].notification_context(), None);
    assert_eq!(rows[0].lifecycle(), None);
    for reading in readings(&rows[0]) {
        assert_reading(reading, MetricState::Unavailable, None);
    }
    assert_eq!(rows[0].rows[0].table, "public.orders");
    assert_reading(rows[0].rows[0].rows, MetricState::Unavailable, None);
}

#[test]
fn separates_all_identity_dimensions_in_deterministic_order() {
    let text = r#"debezium_postgres_total_number_of_events_seen_total{connector="b",task="0",server="lakehouse",context="snapshot"} 6
debezium_sql_server_total_number_of_events_seen_total{connector="a",task="0",server="lakehouse",context="snapshot"} 4
debezium_postgres_total_number_of_events_seen_total{connector="a",task="1",server="lakehouse",context="snapshot"} 5
debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="other",context="snapshot"} 3
debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="lakehouse",context="snapshot"} 2
debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="lakehouse",context="incremental-snapshot"} 1"#;
    let other = r#"debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="remote",context="snapshot"} 7"#;
    let rows = project(vec![
        worker("worker-b", MetricState::Available, other),
        worker("worker-a", MetricState::Available, text),
    ]);
    let expected = [
        (
            "a",
            "0",
            "postgres",
            "lakehouse",
            "incremental-snapshot",
            "worker-a",
            1.0,
        ),
        (
            "a",
            "0",
            "postgres",
            "lakehouse",
            "snapshot",
            "worker-a",
            2.0,
        ),
        ("a", "0", "postgres", "other", "snapshot", "worker-a", 3.0),
        ("a", "0", "postgres", "remote", "snapshot", "worker-b", 7.0),
        (
            "a",
            "0",
            "sql_server",
            "lakehouse",
            "snapshot",
            "worker-a",
            4.0,
        ),
        (
            "a",
            "1",
            "postgres",
            "lakehouse",
            "snapshot",
            "worker-a",
            5.0,
        ),
        (
            "b",
            "0",
            "postgres",
            "lakehouse",
            "snapshot",
            "worker-a",
            6.0,
        ),
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, (connector, task, domain, server, context, worker, value)) in
        rows.iter().zip(expected)
    {
        assert_eq!(row.connector, connector);
        assert_eq!(row.task, task);
        assert_eq!(row.domain, domain);
        assert_eq!(row.server, server);
        assert_eq!(row.context, context);
        assert_eq!(row.worker, worker);
        assert_reading(row.events_total, MetricState::Available, Some(value));
    }
}

#[test]
fn invalid_booleans_and_nonnegative_counts_are_independently_unavailable() {
    for index in 0..MEASUREMENT_COUNT {
        let invalid: &[&str] = if index < BOOLEAN_COUNT {
            &["-1", "0.5", "2", "NaN", "+Inf", "-Inf"]
        } else {
            &["-1", "NaN", "+Inf", "-Inf"]
        };
        for value in invalid {
            let mut values = ["0"; MEASUREMENT_COUNT];
            values[index] = value;
            let text = all_measurements(SNAPSHOT, values);
            let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
            for (measurement, reading) in readings(&rows[0]).into_iter().enumerate() {
                if measurement == index {
                    assert_reading(reading, MetricState::Unavailable, None);
                } else {
                    assert_reading(reading, MetricState::Available, Some(0.0));
                }
            }
        }
    }
    for value in ["-1", "NaN", "+Inf", "-Inf"] {
        let text = table_sample(SNAPSHOT, "public.orders", value)
            + &table_sample(SNAPSHOT, "public.zero", "0");
        let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
        assert_reading(rows[0].rows[0].rows, MetricState::Unavailable, None);
        assert_reading(rows[0].rows[1].rows, MetricState::Available, Some(0.0));
    }
}

#[test]
fn extra_dimensions_and_duplicates_invalidate_only_the_claimed_measurement() {
    let extra = format!("{SNAPSHOT},database=\"other\"");
    for (index, suffix) in SUFFIXES[..MEASUREMENT_COUNT].iter().enumerate() {
        for state in [MetricState::Available, MetricState::Stale] {
            for text in [
                sample("postgres", suffix, &extra, "1"),
                sample("postgres", suffix, SNAPSHOT, "1")
                    + &sample("postgres", suffix, &extra, "1"),
                sample("postgres", suffix, &extra, "1")
                    + &sample("postgres", suffix, SNAPSHOT, "1"),
            ] {
                let rows = project(vec![worker("worker-a", state, &text)]);
                assert_eq!(rows.len(), 1);
                assert_reading(readings(&rows[0])[index], MetricState::Unavailable, None);
            }
            let mut metrics = worker(
                "worker-a",
                state,
                &all_measurements(SNAPSHOT, ["0"; MEASUREMENT_COUNT]),
            );
            let duplicate = metrics.samples[index].clone();
            Arc::make_mut(&mut metrics.samples).push(duplicate);
            let rows = project(vec![metrics]);
            for (measurement, reading) in readings(&rows[0]).into_iter().enumerate() {
                if measurement == index {
                    assert_reading(reading, MetricState::Unavailable, None);
                } else {
                    assert_reading(reading, state, Some(0.0));
                }
            }
        }
    }
}

#[test]
fn row_labels_preserve_table_identity_and_reject_missing_or_extra_dimensions() {
    let text = table_sample(SNAPSHOT, "public.a,b", "10")
        + &table_sample(SNAPSHOT, "public.orders", "20")
        + &table_sample(
            &format!("{SNAPSHOT},database=\"other\""),
            "public.orders",
            "999",
        )
        + &table_sample(SNAPSHOT, "", "99")
        + &sample("postgres", SUFFIXES[ROWS_INDEX], SNAPSHOT, "99");
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    assert_eq!(rows[0].rows.len(), 2);
    assert_eq!(rows[0].rows[0].table, "public.a,b");
    assert_reading(rows[0].rows[0].rows, MetricState::Available, Some(10.0));
    assert_eq!(rows[0].rows[1].table, "public.orders");
    assert_reading(rows[0].rows[1].rows, MetricState::Unavailable, None);
    let mut metrics = worker(
        "worker-a",
        MetricState::Available,
        &table_sample(SNAPSHOT, "public.orders", "20"),
    );
    let duplicate = metrics.samples[0].clone();
    Arc::make_mut(&mut metrics.samples).push(duplicate);
    let rows = project(vec![metrics]);
    assert_reading(rows[0].rows[0].rows, MetricState::Unavailable, None);
}

#[test]
fn worker_ownership_ambiguity_retains_identity_but_no_values() {
    let text = all_measurements(SNAPSHOT, ["0"; MEASUREMENT_COUNT])
        + &table_sample(SNAPSHOT, "public.orders", "10");
    for other_name in ["worker-a", "worker-b"] {
        for other_state in [
            MetricState::Available,
            MetricState::Stale,
            MetricState::CollectionError,
        ] {
            for other_text in [
                text.clone(),
                sample("postgres", "_connected", SNAPSHOT, "1"),
            ] {
                let rows = project(vec![
                    worker("worker-a", MetricState::Available, &text),
                    worker(other_name, other_state, &other_text),
                ]);
                assert_eq!(rows.len(), 2);
                for row in rows {
                    assert_eq!(row.connector, "postgres-source-connector");
                    assert_eq!(row.context, "snapshot");
                    for reading in readings(&row) {
                        assert_reading(reading, MetricState::Unavailable, None);
                    }
                    for table in row.rows {
                        assert_reading(table.rows, MetricState::Unavailable, None);
                    }
                }
            }
        }
    }
}

#[test]
fn stale_missing_invalid_error_and_unavailable_values_keep_correct_states() {
    let text = sample("postgres", "_snapshot_running", SNAPSHOT, "1")
        + &sample("postgres", "_snapshot_completed", SNAPSHOT, "2")
        + &sample(
            "postgres",
            "_total_number_of_events_seen_total",
            SNAPSHOT,
            "500",
        )
        + &table_sample(SNAPSHOT, "public.orders", "100")
        + &table_sample(SNAPSHOT, "public.invalid", "-1");
    let rows = project(vec![worker("worker-a", MetricState::Stale, &text)]);
    assert_reading(rows[0].running, MetricState::Stale, Some(1.0));
    assert_reading(rows[0].completed, MetricState::Unavailable, None);
    assert_reading(rows[0].duration_secs, MetricState::Unavailable, None);
    assert_reading(rows[0].events_total, MetricState::Stale, Some(500.0));
    assert_reading(rows[0].rows[0].rows, MetricState::Unavailable, None);
    assert_reading(rows[0].rows[1].rows, MetricState::Stale, Some(100.0));
    assert_eq!(rows[0].lifecycle(), None);
    for state in [MetricState::CollectionError, MetricState::Unavailable] {
        let text = all_measurements(INCREMENTAL, ["0"; MEASUREMENT_COUNT])
            + &table_sample(INCREMENTAL, "public.orders", "100");
        let rows = project(vec![worker("worker-a", state, &text)]);
        for reading in readings(&rows[0]) {
            assert_reading(reading, state, None);
        }
        assert_reading(rows[0].rows[0].rows, state, None);
    }
    assert_eq!(
        debezium_snapshots(&MetricsSnapshot {
            workers: vec![],
            state: MetricState::CollectionError,
        }),
        [] as [DebeziumSnapshot; 0]
    );
}

#[test]
fn unrelated_collection_errors_do_not_poison_valid_worker_values() {
    let rows = debezium_snapshots(&MetricsSnapshot {
        workers: vec![
            worker(
                "worker-a",
                MetricState::Available,
                &sample(
                    "postgres",
                    "_total_number_of_events_seen_total",
                    SNAPSHOT,
                    "50",
                ),
            ),
            worker("worker-b", MetricState::CollectionError, ""),
        ],
        state: MetricState::CollectionError,
    });
    assert_eq!(rows.len(), 1);
    assert_reading(rows[0].events_total, MetricState::Available, Some(50.0));
}

#[test]
fn lifecycle_requires_all_fresh_flags_and_conservatively_rejects_additive_skip() {
    for (active, expected) in [
        (None, "IDLE"),
        (Some(0), "RUNNING"),
        (Some(1), "COMPLETED"),
        (Some(2), "ABORTED"),
        (Some(3), "PAUSED"),
        (Some(4), "SKIPPED"),
    ] {
        let mut values = ["0"; MEASUREMENT_COUNT];
        if let Some(index) = active {
            values[index] = "1";
        }
        let text = all_measurements(INCREMENTAL, values);
        let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
        assert_eq!(rows[0].lifecycle(), Some(expected));
        for index in 0..BOOLEAN_COUNT {
            for unavailable in [
                MetricLookup {
                    state: MetricState::Unavailable,
                    value: None,
                },
                MetricLookup {
                    state: MetricState::Stale,
                    value: Some(0.0),
                },
                MetricLookup {
                    state: MetricState::Available,
                    value: Some(2.0),
                },
            ] {
                let mut row = rows[0].clone();
                match index {
                    0 => row.running = unavailable,
                    1 => row.completed = unavailable,
                    2 => row.aborted = unavailable,
                    3 => row.paused = unavailable,
                    4 => row.skipped = unavailable,
                    _ => unreachable!(),
                }
                assert_eq!(row.lifecycle(), None);
            }
        }
    }
    for first in 0..BOOLEAN_COUNT {
        for second in first + 1..BOOLEAN_COUNT {
            let mut values = ["0"; MEASUREMENT_COUNT];
            values[first] = "1";
            values[second] = "1";
            let rows = project(vec![worker(
                "worker-a",
                MetricState::Available,
                &all_measurements(INCREMENTAL, values),
            )]);
            assert_eq!(rows[0].lifecycle(), None);
        }
    }
}

#[test]
fn comparison_never_establishes_run_identity_or_associates_shared_context() {
    let text = all_measurements(INCREMENTAL, ["1", "0", "0", "0", "0", "0", "0", "0", "0"]);
    let mut rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    let row = &mut rows[0];
    assert_eq!(row.comparison(None), "no notification observed");
    for status in ["STARTED", "IN_PROGRESS", "RESUMED", "RUNNING"] {
        assert_eq!(
            row.comparison(Some(status)),
            "consistent observations; same run not established"
        );
    }
    assert_eq!(
        row.comparison(Some("COMPLETED")),
        "different observations; same run not established"
    );
    row.running.state = MetricState::Stale;
    assert_eq!(
        row.comparison(Some("STARTED")),
        "metrics unavailable or stale; notification retained"
    );
    row.context = "snapshot".to_owned();
    for status in [None, Some("STARTED"), Some("COMPLETED")] {
        assert_eq!(
            row.comparison(status),
            "exporter shares initial/incremental context; run association unavailable"
        );
    }
    assert_eq!(row.notification_context(), None);
    row.context = "custom-phase".to_owned();
    assert_eq!(row.notification_context(), None);
}

#[test]
fn comparison_matches_terminal_flags_exactly() {
    for (index, status) in [
        (1, "COMPLETED"),
        (2, "ABORTED"),
        (3, "PAUSED"),
        (4, "SKIPPED"),
    ] {
        let mut values = ["0"; MEASUREMENT_COUNT];
        values[index] = "1";
        let rows = project(vec![worker(
            "worker-a",
            MetricState::Available,
            &all_measurements(INCREMENTAL, values),
        )]);
        assert_eq!(
            rows[0].comparison(Some(status)),
            "consistent observations; same run not established",
        );
        assert_eq!(
            rows[0].comparison(Some("STARTED")),
            "different observations; same run not established",
        );
    }
}

#[test]
fn distinct_worker_contexts_do_not_create_false_ownership_ambiguity() {
    let rows = project(vec![
        worker(
            "worker-b",
            MetricState::Available,
            &sample("postgres", "_snapshot_running", SNAPSHOT, "1"),
        ),
        worker(
            "worker-a",
            MetricState::Available,
            &sample("postgres", "_snapshot_running", INCREMENTAL, "0"),
        ),
    ]);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].context, "incremental-snapshot");
    assert_eq!(rows[0].worker, "worker-a");
    assert_reading(rows[0].running, MetricState::Available, Some(0.0));
    assert_eq!(rows[1].context, "snapshot");
    assert_eq!(rows[1].worker, "worker-b");
    assert_reading(rows[1].running, MetricState::Available, Some(1.0));
}

#[test]
fn does_not_associate_one_logical_notification_source_with_multiple_connectors() {
    let other = INCREMENTAL.replace("postgres-source-connector", "other-connector");
    let values = ["1", "0", "0", "0", "0", "4", "1", "1", "20"];
    let text = all_measurements(INCREMENTAL, values) + &all_measurements(&other, values);
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    for row in rows {
        assert_eq!(row.lifecycle(), Some("RUNNING"));
        assert!(!row.notification_scope_unique);
        assert_ne!(
            row.comparison(Some("STARTED")),
            "consistent observations; same run not established"
        );
    }
}
