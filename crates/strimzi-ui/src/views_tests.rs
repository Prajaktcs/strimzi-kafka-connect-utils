use std::sync::Arc;

use strimzi_ops_core::metrics::{
    connector_throughput, parse_prometheus, MetricsSnapshot, ThroughputState, WorkerMetrics,
};

use super::*;

fn source_projection(tasks: usize) -> Vec<ConnectorThroughput> {
    use std::fmt::Write;

    let mut text = String::new();
    for task in 0..tasks {
        writeln!(text, "kafka_connect_source_task_source_record_write_rate{{connector=\"source\",task=\"{task}\"}} 2").unwrap();
        writeln!(text, "kafka_connect_source_task_source_record_write_total{{connector=\"source\",task=\"{task}\"}} 10").unwrap();
    }
    connector_throughput(&MetricsSnapshot {
        state: MetricState::Available,
        workers: vec![WorkerMetrics {
            worker: "worker".to_owned(),
            state: MetricState::Available,
            samples: Arc::new(parse_prometheus(&text).unwrap()),
            age_secs: Some(0),
            error: None,
        }],
    })
}

#[test]
fn bounds_display_without_reducing_complete_connector_totals() {
    let (views, omitted) = throughput_views(source_projection(250), false, 200);
    assert_eq!(views[0].stages[0].rate, "500.00 (available)");
    assert_eq!(views[0].stages[0].total, "2500 (available)");
    assert_eq!(views[0].tasks.len(), 199);
    assert_eq!(omitted, 51);
    assert_eq!(views[0].tasks[0].stages[0].rate, "2.00 (available)");
}

#[test]
fn bounds_multiple_connectors_and_partial_multistage_tasks() {
    let source = source_projection(1).remove(0);
    let mut sink = source.clone();
    sink.connector = "sink".to_owned();
    sink.stages.push(sink.stages[0]);
    let stage = sink.tasks[0].stages[0];
    sink.tasks[0].stages.push(stage);
    let (views, omitted) = throughput_views(vec![source.clone(), sink, source], false, 5);
    assert_eq!(views.len(), 2);
    assert_eq!(views[1].stages.len(), 2);
    assert_eq!(views[1].tasks[0].stages.len(), 1);
    assert_eq!(omitted, 3);
    let (views, omitted) = throughput_views(source_projection(2), false, 0);
    assert_eq!(views.len(), 0);
    assert_eq!(omitted, 3);
}

#[test]
fn suppresses_expired_aggregates_but_retains_real_task_readings() {
    let (views, omitted) = throughput_views(source_projection(2), true, 200);
    assert_eq!(omitted, 0);
    assert_eq!(views[0].stages[0].rate, "stale");
    assert_eq!(views[0].stages[0].total, "stale");
    assert_eq!(views[0].tasks[0].stages[0].rate, "2.00 (stale)");
    assert_eq!(views[0].tasks[0].stages[0].total, "10 (stale)");
}

#[test]
fn expiry_preserves_ambiguous_state_instead_of_relabeling_it() {
    let mut connectors = source_projection(1);
    connectors[0].stages[0].rate = ThroughputValue {
        state: ThroughputState::Ambiguous,
        value: None,
    };
    let (views, _) = throughput_views(connectors, true, 200);
    assert_eq!(views[0].stages[0].rate, "ambiguous");
    assert_eq!(views[0].stages[0].total, "stale");
}
