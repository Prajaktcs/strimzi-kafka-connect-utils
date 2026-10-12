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

fn streaming_projection(state: MetricState, connected: f64) -> DebeziumStreaming {
    let reading = |value| MetricLookup {
        state,
        value: Some(value),
    };
    DebeziumStreaming {
        connector: "source".to_owned(),
        task: "0".to_owned(),
        worker: "worker".to_owned(),
        domain: "postgres".to_owned(),
        server: "lakehouse".to_owned(),
        connected: reading(connected),
        delay_ms: reading(125.0),
        events_total: reading(30.0),
        creates_total: reading(10.0),
        updates_total: reading(15.0),
        deletes_total: reading(5.0),
    }
}

#[test]
fn distinguishes_current_connection_from_historical_worker_and_cache_readings() {
    for connected in [0.0, 1.0] {
        let projection = streaming_projection(MetricState::Available, connected);
        let (fresh, _) = streaming_views(vec![projection.clone()], false, 1);
        assert!(!fresh[0].connected.contains("stale"));
        assert_eq!(fresh[0].connected == "connected", connected > 0.0);
        for (state, expired) in [(MetricState::Stale, false), (MetricState::Available, true)] {
            let (historical, _) =
                streaming_views(vec![streaming_projection(state, connected)], expired, 1);
            assert_ne!(historical[0].connected, fresh[0].connected);
            assert!(historical[0].connected.contains(&fresh[0].connected));
            assert!(historical[0].connected.contains("stale"));
            for reading in [
                &historical[0].delay_ms,
                &historical[0].events_total,
                &historical[0].creates_total,
                &historical[0].updates_total,
                &historical[0].deletes_total,
            ] {
                assert!(reading.contains("stale"));
            }
            assert!(historical[0].delay_ms.starts_with("125"));
            assert!(historical[0].events_total.starts_with("30"));
        }
    }
}

#[test]
fn preserves_missing_and_error_states_when_cache_expires() {
    for state in [MetricState::Unavailable, MetricState::CollectionError] {
        for expired in [false, true] {
            let mut projection = streaming_projection(MetricState::Available, 1.0);
            projection.connected = MetricLookup { state, value: None };
            projection.delay_ms = MetricLookup { state, value: None };
            let (views, _) = streaming_views(vec![projection], expired, 1);
            assert_eq!(views[0].connected, state.to_string());
            assert_eq!(views[0].delay_ms, state.to_string());
            assert!(views[0].events_total.starts_with("30"));
            assert_eq!(views[0].events_total.contains("stale"), expired);
        }
    }
}

#[test]
fn hides_values_in_unavailable_or_error_readings() {
    for state in [MetricState::Unavailable, MetricState::CollectionError] {
        let (views, _) = streaming_views(vec![streaming_projection(state, 1.0)], true, 1);
        for reading in [
            &views[0].connected,
            &views[0].delay_ms,
            &views[0].events_total,
            &views[0].creates_total,
            &views[0].updates_total,
            &views[0].deletes_total,
        ] {
            assert_eq!(*reading, state.to_string());
        }
    }
}

#[test]
fn counts_omitted_streaming_identities_at_row_limit_boundaries() {
    let projections: Vec<_> = (0..201)
        .map(|index| {
            let mut projection = streaming_projection(MetricState::Available, 1.0);
            projection.connector = format!("source-{index}");
            projection
        })
        .collect();
    for limit in [0, 1, 200, 201, 202] {
        let (views, omitted) = streaming_views(projections.clone(), false, limit);
        assert_eq!(views.len(), limit.min(201));
        assert_eq!(omitted, 201 - views.len());
        if let Some(last) = views.last() {
            assert_eq!(last.connector, format!("source-{}", views.len() - 1));
        }
    }
    let (views, omitted) = streaming_views(Vec::new(), false, 200);
    assert!(views.is_empty());
    assert_eq!(omitted, 0);
}
