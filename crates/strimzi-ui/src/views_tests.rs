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
    assert_eq!(views.len(), 0);
    assert_eq!(omitted, 0);
}

fn lag_snapshot(partitions: usize) -> SinkLagSnapshot {
    use strimzi_ops_core::metrics::{ConnectorLag, PartitionLag};
    SinkLagSnapshot {
        state: MetricState::Available,
        age_secs: Some(0),
        error: None,
        connectors: Arc::new(vec![ConnectorLag {
            connector: "sink".to_owned(),
            group: Some("connect-sink".to_owned()),
            state: MetricState::Available,
            total_lag: Some(partitions as u64 * 7),
            error: None,
            partitions: (0..partitions)
                .map(|partition| PartitionLag {
                    topic: "orders".to_owned(),
                    partition: i32::try_from(partition).unwrap(),
                    committed: Some(10),
                    end: Some(17),
                    lag: Some(7),
                    state: MetricState::Available,
                    error: None,
                })
                .collect(),
        }]),
    }
}

#[test]
fn lag_display_limits_do_not_recompute_complete_totals() {
    let snapshot = lag_snapshot(201);
    for limit in [0, 1, 200, 202, 203] {
        let (views, omitted) = sink_lag_views(&snapshot, false, limit);
        let displayed = views
            .iter()
            .map(|row| 1 + row.partitions.len())
            .sum::<usize>();
        assert_eq!(displayed, limit.min(202));
        assert_eq!(omitted, 202 - displayed);
        if let Some(row) = views.first() {
            assert_eq!(row.total, "1407");
        }
    }
}

#[test]
fn expired_lag_offsets_and_totals_are_historical() {
    let snapshot = lag_snapshot(1);
    let (fresh, _) = sink_lag_views(&snapshot, false, 200);
    let (expired, _) = sink_lag_views(&snapshot, true, 200);
    assert_eq!(fresh[0].total, "7");
    assert_eq!(expired[0].state, MetricState::Stale);
    assert_ne!(expired[0].total, fresh[0].total);
    let partition = &expired[0].partitions[0];
    assert_eq!(partition.state, MetricState::Stale);
    assert!(partition.committed.contains("stale"));
    assert!(partition.end.contains("stale"));
    assert!(partition.lag.contains("stale"));
}

#[test]
fn unavailable_lag_never_becomes_zero_or_a_complete_sum() {
    for state in [MetricState::Unavailable, MetricState::CollectionError] {
        let mut snapshot = lag_snapshot(1);
        let connector = &mut Arc::get_mut(&mut snapshot.connectors).unwrap()[0];
        connector.state = state;
        connector.total_lag = None;
        connector.group = None;
        connector.error = Some("cannot resolve sink consumer group");
        connector.partitions[0].state = state;
        connector.partitions[0].committed = None;
        connector.partitions[0].lag = None;
        for expired in [false, true] {
            let (views, _) = sink_lag_views(&snapshot, expired, 200);
            assert_eq!(views[0].state, state);
            assert_eq!(views[0].total, state.to_string());
            assert_eq!(views[0].group, "unavailable");
            assert!(views[0].error.is_some());
            assert_eq!(views[0].partitions[0].lag, state.to_string());
            assert_eq!(views[0].partitions[0].committed, state.to_string());
            if expired {
                assert!(views[0].partitions[0].end.contains("stale"));
            } else {
                assert_eq!(views[0].partitions[0].end, "17");
            }
        }
    }
}

#[test]
fn one_stale_group_does_not_expire_an_independent_fresh_group() {
    let mut snapshot = lag_snapshot(1);
    let mut stale = snapshot.connectors[0].clone();
    stale.connector = "older-sink".to_owned();
    stale.group = Some("older-group".to_owned());
    stale.state = MetricState::Stale;
    stale.partitions[0].state = MetricState::Stale;
    Arc::get_mut(&mut snapshot.connectors).unwrap().push(stale);
    snapshot.state = MetricState::Stale;
    snapshot.age_secs = Some(60);
    let (views, _) = sink_lag_views(&snapshot, false, 200);
    assert_eq!(views[0].state, MetricState::Available);
    assert_eq!(views[0].total, "7");
    assert_eq!(views[0].partitions[0].state, MetricState::Available);
    assert_eq!(views[1].state, MetricState::Stale);
    assert!(views[1].total.contains("stale"));
    assert_eq!(views[1].partitions[0].state, MetricState::Stale);
}
