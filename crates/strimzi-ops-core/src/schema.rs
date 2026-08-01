use serde_json::Value;

use crate::linter::types::{LintResult, Severity};

const SCHEMA_RULE: &str = "pydantic-schema";

const SNAPSHOT_MODES: &[&str] = &["initial", "always", "never", "when_needed", "schema_only"];

pub fn validate_schema(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    let mut results = Vec::new();

    if let Some(err) = require_string(config, "name") {
        results.push(err);
    }
    if let Some(err) = require_string(config, "connector.class") {
        results.push(err);
    }
    if let Some(err) = validate_tasks_max(config) {
        results.push(err);
    }

    let Some(connector_class) = config.get("connector.class").and_then(Value::as_str) else {
        return results;
    };

    if connector_class.contains("PostgresConnector") {
        results.extend(validate_debezium_postgres(config));
    } else if connector_class.contains("IcebergSinkConnector") {
        results.extend(validate_iceberg_sink(config));
    }

    results
}

fn field_error(path: &str, message: impl Into<String>) -> LintResult {
    LintResult::new(SCHEMA_RULE, Severity::Error, message, Some(path))
}

fn require_string(config: &serde_json::Map<String, Value>, key: &str) -> Option<LintResult> {
    match config.get(key) {
        Some(Value::String(_)) => None,
        None => Some(field_error(key, format!("Field required for field {key}"))),
        Some(_) => Some(field_error(
            key,
            format!("Input should be a valid string for field {key}"),
        )),
    }
}

fn require_string_keys(config: &serde_json::Map<String, Value>, keys: &[&str]) -> Vec<LintResult> {
    keys.iter()
        .filter_map(|key| require_string(config, key))
        .collect()
}

fn as_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn validate_tasks_max(config: &serde_json::Map<String, Value>) -> Option<LintResult> {
    let value = config.get("tasks.max")?;
    let Some(tasks) = as_i64(value) else {
        return Some(field_error(
            "tasks.max",
            "Input should be a valid integer for field tasks.max",
        ));
    };
    if tasks < 1 {
        return Some(field_error(
            "tasks.max",
            "Input should be greater than or equal to 1 for field tasks.max",
        ));
    }
    None
}

fn validate_debezium_postgres(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    let mut results = require_string_keys(
        config,
        &[
            "database.hostname",
            "database.user",
            "database.password",
            "database.dbname",
            "topic.prefix",
        ],
    );

    if let Some(mode) = config.get("snapshot.mode").and_then(Value::as_str) {
        if !SNAPSHOT_MODES.contains(&mode) {
            results.push(field_error(
                "snapshot.mode",
                format!("Value error, snapshot.mode must be one of {SNAPSHOT_MODES:?} for field snapshot.mode"),
            ));
        }
    }

    results
}

fn validate_iceberg_sink(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    require_string_keys(config, &["topics", "iceberg.catalog.warehouse"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn postgres_requires_database_fields() {
        let config = map(json!({
            "name": "test-postgres",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        }));
        let results = validate_schema(&config);
        assert!(results
            .iter()
            .any(|r| r.path.as_deref() == Some("database.hostname")));
    }

    #[test]
    fn invalid_snapshot_mode_errors() {
        let config = map(json!({
            "name": "test-postgres",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
            "database.hostname": "localhost",
            "database.user": "postgres",
            "database.password": "password",
            "database.dbname": "test_db",
            "topic.prefix": "test_prefix",
            "snapshot.mode": "invalid_mode",
        }));
        let results = validate_schema(&config);
        assert!(results
            .iter()
            .any(|r| r.path.as_deref() == Some("snapshot.mode")));
    }

    #[test]
    fn empty_string_fields_are_accepted() {
        let config = map(json!({
            "name": "",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
            "database.hostname": "localhost",
            "database.user": "postgres",
            "database.password": "password",
            "database.dbname": "test_db",
            "topic.prefix": "test_prefix",
        }));
        let results = validate_schema(&config);
        assert!(!results.iter().any(|r| r.path.as_deref() == Some("name")));
    }
}
