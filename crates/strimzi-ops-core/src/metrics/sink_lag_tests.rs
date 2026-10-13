use std::time::Duration;

use serde_json::json;

use super::*;

fn config(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

fn mapping(value: Value) -> Mapping {
    resolve_mapping("orders-sink", &config(value), "broker:9092")
}

fn mappings(mapping: Mapping) -> BTreeMap<String, Mapping> {
    BTreeMap::from([("orders-sink".to_owned(), mapping)])
}

fn measured(mapping: &Mapping, offsets: &[Option<i64>]) -> ConnectorLag {
    let mut reading = mapping.reading("orders-sink");
    reading.partitions = offsets
        .iter()
        .enumerate()
        .map(|(index, offset)| {
            partition_reading(
                "orders".to_owned(),
                i32::try_from(index).unwrap(),
                *offset,
                Ok((0, 10)),
            )
        })
        .collect();
    aggregate(&mut reading);
    reading
}

#[test]
fn resolves_default_override_and_unique_configured_topics() {
    let default = mapping(json!({"topics": " orders, orders, audit "}));
    assert_eq!(default.group.as_deref(), Some("connect-orders-sink"));
    assert!(default.error.is_none());
    assert!(
        matches!(default.subscription, Some(Subscription::Topics(topics)) if topics == ["audit", "orders"])
    );
    let overridden = mapping(json!({
        "topics": "orders", "consumer.override.group.id": "production-sinks"
    }));
    assert_eq!(overridden.group.as_deref(), Some("production-sinks"));
    assert!(overridden.error.is_none());
}

#[test]
fn rejects_unresolved_mapping_and_ambiguous_subscriptions() {
    for value in [
        json!({"topics": "orders", "consumer.override.group.id": "${file:secrets:group}"}),
        json!({"topics": "${env:TOPICS}"}),
        json!({"topics.regex": "${env:PATTERN}"}),
        json!({"topics": "orders", "topics.regex": "orders.*"}),
        json!({"topics": "orders,,audit"}),
        json!({"topics": "orders", "consumer.override.group.id": ""}),
        json!({"topics": "orders", "consumer.override.group.id": 12}),
    ] {
        assert!(mapping(value).error.is_some());
    }
}

#[test]
fn rejects_wrong_cluster_security_and_isolation_overrides() {
    for (key, value) in [
        ("consumer.override.bootstrap.servers", "other:9092"),
        ("consumer.override.bootstrap.servers", "${env:BROKERS}"),
        ("consumer.override.security.protocol", "SASL_SSL"),
        ("consumer.override.sasl.jaas.config", "secret"),
        (
            "consumer.override.ssl.truststore.location",
            "/private/store",
        ),
        ("consumer.override.isolation.level", "read_committed"),
    ] {
        let mut value_config = config(json!({"topics": "orders"}));
        value_config.insert(key.to_owned(), Value::String(value.to_owned()));
        assert!(resolve_mapping("orders-sink", &value_config, "broker:9092")
            .error
            .is_some());
    }
    let same = mapping(json!({
        "topics": "orders", "consumer.override.bootstrap.servers": " BROKER:9092 "
    }));
    assert!(same.error.is_none());
}

#[test]
fn discovery_uses_expanded_sink_info_not_tasks_or_source_offsets() {
    let value = config(json!({
        "orders-sink": {"info": {"type": "sink", "config": {"topics": "orders"}},
                        "status": {"tasks": [{"id": 0}, {"id": 1}]}},
        "orders-source": {"info": {"type": "source", "config": {"topic.prefix": "orders"}}}
    }));
    let found = discover(&value, "broker:9092").unwrap();
    assert_eq!(found.len(), 1);
    assert!(found.contains_key("orders-sink"));
    assert!(discover(
        &config(json!({"sink": {"status": {"type": "sink"}}})),
        "broker:9092"
    )
    .is_err());
}

#[test]
fn discovery_limits_and_unsafe_identifiers_fail_explicitly() {
    let mut value = Map::new();
    for index in 0..=MAX_CONNECTORS {
        value.insert(
            format!("sink-{index}"),
            json!({"info": {"type": "sink", "config": {"topics": "orders"}}}),
        );
    }
    assert!(discover(&value, "broker:9092").is_err());
    let unsafe_name = config(
        json!({"https://user:secret@host": {"info": {"type": "sink", "config": {"topics": "orders"}}}}),
    );
    assert!(discover(&unsafe_name, "broker:9092").is_err());
    assert!(!safe_identifier("https://user:secret@host"));
}

#[test]
fn regex_matches_entire_topics_and_rejects_java_only_features() {
    let regex = topic_regex(r"orders\.(created|updated).* ").err();
    assert!(regex.is_some());
    let regex = topic_regex(r"orders\.(created|updated).*").unwrap();
    assert!(regex.is_match("orders.created.eu"));
    assert!(regex.is_match("orders.updated"));
    assert!(!regex.is_match("prefix.orders.created"));
    let exact = topic_regex("orders").unwrap();
    assert!(!exact.is_match("orders.backup"));
    for unsupported in [
        r"orders\d+",
        "(?i)orders",
        "orders(?=x)",
        "[a-z]+",
        "orders{2}",
        "orders.*?",
        "orders.*+",
        r"(orders)\1",
    ] {
        assert!(topic_regex(unsupported).is_err(), "{unsupported}");
    }
}

#[test]
fn offset_boundaries_are_real_zero_or_explicitly_unavailable() {
    for (commit, low, high, lag) in [
        (Some(10), 0, 10, Some(0)),
        (Some(5), 5, 10, Some(5)),
        (Some(0), 0, 0, Some(0)),
        (Some(0), 0, i64::MAX, Some(u64::try_from(i64::MAX).unwrap())),
        (None, 0, 10, None),
        (Some(-1), 0, 10, None),
        (Some(4), 5, 10, None),
        (Some(11), 0, 10, None),
        (Some(0), -1, 10, None),
        (Some(5), 10, 5, None),
    ] {
        let reading = partition_reading("orders".to_owned(), 0, commit, Ok((low, high)));
        assert_eq!(reading.lag, lag);
        assert_eq!(reading.committed, commit);
        assert_eq!(reading.end, Some(high));
        assert_eq!(
            reading.state,
            if lag.is_some() {
                MetricState::Available
            } else {
                MetricState::Unavailable
            }
        );
        assert_eq!(reading.error.is_some(), lag.is_none());
    }
}

#[test]
fn partial_missing_offsets_never_form_an_aggregate() {
    let mapping = mapping(json!({"topics": "orders"}));
    let complete = measured(&mapping, &[Some(2), Some(3)]);
    assert_eq!(complete.total_lag, Some(15));
    assert_eq!(complete.partitions.len(), 2);
    let partial = measured(&mapping, &[Some(2), None]);
    assert_eq!(partial.total_lag, None);
    assert_eq!(partial.state, MetricState::Unavailable);
    assert_eq!(partial.partitions[0].lag, Some(8));
    assert_eq!(partial.partitions[1].lag, None);
    let missing = measured(&mapping, &[None, None]);
    assert_eq!(missing.total_lag, None);
    assert_eq!(missing.state, MetricState::Unavailable);
}

#[test]
fn partial_readings_expire_and_discovery_failure_marks_them_historical() {
    let mapping = mapping(json!({"topics": "orders"}));
    let now = Instant::now();
    let mut collector = SinkLagCollector::new(None, None);
    collector.apply(
        mappings(mapping.clone()),
        vec![measured(&mapping, &[Some(2), None])],
        now,
    );
    let expired = collector.snapshot_at(now + STALE_AFTER);
    assert_eq!(expired.connectors[0].total_lag, None);
    assert_eq!(
        expired.connectors[0].partitions[0].state,
        MetricState::Stale
    );
    assert_eq!(expired.age_secs, Some(30));
    collector.fail_all("cannot discover sink connectors");
    let failed = collector.snapshot_at(now + Duration::from_secs(1));
    assert_eq!(failed.connectors[0].total_lag, None);
    assert_eq!(failed.connectors[0].partitions[0].state, MetricState::Stale);
    assert_eq!(failed.age_secs, Some(1));
}

#[test]
fn error_and_overflow_do_not_become_zero_or_partial_sums() {
    let mapping = mapping(json!({"topics": "orders"}));
    let mut reading = measured(&mapping, &[Some(2)]);
    reading.partitions.push(partition_reading(
        "orders".to_owned(),
        1,
        Some(2),
        Err("cannot fetch log offsets"),
    ));
    aggregate(&mut reading);
    assert_eq!(reading.total_lag, None);
    assert_eq!(reading.state, MetricState::CollectionError);
    let mut overflow = mapping.reading("orders-sink");
    overflow.partitions = (0..3)
        .map(|partition| {
            partition_reading("orders".to_owned(), partition, Some(0), Ok((0, i64::MAX)))
        })
        .collect();
    aggregate(&mut overflow);
    assert_eq!(overflow.total_lag, None);
    assert_eq!(overflow.state, MetricState::Unavailable);
}

#[test]
fn failures_keep_last_good_stale_and_age_without_new_io() {
    let mapping = mapping(json!({"topics": "orders"}));
    let now = Instant::now();
    let mut collector = SinkLagCollector::new(None, None);
    collector.apply(
        mappings(mapping.clone()),
        vec![measured(&mapping, &[Some(2)])],
        now,
    );
    let fresh = collector.snapshot_at(now + Duration::from_secs(29));
    assert_eq!(fresh.state, MetricState::Available);
    assert_eq!(fresh.age_secs, Some(29));
    assert!(Arc::ptr_eq(&fresh.connectors, &collector.cached.connectors));
    let aged = collector.snapshot_at(now + STALE_AFTER);
    assert_eq!(aged.state, MetricState::Stale);
    assert_eq!(aged.connectors[0].partitions[0].state, MetricState::Stale);
    assert_eq!(aged.connectors[0].total_lag, Some(8));
    let mut failed = mapping.reading("orders-sink");
    failed.state = MetricState::CollectionError;
    failed.error = Some("cannot fetch committed offsets");
    collector.apply(
        mappings(mapping),
        vec![failed],
        now + Duration::from_secs(10),
    );
    let stale = collector.snapshot_at(now + Duration::from_secs(11));
    assert_eq!(stale.connectors[0].total_lag, Some(8));
    assert_eq!(stale.state, MetricState::Stale);
    assert_eq!(stale.age_secs, Some(11));
    collector.fail_all("cannot discover sink connectors");
    assert_eq!(collector.cached.connectors[0].state, MetricState::Stale);
    assert!(collector.cached.error.is_some());
}

#[test]
fn group_topic_and_known_partition_changes_discard_old_values() {
    for changed in [
        mapping(json!({"topics": "orders", "consumer.override.group.id": "new-group"})),
        mapping(json!({"topics": "audit"})),
        mapping(json!({"topics.regex": "orders.*"})),
    ] {
        let original = mapping(json!({"topics": "orders"}));
        let now = Instant::now();
        let mut collector = SinkLagCollector::new(None, None);
        collector.apply(
            mappings(original.clone()),
            vec![measured(&original, &[Some(2)])],
            now,
        );
        let mut failure = changed.reading("orders-sink");
        failure.state = MetricState::CollectionError;
        failure.error = Some("cannot fetch committed offsets");
        collector.apply(mappings(changed), vec![failure], now);
        assert_eq!(collector.cached.connectors[0].total_lag, None);
        assert_eq!(collector.cached.state, MetricState::CollectionError);
    }
    let mapping = mapping(json!({"topics": "orders"}));
    let now = Instant::now();
    let mut collector = SinkLagCollector::new(None, None);
    collector.apply(
        mappings(mapping.clone()),
        vec![measured(&mapping, &[Some(2)])],
        now,
    );
    let mut changed_partitions = measured(&mapping, &[Some(2), None]);
    changed_partitions.partitions[1].state = MetricState::CollectionError;
    aggregate(&mut changed_partitions);
    collector.apply(mappings(mapping), vec![changed_partitions], now);
    assert_eq!(collector.cached.connectors[0].total_lag, None);
    assert_eq!(collector.cached.connectors[0].partitions.len(), 2);
}

#[test]
fn missing_commits_clear_old_measurements_and_removed_connectors_disappear() {
    let mapping = mapping(json!({"topics": "orders"}));
    let now = Instant::now();
    let mut collector = SinkLagCollector::new(None, None);
    collector.apply(
        mappings(mapping.clone()),
        vec![measured(&mapping, &[Some(2)])],
        now,
    );
    collector.apply(
        mappings(mapping.clone()),
        vec![measured(&mapping, &[None])],
        now,
    );
    assert_eq!(
        collector.cached.connectors[0].state,
        MetricState::Unavailable
    );
    assert_eq!(collector.cached.connectors[0].total_lag, None);
    collector.apply(BTreeMap::new(), Vec::new(), now);
    assert!(collector.cached.connectors.is_empty());
    assert_eq!(collector.cached.state, MetricState::Unavailable);
}

#[test]
fn public_debug_never_contains_private_endpoints() {
    let collector = SinkLagCollector::new(
        Some("http://private-user:private-password@connect.local".to_owned()),
        Some("private-broker.local:9092".to_owned()),
    );
    let debug = format!("{collector:?}");
    assert!(!debug.contains("private"));
    assert!(!debug.contains("connect.local"));
}

#[cfg(not(feature = "kafka"))]
#[test]
fn no_kafka_feature_is_explicitly_unavailable() {
    let mut collector = SinkLagCollector::new(
        Some("http://connect".to_owned()),
        Some("broker:9092".to_owned()),
    );
    let snapshot = collector.collect();
    assert_eq!(snapshot.state, MetricState::Unavailable);
    assert_eq!(
        snapshot.error,
        Some("sink lag is unavailable without Kafka support")
    );
    assert!(snapshot.connectors.is_empty());
}
