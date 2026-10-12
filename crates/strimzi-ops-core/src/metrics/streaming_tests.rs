use std::sync::Arc;

use super::*;
use crate::metrics::parse_prometheus;

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

fn project(workers: Vec<WorkerMetrics>) -> Vec<DebeziumStreaming> {
    debezium_streaming(&MetricsSnapshot {
        workers,
        state: MetricState::Available,
    })
}

fn readings(row: &DebeziumStreaming) -> [MetricLookup; MEASUREMENT_COUNT] {
    [
        row.connected,
        row.delay_ms,
        row.events_total,
        row.creates_total,
        row.updates_total,
        row.deletes_total,
    ]
}

fn assert_reading(reading: MetricLookup, state: MetricState, value: Option<f64>) {
    assert_eq!(reading, MetricLookup { state, value });
}

fn sample(domain: &str, suffix: &str, labels: &str, value: &str) -> String {
    format!("debezium_{domain}{suffix}{{{labels}}} {value}\n")
}

const STREAMING: &str =
    r#"connector="postgres-source-connector",task="0",server="lakehouse",context="streaming""#;
const SNAPSHOT: &str =
    r#"connector="postgres-source-connector",task="0",server="lakehouse",context="snapshot""#;

fn all_measurements(labels: &str, values: [&str; MEASUREMENT_COUNT]) -> String {
    SUFFIXES[..MEASUREMENT_COUNT]
        .iter()
        .zip(values)
        .map(|(suffix, value)| sample("postgres", suffix, labels, value))
        .collect()
}

#[test]
fn projects_real_streaming_readings_without_snapshot_counter_contamination() {
    let text = all_measurements(STREAMING, ["1", "250", "60", "10", "20", "30"])
        + &all_measurements(SNAPSHOT, ["0", "999", "6000", "1000", "2000", "3000"]);
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.connector, "postgres-source-connector");
    assert_eq!(row.task, "0");
    assert_eq!(row.worker, "worker-a");
    assert_eq!(row.domain, "postgres");
    assert_eq!(row.server, "lakehouse");
    for (reading, value) in readings(row)
        .into_iter()
        .zip([1.0, 250.0, 60.0, 10.0, 20.0, 30.0])
    {
        assert_reading(reading, MetricState::Available, Some(value));
    }
}

#[test]
fn snapshot_and_queue_attributes_seed_missing_streaming_identity() {
    for suffix in SUFFIXES {
        let text = sample("postgres", suffix, SNAPSHOT, "1");
        let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
        assert_eq!(rows.len(), 1, "{suffix}");
        for reading in readings(&rows[0]) {
            assert_reading(reading, MetricState::Unavailable, None);
        }
    }
}

#[test]
fn ignores_common_plugins_untagged_samples_and_unidentifiable_domains() {
    let text = r#"kafka_connect_source_task_source_record_write_total{connector="common",task="0"} 50
kafka_connect_connector_task_status{connector="common",task="0",status="running"} 1
debezium_postgres_connected{server="lakehouse",context="streaming"} 1
debezium_postgres_connected{connector="",task="0",server="lakehouse",context="streaming"} 1
debezium_postgres_connected{connector="source",task="",server="lakehouse",context="streaming"} 1
debezium_postgres_connected{connector="source",task="0",server="",context="streaming"} 1
debezium_postgres_connected{connector="source",task="0",server="lakehouse",context=""} 1
debezium_postgres_connected{connector="source",task="0",server="lakehouse"} 1
debezium_postgres_arbitrary_attribute{connector="source",task="0",server="lakehouse",context="streaming"} 1
debezium__connected{connector="source",task="0",server="lakehouse",context="streaming"} 1"#;
    assert_eq!(
        project(vec![worker("worker-a", MetricState::Available, text)]),
        Vec::new()
    );
}

#[test]
fn keeps_connector_task_domain_server_and_worker_identities_separate() {
    let text = r#"debezium_postgres_total_number_of_events_seen_total{connector="b",task="0",server="lakehouse",context="streaming"} 6
debezium_sql_server_total_number_of_events_seen_total{connector="a",task="0",server="lakehouse",context="streaming"} 3
debezium_postgres_total_number_of_events_seen_total{connector="a",task="1",server="lakehouse",context="streaming"} 5
debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="other",context="streaming"} 2
debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="lakehouse",context="streaming"} 1"#;
    let other = r#"debezium_postgres_total_number_of_events_seen_total{connector="a",task="0",server="remote",context="streaming"} 4"#;
    let rows = project(vec![
        worker("worker-b", MetricState::Available, other),
        worker("worker-a", MetricState::Available, text),
    ]);
    let expected = [
        ("a", "0", "postgres", "lakehouse", "worker-a", 1.0),
        ("a", "0", "postgres", "other", "worker-a", 2.0),
        ("a", "0", "postgres", "remote", "worker-b", 4.0),
        ("a", "0", "sql_server", "lakehouse", "worker-a", 3.0),
        ("a", "1", "postgres", "lakehouse", "worker-a", 5.0),
        ("b", "0", "postgres", "lakehouse", "worker-a", 6.0),
    ];
    assert_eq!(rows.len(), expected.len());
    for (row, (connector, task, domain, server, worker, value)) in rows.iter().zip(expected) {
        assert_eq!(row.connector, connector);
        assert_eq!(row.task, task);
        assert_eq!(row.domain, domain);
        assert_eq!(row.server, server);
        assert_eq!(row.worker, worker);
        assert_reading(row.events_total, MetricState::Available, Some(value));
        assert_reading(row.connected, MetricState::Unavailable, None);
    }
}

#[test]
fn negative_delay_sentinel_and_clock_skew_are_not_healthy_zero() {
    for delay in ["-1", "-200", "-0.001", "NaN", "+Inf", "-Inf"] {
        let text = all_measurements(STREAMING, ["1", delay, "0", "0", "0", "0"]);
        let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
        assert_reading(rows[0].delay_ms, MetricState::Unavailable, None);
        assert_reading(rows[0].connected, MetricState::Available, Some(1.0));
        for reading in readings(&rows[0]).into_iter().skip(2) {
            assert_reading(reading, MetricState::Available, Some(0.0));
        }
    }
}

#[test]
fn genuine_zero_including_disconnected_is_available() {
    let text = all_measurements(STREAMING, ["0"; MEASUREMENT_COUNT]);
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    for reading in readings(&rows[0]) {
        assert_reading(reading, MetricState::Available, Some(0.0));
    }
}

#[test]
fn connected_requires_exactly_boolean_numeric_values() {
    for value in ["-1", "0.5", "2", "NaN", "+Inf", "-Inf"] {
        let text = all_measurements(STREAMING, [value, "0", "0", "0", "0", "0"]);
        let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
        assert_reading(rows[0].connected, MetricState::Unavailable, None);
        assert_reading(rows[0].delay_ms, MetricState::Available, Some(0.0));
    }
}

#[test]
fn missing_measurements_do_not_borrow_another_counter() {
    let text = sample("postgres", SUFFIXES[3], STREAMING, "5");
    let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
    assert_eq!(rows.len(), 1);
    for (index, reading) in readings(&rows[0]).into_iter().enumerate() {
        if index == 3 {
            assert_reading(reading, MetricState::Available, Some(5.0));
        } else {
            assert_reading(reading, MetricState::Unavailable, None);
        }
    }
}

#[test]
fn negative_and_nonfinite_counters_are_unavailable_independently() {
    for index in 2..MEASUREMENT_COUNT {
        for value in ["-1", "NaN", "+Inf", "-Inf"] {
            let mut values = ["0"; MEASUREMENT_COUNT];
            values[index] = value;
            let text = all_measurements(STREAMING, values);
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
}

#[test]
fn stale_values_stay_stale_and_missing_or_invalid_values_stay_unavailable() {
    let text = sample("postgres", SUFFIXES[0], STREAMING, "1")
        + &sample("postgres", SUFFIXES[1], STREAMING, "-1")
        + &sample("postgres", SUFFIXES[2], STREAMING, "50")
        + &sample("postgres", SUFFIXES[4], STREAMING, "0");
    let rows = project(vec![worker("worker-a", MetricState::Stale, &text)]);
    assert_reading(rows[0].connected, MetricState::Stale, Some(1.0));
    assert_reading(rows[0].delay_ms, MetricState::Unavailable, None);
    assert_reading(rows[0].events_total, MetricState::Stale, Some(50.0));
    assert_reading(rows[0].creates_total, MetricState::Unavailable, None);
    assert_reading(rows[0].updates_total, MetricState::Stale, Some(0.0));
    assert_reading(rows[0].deletes_total, MetricState::Unavailable, None);
}

#[test]
fn collection_error_never_exposes_retained_measurements() {
    for text in [
        all_measurements(STREAMING, ["1", "25", "60", "10", "20", "30"]),
        sample("postgres", "_snapshot_completed", SNAPSHOT, "1"),
    ] {
        let rows = project(vec![worker(
            "worker-a",
            MetricState::CollectionError,
            &text,
        )]);
        assert_eq!(rows.len(), 1);
        for reading in readings(&rows[0]) {
            assert_reading(reading, MetricState::CollectionError, None);
        }
    }
    assert_eq!(
        debezium_streaming(&MetricsSnapshot {
            workers: vec![],
            state: MetricState::CollectionError,
        }),
        Vec::new()
    );
}

#[test]
fn unavailable_workers_do_not_expose_values_and_unrelated_errors_do_not_poison_tasks() {
    let text = all_measurements(STREAMING, ["1", "25", "60", "10", "20", "30"]);
    let rows = project(vec![worker("worker-a", MetricState::Unavailable, &text)]);
    for reading in readings(&rows[0]) {
        assert_reading(reading, MetricState::Unavailable, None);
    }
    let rows = debezium_streaming(&MetricsSnapshot {
        workers: vec![
            worker("worker-a", MetricState::Available, &text),
            worker("worker-b", MetricState::CollectionError, ""),
        ],
        state: MetricState::CollectionError,
    });
    assert_eq!(rows.len(), 1);
    assert_reading(rows[0].events_total, MetricState::Available, Some(60.0));
}

#[test]
fn extra_dimensions_and_duplicate_series_do_not_choose_a_healthy_reading() {
    let extra = format!("{STREAMING},table=\"orders\"");
    for (index, suffix) in SUFFIXES[..MEASUREMENT_COUNT].iter().enumerate() {
        for state in [MetricState::Available, MetricState::Stale] {
            for text in [
                sample("postgres", suffix, &extra, "1"),
                sample("postgres", suffix, STREAMING, "1")
                    + &sample("postgres", suffix, &extra, "1"),
                sample("postgres", suffix, &extra, "1")
                    + &sample("postgres", suffix, STREAMING, "1"),
            ] {
                let rows = project(vec![worker("worker-a", state, &text)]);
                assert_eq!(rows.len(), 1);
                assert_reading(readings(&rows[0])[index], MetricState::Unavailable, None);
            }
        }
    }
}

#[test]
fn direct_snapshot_duplicates_invalidate_only_the_claimed_measurement() {
    for index in 0..MEASUREMENT_COUNT {
        for state in [MetricState::Available, MetricState::Stale] {
            let text = all_measurements(STREAMING, ["0"; MEASUREMENT_COUNT]);
            let mut metrics = worker("worker-a", state, &text);
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
fn snapshot_seed_suffixes_preserve_domains_with_underscores() {
    for suffix in &SUFFIXES[MEASUREMENT_COUNT..] {
        let text = sample("sql_server", suffix, SNAPSHOT, "1");
        let rows = project(vec![worker("worker-a", MetricState::Available, &text)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].domain, "sql_server");
        for reading in readings(&rows[0]) {
            assert_reading(reading, MetricState::Unavailable, None);
        }
    }
}

#[test]
fn duplicate_worker_ownership_invalidates_all_readings_without_losing_identity() {
    let text = all_measurements(STREAMING, ["1", "25", "60", "10", "20", "30"]);
    for other_state in [
        MetricState::Available,
        MetricState::Stale,
        MetricState::CollectionError,
    ] {
        for other_text in [
            text.clone(),
            sample("postgres", "_snapshot_completed", SNAPSHOT, "1"),
        ] {
            for other_name in ["worker-b", "worker-a"] {
                let rows = project(vec![
                    worker("worker-a", MetricState::Available, &text),
                    worker(other_name, other_state, &other_text),
                ]);
                assert_eq!(rows.len(), 2);
                for row in rows {
                    assert_eq!(row.connector, "postgres-source-connector");
                    assert_eq!(row.task, "0");
                    assert_eq!(row.domain, "postgres");
                    assert_eq!(row.server, "lakehouse");
                    for reading in readings(&row) {
                        assert_reading(reading, MetricState::Unavailable, None);
                    }
                }
            }
        }
    }
}
