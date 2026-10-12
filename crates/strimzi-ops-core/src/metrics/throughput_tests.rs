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

fn project(workers: Vec<WorkerMetrics>) -> Vec<ConnectorThroughput> {
    let state = workers
        .iter()
        .map(|worker| worker.state)
        .max_by_key(|state| match state {
            MetricState::Unavailable => 0,
            MetricState::Available => 1,
            MetricState::Stale => 2,
            MetricState::CollectionError => 3,
        })
        .unwrap_or(MetricState::Unavailable);
    connector_throughput(&MetricsSnapshot { workers, state })
}

fn assert_value(reading: ThroughputValue, state: ThroughputState, value: Option<f64>) {
    assert_eq!(reading.state, state);
    assert_eq!(reading.value, value);
}

fn source_text(task: &str, rate: f64, total: f64) -> String {
    format!(
        "kafka_connect_source_task_source_record_write_rate{{connector=\"source\",task=\"{task}\"}} {rate}\n\
         kafka_connect_source_task_source_record_write_total{{connector=\"source\",task=\"{task}\"}} {total}"
    )
}

#[test]
fn sums_native_task_rates_and_totals_across_workers_without_averaging() {
    let projection = project(vec![
        worker(
            "worker-b",
            MetricState::Available,
            r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="1"} 20
kafka_connect_source_task_source_record_write_total{connector="source",task="1"} 100
kafka_connect_sink_task_sink_record_read_rate{connector="sink",task="1"} 8
kafka_connect_sink_task_sink_record_read_total{connector="sink",task="1"} 80
kafka_connect_sink_task_sink_record_send_rate{connector="sink",task="1"} 7
kafka_connect_sink_task_sink_record_send_total{connector="sink",task="1"} 70"#,
        ),
        worker(
            "worker-a",
            MetricState::Available,
            r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0"} 5
kafka_connect_source_task_source_record_write_total{connector="source",task="0"} 50
kafka_connect_sink_task_sink_record_read_rate{connector="sink",task="0"} 4
kafka_connect_sink_task_sink_record_read_total{connector="sink",task="0"} 40
kafka_connect_sink_task_sink_record_send_rate{connector="sink",task="0"} 3
kafka_connect_sink_task_sink_record_send_total{connector="sink",task="0"} 30"#,
        ),
    ]);
    assert_eq!(projection.len(), 2);
    let sink = &projection[0];
    assert_eq!(sink.connector, "sink");
    assert_eq!(sink.stages.len(), 2);
    assert_eq!(sink.stages[0].label, "Sink records read");
    assert_eq!(sink.stages[1].label, "Sink records sent");
    assert_value(sink.stages[0].rate, ThroughputState::Available, Some(12.0));
    assert_value(
        sink.stages[0].total,
        ThroughputState::Available,
        Some(120.0),
    );
    assert_value(sink.stages[1].rate, ThroughputState::Available, Some(10.0));
    assert_value(
        sink.stages[1].total,
        ThroughputState::Available,
        Some(100.0),
    );
    let source = &projection[1];
    assert_eq!(source.connector, "source");
    assert_eq!(source.stages.len(), 1);
    assert_eq!(source.stages[0].label, "Source records written");
    assert_value(
        source.stages[0].rate,
        ThroughputState::Available,
        Some(25.0),
    );
    assert_value(
        source.stages[0].total,
        ThroughputState::Available,
        Some(150.0),
    );
    assert_eq!(source.tasks.len(), 2);
    assert_eq!(source.tasks[0].task, "0");
    assert_eq!(source.tasks[0].worker, "worker-a");
    assert_eq!(source.tasks[1].task, "1");
    assert_eq!(source.tasks[1].worker, "worker-b");
    assert_value(
        source.tasks[0].stages[0].rate,
        ThroughputState::Available,
        Some(5.0),
    );
    assert_value(
        source.tasks[1].stages[0].total,
        ThroughputState::Available,
        Some(100.0),
    );
}

#[test]
fn missing_task_metric_invalidates_only_its_complete_aggregate() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0"} 10
kafka_connect_source_task_source_record_write_total{connector="source",task="0"} 100
kafka_connect_source_task_source_record_write_rate{connector="source",task="1"} 2
kafka_connect_source_task_source_record_poll_total{connector="source",task="1"} 999
kafka_connect_sink_task_sink_record_read_rate{connector="sink",task="0"} 3
kafka_connect_sink_task_sink_record_read_total{connector="sink",task="0"} 30
kafka_connect_sink_task_sink_record_send_rate{connector="sink",task="0"} 2
kafka_connect_sink_task_sink_record_send_total{connector="sink",task="0"} 20
kafka_connect_sink_task_sink_record_read_rate{connector="sink",task="1"} 5
kafka_connect_sink_task_sink_record_read_total{connector="sink",task="1"} 50
kafka_connect_sink_task_sink_record_send_total{connector="sink",task="1"} 30"#,
    )]);
    let sink = &projection[0];
    assert_value(sink.stages[0].rate, ThroughputState::Available, Some(8.0));
    assert_value(sink.stages[0].total, ThroughputState::Available, Some(80.0));
    assert_value(sink.stages[1].rate, ThroughputState::Unsupported, None);
    assert_value(sink.stages[1].total, ThroughputState::Available, Some(50.0));
    assert_value(
        sink.tasks[0].stages[1].rate,
        ThroughputState::Available,
        Some(2.0),
    );
    assert_value(
        sink.tasks[1].stages[1].rate,
        ThroughputState::Unsupported,
        None,
    );
    let source = &projection[1];
    assert_value(
        source.stages[0].rate,
        ThroughputState::Available,
        Some(12.0),
    );
    assert_value(source.stages[0].total, ThroughputState::Unsupported, None);
}

#[test]
fn task_status_seeds_missing_tasks_instead_of_publishing_partial_sums() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0"} 4
kafka_connect_source_task_source_record_write_total{connector="source",task="0"} 40
kafka_connect_connector_task_status{connector="source",task="1",status="running"} 1"#,
    )]);
    let source = &projection[0];
    assert_eq!(source.tasks.len(), 2);
    assert_eq!(source.tasks[1].task, "1");
    assert_value(source.stages[0].rate, ThroughputState::Unsupported, None);
    assert_value(source.stages[0].total, ThroughputState::Unsupported, None);
    assert_value(
        source.tasks[0].stages[0].rate,
        ThroughputState::Available,
        Some(4.0),
    );
    assert_value(
        source.tasks[1].stages[0].rate,
        ThroughputState::Unsupported,
        None,
    );
}

#[test]
fn zero_is_a_real_available_measurement() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        &source_text("0", 0.0, 0.0),
    )]);
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Available,
        Some(0.0),
    );
    assert_value(
        projection[0].stages[0].total,
        ThroughputState::Available,
        Some(0.0),
    );
    assert_value(
        projection[0].tasks[0].stages[0].rate,
        ThroughputState::Available,
        Some(0.0),
    );
}

#[test]
fn status_only_connectors_remain_visible_with_unknown_type() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        r#"kafka_connect_connector_status{connector="empty",status="running"} 1
kafka_connect_connector_task_status{connector="unknown",task="0",status="paused"} 1"#,
    )]);
    assert_eq!(projection.len(), 2);
    assert_eq!(projection[0].connector, "empty");
    assert!(projection[0].tasks.is_empty());
    assert_eq!(projection[1].connector, "unknown");
    assert_eq!(projection[1].tasks.len(), 1);
    for connector in &projection {
        assert_eq!(connector.stages.len(), 3);
        assert_eq!(connector.stages[0].label, "Source records written");
        assert_eq!(connector.stages[1].label, "Sink records read");
        assert_eq!(connector.stages[2].label, "Sink records sent");
        for stage in &connector.stages {
            assert_value(stage.rate, ThroughputState::Unsupported, None);
            assert_value(stage.total, ThroughputState::Unsupported, None);
        }
    }
    assert_eq!(projection[1].tasks[0].stages.len(), 3);
}

#[test]
fn other_common_metrics_infer_source_and_sink_without_fabricating_throughput() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        r#"kafka_connect_source_task_source_record_poll_total{connector="source",task="0"} 1000
kafka_connect_sink_task_sink_record_lag_max{connector="sink",task="0"} 10"#,
    )]);
    assert_eq!(projection[0].connector, "sink");
    assert_eq!(projection[0].stages.len(), 2);
    assert_eq!(projection[1].connector, "source");
    assert_eq!(projection[1].stages.len(), 1);
    for connector in &projection {
        for stage in &connector.stages {
            assert_value(stage.rate, ThroughputState::Unsupported, None);
            assert_value(stage.total, ThroughputState::Unsupported, None);
        }
    }
}

#[test]
fn unreachable_worker_invalidates_all_connector_sums_but_not_healthy_tasks() {
    let projection = project(vec![
        worker(
            "worker-a",
            MetricState::Available,
            &source_text("0", 7.0, 70.0),
        ),
        worker("worker-b", MetricState::CollectionError, ""),
    ]);
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Unreachable,
        None,
    );
    assert_value(
        projection[0].stages[0].total,
        ThroughputState::Unreachable,
        None,
    );
    assert_value(
        projection[0].tasks[0].stages[0].rate,
        ThroughputState::Available,
        Some(7.0),
    );
}

#[test]
fn stale_worker_can_hide_tasks_even_when_it_has_no_samples_for_this_connector() {
    let projection = project(vec![
        worker(
            "worker-a",
            MetricState::Available,
            &source_text("0", 7.0, 70.0),
        ),
        worker(
            "worker-b",
            MetricState::Stale,
            "kafka_connect_worker_connector_count 0",
        ),
    ]);
    assert_value(projection[0].stages[0].rate, ThroughputState::Stale, None);
    assert_value(projection[0].stages[0].total, ThroughputState::Stale, None);
    assert_value(
        projection[0].tasks[0].stages[0].total,
        ThroughputState::Available,
        Some(70.0),
    );
}

#[test]
fn stale_real_task_values_are_retained_but_not_published_as_complete_sums() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Stale,
        &source_text("0", 0.0, 12.0),
    )]);
    assert_value(projection[0].stages[0].rate, ThroughputState::Stale, None);
    assert_value(projection[0].stages[0].total, ThroughputState::Stale, None);
    assert_value(
        projection[0].tasks[0].stages[0].rate,
        ThroughputState::Stale,
        Some(0.0),
    );
    assert_value(
        projection[0].tasks[0].stages[0].total,
        ThroughputState::Stale,
        Some(12.0),
    );
}

#[test]
fn unavailable_worker_prevents_complete_sums_and_failed_task_readings_are_not_zero() {
    let projection = project(vec![
        worker(
            "worker-a",
            MetricState::Available,
            &source_text("0", 5.0, 50.0),
        ),
        worker("worker-b", MetricState::Unavailable, ""),
    ]);
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Unsupported,
        None,
    );
    assert_value(
        projection[0].tasks[0].stages[0].rate,
        ThroughputState::Available,
        Some(5.0),
    );
    let failed = project(vec![worker(
        "worker-a",
        MetricState::CollectionError,
        &source_text("0", 5.0, 50.0),
    )]);
    assert_value(
        failed[0].tasks[0].stages[0].rate,
        ThroughputState::Unreachable,
        None,
    );
    assert_value(
        failed[0].tasks[0].stages[0].total,
        ThroughputState::Unreachable,
        None,
    );
}

#[test]
fn duplicate_task_claims_never_select_an_owner_or_double_count() {
    for second_state in [MetricState::Available, MetricState::Stale] {
        let projection = project(vec![
            worker("worker-b", second_state, &source_text("0", 40.0, 400.0)),
            worker(
                "worker-a",
                MetricState::Available,
                &format!(
                    "{}\n{}",
                    source_text("0", 10.0, 100.0),
                    source_text("1", 3.0, 30.0)
                ),
            ),
        ]);
        let source = &projection[0];
        assert_eq!(source.tasks.len(), 3);
        assert_eq!(source.tasks[0].task, "0");
        assert_eq!(source.tasks[0].worker, "worker-a");
        assert_eq!(source.tasks[1].task, "0");
        assert_eq!(source.tasks[1].worker, "worker-b");
        for task in &source.tasks[..2] {
            assert_value(task.stages[0].rate, ThroughputState::Ambiguous, None);
            assert_value(task.stages[0].total, ThroughputState::Ambiguous, None);
        }
        assert_value(source.stages[0].rate, ThroughputState::Ambiguous, None);
        assert_value(source.stages[0].total, ThroughputState::Ambiguous, None);
        assert_value(
            source.tasks[2].stages[0].rate,
            ThroughputState::Available,
            Some(3.0),
        );
    }
}

#[test]
fn repeated_worker_names_cannot_silently_collapse_duplicate_task_claims() {
    let projection = project(vec![
        worker(
            "same-worker",
            MetricState::Available,
            &source_text("0", 1.0, 10.0),
        ),
        worker(
            "same-worker",
            MetricState::Available,
            &source_text("0", 2.0, 20.0),
        ),
    ]);
    assert_eq!(projection[0].tasks.len(), 2);
    for task in &projection[0].tasks {
        assert_value(task.stages[0].rate, ThroughputState::Ambiguous, None);
    }
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Ambiguous,
        None,
    );
}

#[test]
fn dimensional_duplicate_series_are_ambiguous_even_with_an_exact_identity_present() {
    for rates in [
        r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0",topic="a"} 3
kafka_connect_source_task_source_record_write_rate{connector="source",task="0",topic="b"} 7"#,
        r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0"} 3
kafka_connect_source_task_source_record_write_rate{connector="source",task="0",topic="b"} 7"#,
    ] {
        let projection = project(vec![worker(
            "worker-a",
            MetricState::Available,
            &format!(
                "{rates}\nkafka_connect_source_task_source_record_write_total{{connector=\"source\",task=\"0\"}} 100"
            ),
        )]);
        assert_value(
            projection[0].stages[0].rate,
            ThroughputState::Ambiguous,
            None,
        );
        assert_value(
            projection[0].tasks[0].stages[0].rate,
            ThroughputState::Ambiguous,
            None,
        );
        assert_value(
            projection[0].stages[0].total,
            ThroughputState::Available,
            Some(100.0),
        );
    }
}

#[test]
fn a_single_extra_dimension_is_not_an_eligible_task_measurement() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0",topic="a"} 3
kafka_connect_source_task_source_record_write_total{connector="source",task="0",topic="a"} 30"#,
    )]);
    assert_eq!(projection[0].tasks.len(), 1);
    assert_value(
        projection[0].tasks[0].stages[0].rate,
        ThroughputState::Unsupported,
        None,
    );
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Unsupported,
        None,
    );
    assert_value(
        projection[0].stages[0].total,
        ThroughputState::Unsupported,
        None,
    );
}

#[test]
fn direct_snapshot_duplicate_samples_are_ambiguous() {
    let mut first = worker(
        "worker-a",
        MetricState::Available,
        &source_text("0", 3.0, 30.0),
    );
    let duplicate = first.samples[0].clone();
    Arc::make_mut(&mut first.samples).push(duplicate);
    let projection = project(vec![first]);
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Ambiguous,
        None,
    );
    assert_value(
        projection[0].stages[0].total,
        ThroughputState::Available,
        Some(30.0),
    );
}

#[test]
fn worker_connector_and_missing_task_aggregates_never_contribute_to_task_sums() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        r#"kafka_connect_source_task_source_record_write_rate{connector="source",task="0"} 5
kafka_connect_source_task_source_record_write_total{connector="source",task="0"} 50
kafka_connect_source_task_source_record_write_rate{connector="source"} 1000
kafka_connect_source_task_source_record_write_total{connector="source"} 10000
kafka_connect_source_task_source_record_write_rate{connector="aggregate-only"} 999
kafka_connect_source_task_source_record_write_total{connector="aggregate-only"} 9999
kafka_connect_source_task_source_record_write_rate{task="1"} 100
kafka_connect_source_task_source_record_write_total 100000
kafka_connect_source_task_source_record_write_rate{connector="source",task=""} 200
kafka_connect_source_task_source_record_write_rate{connector="",task="0"} 300
kafka_connect_worker_source_record_write_rate{connector="source",task="0"} 900
kafka_connect_connector_source_record_write_total{connector="source",task="0"} 9000"#,
    )]);
    assert_eq!(projection.len(), 2);
    assert_eq!(projection[0].connector, "aggregate-only");
    assert!(projection[0].tasks.is_empty());
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Unsupported,
        None,
    );
    let source = &projection[1];
    assert_eq!(source.connector, "source");
    assert_eq!(source.tasks.len(), 1);
    assert_value(source.stages[0].rate, ThroughputState::Available, Some(5.0));
    assert_value(
        source.stages[0].total,
        ThroughputState::Available,
        Some(50.0),
    );
}

#[test]
fn negative_and_nonfinite_rates_and_counters_are_rejected_independently() {
    for invalid in ["-1", "NaN", "+Inf", "-Inf"] {
        for (rate, total) in [(invalid, "9"), ("2", invalid)] {
            let projection = project(vec![worker(
                "worker-a",
                MetricState::Available,
                &format!(
                    "kafka_connect_source_task_source_record_write_rate{{connector=\"source\",task=\"0\"}} {rate}\n\
                     kafka_connect_source_task_source_record_write_total{{connector=\"source\",task=\"0\"}} {total}"
                ),
            )]);
            let stage = projection[0].stages[0];
            if rate == invalid {
                assert_value(stage.rate, ThroughputState::Unsupported, None);
                assert_value(stage.total, ThroughputState::Available, Some(9.0));
                assert_value(
                    projection[0].tasks[0].stages[0].rate,
                    ThroughputState::Unsupported,
                    None,
                );
            } else {
                assert_value(stage.rate, ThroughputState::Available, Some(2.0));
                assert_value(stage.total, ThroughputState::Unsupported, None);
            }
        }
    }
}

#[test]
fn nonfinite_sum_is_rejected_even_when_each_task_reading_is_finite() {
    let projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        &format!(
            "{}\n{}",
            source_text("0", 1e308, 1e308),
            source_text("1", 1e308, 1e308)
        ),
    )]);
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Unsupported,
        None,
    );
    assert_value(
        projection[0].stages[0].total,
        ThroughputState::Unsupported,
        None,
    );
    for task in &projection[0].tasks {
        assert_value(task.stages[0].rate, ThroughputState::Available, Some(1e308));
        assert_value(
            task.stages[0].total,
            ThroughputState::Available,
            Some(1e308),
        );
    }
}

#[test]
fn restart_and_completed_migration_show_native_gauges_and_new_task_counters() {
    let before = project(vec![worker(
        "worker-a",
        MetricState::Available,
        &source_text("0", 4.0, 900.0),
    )]);
    let after = project(vec![worker(
        "worker-b",
        MetricState::Available,
        &source_text("0", 3.0, 2.0),
    )]);
    assert_value(
        before[0].stages[0].rate,
        ThroughputState::Available,
        Some(4.0),
    );
    assert_value(
        before[0].stages[0].total,
        ThroughputState::Available,
        Some(900.0),
    );
    assert_value(
        after[0].stages[0].rate,
        ThroughputState::Available,
        Some(3.0),
    );
    assert_value(
        after[0].stages[0].total,
        ThroughputState::Available,
        Some(2.0),
    );
    assert_eq!(after[0].tasks[0].worker, "worker-b");
}

#[test]
fn cache_expiry_preserves_real_zero_and_does_not_relabel_failures() {
    let mut projection = project(vec![worker(
        "worker-a",
        MetricState::Available,
        &source_text("0", 0.0, 5.0),
    )]);
    projection[0].stages[0].rate.expire();
    projection[0].tasks[0].stages[0].total.expire();
    assert_value(
        projection[0].stages[0].rate,
        ThroughputState::Stale,
        Some(0.0),
    );
    assert_value(
        projection[0].tasks[0].stages[0].total,
        ThroughputState::Stale,
        Some(5.0),
    );
    for state in [
        ThroughputState::Unsupported,
        ThroughputState::Stale,
        ThroughputState::Unreachable,
        ThroughputState::Ambiguous,
    ] {
        let mut reading = ThroughputValue { state, value: None };
        reading.expire();
        assert_value(reading, state, None);
    }
}
