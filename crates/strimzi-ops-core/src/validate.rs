use std::collections::BTreeSet;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::linter::directives::parse_lint_directives;
use crate::linter::engine::ConnectorLinter;
use crate::linter::types::{LintResult, Summary};
use crate::parse::{parse_config_text, with_name, ConfigFormat};
use crate::schema::validate_schema;
use crate::Result;

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub results: Vec<LintResult>,
    pub summary: Summary,
    #[serde(skip_serializing)]
    pub formatted: String,
}

pub fn validate_config(
    config: serde_json::Map<String, Value>,
    connector_name: Option<&str>,
    linter_config_path: Option<&Path>,
) -> Result<ValidationReport> {
    let linter = ConnectorLinter::new(linter_config_path)?;
    let config = with_name(config, connector_name);
    let mut results = linter.lint(&config, &BTreeSet::new());
    results.extend(validate_schema(&config));
    Ok(build_report(results))
}

pub fn validate_text(
    text: &str,
    format: ConfigFormat,
    connector_name: Option<&str>,
    linter_config_path: Option<&Path>,
) -> Result<ValidationReport> {
    let linter = ConnectorLinter::new(linter_config_path)?;
    let disabled_inline = parse_lint_directives(text);
    let config = parse_config_text(text, format)?;
    let config = with_name(config, connector_name);

    let mut results = linter.lint(&config, &disabled_inline);
    results.extend(validate_schema(&config));
    Ok(build_report(results))
}

fn build_report(results: Vec<LintResult>) -> ValidationReport {
    let summary = ConnectorLinter::summary(&results);
    let formatted = ConnectorLinter::format_results(&results);
    ValidationReport {
        valid: summary.errors == 0,
        results,
        summary,
        formatted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn valid_postgres_connector() {
        let config = map(json!({
            "name": "test-postgres",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
            "tasks.max": "1",
            "database.hostname": "localhost",
            "database.port": "5432",
            "database.user": "postgres",
            "database.password": "password",
            "database.dbname": "test_db",
            "topic.prefix": "test_prefix",
        }));
        let report =
            validate_config(config, None, Some(Path::new("nonexistent-lintrc.toml"))).unwrap();
        assert!(report.valid);
        assert_eq!(report.summary.errors, 0);
    }

    #[test]
    fn missing_postgres_fields_invalid() {
        let config = map(json!({
            "name": "test-postgres",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        }));
        let report =
            validate_config(config, None, Some(Path::new("nonexistent-lintrc.toml"))).unwrap();
        assert!(!report.valid);
        assert!(report
            .results
            .iter()
            .any(|r| r.rule_id == "pydantic-schema"));
    }

    #[test]
    fn connect_api_config_without_name() {
        let config = map(json!({
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
            "tasks.max": "1",
            "database.hostname": "localhost",
            "database.user": "postgres",
            "database.password": "password",
            "database.dbname": "test_db",
            "topic.prefix": "test_prefix",
        }));

        let missing = validate_config(
            config.clone(),
            None,
            Some(Path::new("nonexistent-lintrc.toml")),
        )
        .unwrap();
        assert!(!missing.valid);

        let with_name = validate_config(
            config,
            Some("test-postgres"),
            Some(Path::new("nonexistent-lintrc.toml")),
        )
        .unwrap();
        assert!(with_name.valid);
    }

    #[test]
    fn valid_iceberg_connector() {
        let config = map(json!({
            "name": "test-iceberg",
            "connector.class": "org.apache.iceberg.connect.IcebergSinkConnector",
            "topics": "topic1,topic2",
            "iceberg.catalog.warehouse": "s3://bucket/warehouse",
        }));
        let report =
            validate_config(config, None, Some(Path::new("nonexistent-lintrc.toml"))).unwrap();
        assert!(report.valid);
    }

    #[test]
    fn invalid_tasks_max() {
        let config = map(json!({
            "name": "test-postgres",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
            "tasks.max": "0",
            "database.hostname": "localhost",
            "database.user": "postgres",
            "database.password": "password",
            "database.dbname": "test_db",
            "topic.prefix": "test_prefix",
        }));
        let report =
            validate_config(config, None, Some(Path::new("nonexistent-lintrc.toml"))).unwrap();
        assert!(!report.valid);
        assert!(report
            .results
            .iter()
            .any(|r| r.rule_id == "pydantic-schema" && r.path.as_deref() == Some("tasks.max")));
    }

    #[test]
    fn invalid_snapshot_mode() {
        let config = map(json!({
            "name": "test-postgres",
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
            "tasks.max": "1",
            "database.hostname": "localhost",
            "database.user": "postgres",
            "database.password": "password",
            "database.dbname": "test_db",
            "topic.prefix": "test_prefix",
            "snapshot.mode": "invalid_mode",
        }));
        let report =
            validate_config(config, None, Some(Path::new("nonexistent-lintrc.toml"))).unwrap();
        assert!(!report.valid);
        assert!(report
            .results
            .iter()
            .any(|r| { r.rule_id == "pydantic-schema" && r.message.contains("snapshot.mode") }));
    }
}
