use super::*;
use crate::metrics::parse_prometheus;
use std::sync::Arc;

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

fn queues(workers: Vec<WorkerMetrics>) -> Vec<DebeziumQueue> {
    debezium_queues(&MetricsSnapshot {
        workers,
        state: MetricState::Available,
    })
}

fn actives(workers: Vec<WorkerMetrics>) -> Vec<ConnectBackpressure> {
    connect_backpressure(&MetricsSnapshot {
        workers,
        state: MetricState::Available,
    })
}

fn assert_reading(reading: MetricLookup, state: MetricState, value: Option<f64>) {
    assert_eq!(reading, MetricLookup { state, value });
}

fn queue_sample(suffix: &str, labels: &str, value: &str) -> String {
    format!("debezium_postgres{suffix}{{{labels}}} {value}\n")
}

const STREAMING: &str =
    r#"connector="postgres-source-connector",task="0",server="lakehouse",context="streaming""#;
const SNAPSHOT: &str =
    r#"connector="postgres-source-connector",task="0",server="lakehouse",context="snapshot""#;
const SOURCE: &str = r#"connector="source-connector",task="0""#;
const SINK: &str = r#"connector="sink-connector",task="0""#;

fn queue_set(
    labels: &str,
    total: &str,
    remaining: &str,
    max_bytes: &str,
    current_bytes: &str,
) -> String {
    QUEUE_SUFFIXES
        .iter()
        .zip([total, remaining, max_bytes, current_bytes])
        // An empty value means the exporter does not export that family.
        .filter(|(_, value)| !value.is_empty())
        .map(|(suffix, value)| queue_sample(suffix, labels, value))
        .collect()
}

fn connect_sample(prefix: &str, suffix: &str, labels: &str, value: &str) -> String {
    format!("kafka_connect_{prefix}{suffix}{{{labels}}} {value}\n")
}

fn source_set(labels: &str, count: &str, max: &str, avg: &str) -> String {
    SOURCE_SUFFIXES
        .iter()
        .zip([count, max, avg])
        .map(|(suffix, value)| connect_sample("source_task_", suffix, labels, value))
        .collect()
}

fn sink_set(labels: &str, count: &str, max: &str, avg: &str, lag: &str) -> String {
    SINK_SUFFIXES
        .iter()
        .zip([count, max, avg, lag])
        .map(|(suffix, value)| connect_sample("sink_task_", suffix, labels, value))
        .collect()
}

#[test]
fn projects_queue_capacity_utilization_and_bytes() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &queue_set(STREAMING, "2048", "1024", "25000000", "10000000"),
    )]);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_reading(row.total_capacity, MetricState::Available, Some(2048.0));
    assert_reading(row.remaining_capacity, MetricState::Available, Some(1024.0));
    assert_reading(row.utilization, MetricState::Available, Some(0.5));
    assert_reading(
        row.max_queue_bytes,
        MetricState::Available,
        Some(25_000_000.0),
    );
    assert_reading(
        row.current_queue_bytes,
        MetricState::Available,
        Some(10_000_000.0),
    );
    assert_reading(row.bytes_utilization, MetricState::Available, Some(0.4));
    assert_eq!(row.domain, "postgres");
    assert_eq!(row.server, "lakehouse");
    assert_eq!(row.context, "streaming");
    assert_eq!(row.worker, "worker-a");
}

#[test]
fn snapshot_and_streaming_contexts_keep_separate_rows() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &format!(
            "{}{}",
            queue_set(STREAMING, "2048", "2048", "", ""),
            queue_set(SNAPSHOT, "2048", "512", "", "")
        ),
    )]);
    assert_eq!(rows.len(), 2);
    let streaming = rows
        .iter()
        .find(|row| row.context == "streaming")
        .expect("streaming row");
    let snapshot = rows
        .iter()
        .find(|row| row.context == "snapshot")
        .expect("snapshot row");
    assert_reading(streaming.utilization, MetricState::Available, Some(0.0));
    assert_reading(snapshot.utilization, MetricState::Available, Some(0.75));
    // Byte labels are missing independently, so byte utilization is
    // unavailable while record utilization still reads the same queue pair.
    assert_eq!(snapshot.max_queue_bytes, absent(MetricState::Unavailable));
    assert_eq!(snapshot.bytes_utilization, absent(MetricState::Unavailable));
}

#[test]
fn zero_total_capacity_is_unavailable_not_zero() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &queue_set(STREAMING, "0", "0", "0", "0"),
    )]);
    let row = &rows[0];
    assert_reading(row.total_capacity, MetricState::Available, Some(0.0));
    assert_reading(row.remaining_capacity, MetricState::Available, Some(0.0));
    assert_eq!(row.utilization, absent(MetricState::Unavailable));
    assert_reading(row.max_queue_bytes, MetricState::Available, Some(0.0));
    assert_eq!(row.bytes_utilization, absent(MetricState::Unavailable));
}

#[test]
fn remaining_above_total_is_inconsistent_and_derived_only() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &queue_set(STREAMING, "100", "200", "1000", "500"),
    )]);
    let row = &rows[0];
    assert_reading(row.total_capacity, MetricState::Available, Some(100.0));
    assert_reading(row.remaining_capacity, MetricState::Available, Some(200.0));
    assert_eq!(row.utilization, absent(MetricState::Unavailable));
    assert_reading(row.bytes_utilization, MetricState::Available, Some(0.5));
}

#[test]
fn negative_and_nonfinite_values_are_unavailable() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &queue_set(STREAMING, "-5", "NaN", "+Inf", "-1"),
    )]);
    let row = &rows[0];
    assert_eq!(row.total_capacity, absent(MetricState::Unavailable));
    assert_eq!(row.remaining_capacity, absent(MetricState::Unavailable));
    assert_eq!(row.utilization, absent(MetricState::Unavailable));
    assert_eq!(row.max_queue_bytes, absent(MetricState::Unavailable));
    assert_eq!(row.current_queue_bytes, absent(MetricState::Unavailable));
    assert_eq!(row.bytes_utilization, absent(MetricState::Unavailable));
}

#[test]
fn custom_metric_tag_dimensions_are_unavailable() {
    let labeled = format!("{STREAMING},database=\"salesdb\"");
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &queue_set(&labeled, "2048", "1024", "", ""),
    )]);
    assert_eq!(rows.len(), 1);
    for reading in [
        rows[0].total_capacity,
        rows[0].remaining_capacity,
        rows[0].utilization,
        rows[0].max_queue_bytes,
        rows[0].current_queue_bytes,
        rows[0].bytes_utilization,
    ] {
        assert_eq!(reading, absent(MetricState::Unavailable));
    }
}

#[test]
fn duplicate_series_do_not_choose_a_winner() {
    // The custom-tag variant shares the four-label identity, so the extra-
    // dimension export collides with the complete one and stays ambiguous.
    let text = queue_set(STREAMING, "2048", "1024", "", "")
        + &queue_sample(
            QUEUE_SUFFIXES[0],
            &format!("{STREAMING},database=\"salesdb\""),
            "2048",
        );
    let rows = queues(vec![worker("worker-a", MetricState::Available, &text)]);
    let row = &rows[0];
    assert_eq!(row.total_capacity, absent(MetricState::Unavailable));
    assert_reading(row.remaining_capacity, MetricState::Available, Some(1024.0));
    assert_eq!(row.utilization, absent(MetricState::Unavailable));
}

#[test]
fn duplicate_worker_ownership_invalidates_readings_but_keeps_rows() {
    let text = queue_set(STREAMING, "2048", "1024", "", "");
    let rows = queues(vec![
        worker("worker-a", MetricState::Available, &text),
        worker("worker-b", MetricState::Available, &text),
    ]);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.total_capacity, absent(MetricState::Unavailable));
        assert_eq!(row.utilization, absent(MetricState::Unavailable));
    }
    let names: [&str; 2] = ["worker-a", "worker-b"];
    for (row, name) in rows.iter().zip(names) {
        assert_eq!(row.worker, name);
    }
}

#[test]
fn stale_workers_keep_values_with_stale_state() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Stale,
        &queue_set(STREAMING, "2048", "1024", "", ""),
    )]);
    let row = &rows[0];
    assert_reading(row.total_capacity, MetricState::Stale, Some(2048.0));
    assert_reading(row.utilization, MetricState::Stale, Some(0.5));
    assert_eq!(row.max_queue_bytes, absent(MetricState::Unavailable));
}

#[test]
fn collection_error_workers_never_expose_values() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::CollectionError,
        &queue_set(STREAMING, "2048", "1024", "", ""),
    )]);
    let row = &rows[0];
    for reading in [
        row.total_capacity,
        row.remaining_capacity,
        row.utilization,
        row.max_queue_bytes,
        row.current_queue_bytes,
        row.bytes_utilization,
    ] {
        assert_eq!(reading, absent(MetricState::CollectionError));
    }
    // The same identity exported by a healthy worker is unaffected, unless it
    // claims ambiguous ownership, which this test avoids.
    let healthy = queues(vec![
        worker(
            "worker-a",
            MetricState::CollectionError,
            &queue_set(STREAMING, "2048", "1024", "", ""),
        ),
        worker(
            "worker-b",
            MetricState::Available,
            &queue_set(
                &STREAMING.replace("postgres-source", "mysql-source"),
                "1024",
                "256",
                "",
                "",
            ),
        ),
    ]);
    let mysql = &healthy[0];
    assert_eq!(mysql.connector, "mysql-source-connector");
    assert_reading(mysql.utilization, MetricState::Available, Some(0.75));
}

#[test]
fn unrelated_worker_errors_do_not_poison_other_rows() {
    let rows = queues(vec![
        worker(
            "worker-a",
            MetricState::CollectionError,
            "debezium_postgres_queue_total_capacity 1\n",
        ),
        worker(
            "worker-b",
            MetricState::Available,
            &queue_set(STREAMING, "2048", "1536", "", ""),
        ),
    ]);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.worker, "worker-b");
    assert_reading(row.utilization, MetricState::Available, Some(0.25));
}

#[test]
fn unknown_metrics_and_untagged_domains_are_ignored() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &format!(
            "debezium_postgres_queue_size{{{STREAMING}}} 42\n\
             kafka_connect_source_task_source_record_active_count{{{SOURCE}}} 3\n"
        ),
    )]);
    assert_eq!(rows, Vec::<DebeziumQueue>::new());
}

#[test]
fn identifies_nested_domains_and_components() {
    let rows = queues(vec![worker(
        "worker-a",
        MetricState::Available,
        &queue_set(STREAMING, "2048", "1024", "", ""),
    )]);
    assert_eq!(rows[0].domain, "postgres");
    // A component-style name strips only the known suffix.
    let text = queue_sample(QUEUE_SUFFIXES[3], STREAMING, "1024");
    let rows = queues(vec![worker("worker-a", MetricState::Available, &text)]);
    assert_eq!(rows[0].domain, "postgres");
}

#[test]
fn projects_source_active_counts_and_marks_lag_unsupported() {
    let rows = actives(vec![worker(
        "worker-a",
        MetricState::Available,
        &source_set(SOURCE, "3", "9", "2.5"),
    )]);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, TaskKind::Source);
    assert_reading(row.active_records, MetricState::Available, Some(3.0));
    assert_reading(row.active_records_max, MetricState::Available, Some(9.0));
    assert_reading(row.active_records_avg, MetricState::Available, Some(2.5));
    assert_eq!(row.consumer_lag_max, absent(MetricState::Unavailable));
}

#[test]
fn sinks_expose_active_counts_and_consumer_lag() {
    let rows = actives(vec![worker(
        "worker-a",
        MetricState::Available,
        &sink_set(SINK, "12", "120", "15.25", "3400"),
    )]);
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.kind, TaskKind::Sink);
    assert_reading(row.active_records, MetricState::Available, Some(12.0));
    assert_reading(row.consumer_lag_max, MetricState::Available, Some(3400.0));
}

#[test]
fn connect_dimensions_duplicates_ownership_and_untagged_samples() {
    // A lone extra-dimension series is unsupported instead of collapsed.
    let extra = sink_set(
        r#"connector="sink-connector",task="0",partition="1""#,
        "12",
        "120",
        "15",
        "3400",
    );
    let rows = actives(vec![worker("worker-a", MetricState::Available, &extra)]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].active_records, absent(MetricState::Unavailable));
    assert_eq!(rows[0].active_records_max, absent(MetricState::Unavailable));
    assert_eq!(rows[0].active_records_avg, absent(MetricState::Unavailable));
    assert_eq!(rows[0].consumer_lag_max, absent(MetricState::Unavailable));

    // The extra-dimension export collides with the identical complete one.
    let duplicate = sink_set(SINK, "12", "120", "15", "3400")
        + &sink_sample(SINK_SUFFIXES[0], &format!("{SINK},partition=\"1\""), "12");
    let rows = actives(vec![worker("worker-a", MetricState::Available, &duplicate)]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].active_records, absent(MetricState::Unavailable));
    assert_reading(
        rows[0].consumer_lag_max,
        MetricState::Available,
        Some(3400.0),
    );

    // Duplicate worker ownership invalidates every reading but keeps rows.
    let text = sink_set(SINK, "12", "120", "15", "3400");
    let rows = actives(vec![
        worker("worker-a", MetricState::Available, &text),
        worker("worker-b", MetricState::Available, &text),
    ]);
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.active_records, absent(MetricState::Unavailable));
        assert_eq!(row.consumer_lag_max, absent(MetricState::Unavailable));
    }

    // A family sample without both core labels builds no identity at all.
    let untagged = sink_set("task=\"0\"", "12", "120", "15", "3400");
    let rows = actives(vec![worker("worker-a", MetricState::Available, &untagged)]);
    assert_eq!(rows, Vec::<ConnectBackpressure>::new());
}

fn sink_sample(suffix: &str, labels: &str, value: &str) -> String {
    format!("kafka_connect_sink_task_{suffix}{{{labels}}} {value}\n")
}

#[test]
fn stale_collection_error_and_negative_connect_values() {
    let stale = actives(vec![worker(
        "worker-a",
        MetricState::Stale,
        &source_set(SOURCE, "3", "9", "2.5"),
    )]);
    assert_reading(stale[0].active_records, MetricState::Stale, Some(3.0));

    let errored = actives(vec![worker(
        "worker-a",
        MetricState::CollectionError,
        &source_set(SOURCE, "3", "9", "2.5"),
    )]);
    assert_eq!(
        errored[0].active_records,
        absent(MetricState::CollectionError)
    );
    assert_eq!(
        errored[0].consumer_lag_max,
        absent(MetricState::CollectionError)
    );

    let negative = actives(vec![worker(
        "worker-a",
        MetricState::Available,
        &source_set(SOURCE, "-3", "9", "2.5"),
    )]);
    assert_eq!(negative[0].active_records, absent(MetricState::Unavailable));
}

#[test]
fn orders_rows_by_identity() {
    let mut text = sink_set(
        "connector=\"sink-connector\",task=\"1\"",
        "12",
        "120",
        "15",
        "3400",
    );
    text.push_str(&sink_set(
        "connector=\"sink-connector\",task=\"0\"",
        "1",
        "2",
        "3",
        "4",
    ));
    text.push_str(&sink_set(
        "connector=\"aaa-sink\",task=\"0\"",
        "5",
        "6",
        "7",
        "8",
    ));
    text.push_str(&source_set(
        "connector=\"aaa-sink\",task=\"0\"",
        "5",
        "6",
        "7",
    ));
    let rows = actives(vec![worker("worker-a", MetricState::Available, &text)]);
    let keys: Vec<(TaskKind, &str, &str)> = rows
        .iter()
        .map(|row| (row.kind, row.connector.as_str(), row.task.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            (TaskKind::Source, "aaa-sink", "0"),
            (TaskKind::Sink, "aaa-sink", "0"),
            (TaskKind::Sink, "sink-connector", "0"),
            (TaskKind::Sink, "sink-connector", "1"),
        ]
    );
    // Queue rows order by connector, task, domain, server, context.
    let mut text = queue_set(STREAMING, "1", "1", "", "");
    text.push_str(&queue_set(SNAPSHOT, "2", "2", "", ""));
    let rows = queues(vec![worker("worker-a", MetricState::Available, &text)]);
    let contexts: Vec<&str> = rows.iter().map(|row| row.context.as_str()).collect();
    assert_eq!(contexts, vec!["snapshot", "streaming"]);
}
