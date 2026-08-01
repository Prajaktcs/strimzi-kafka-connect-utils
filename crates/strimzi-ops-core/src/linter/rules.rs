//! Built-in connector lint rules.

use serde_json::Value;

use crate::linter::types::{LintResult, Severity};

/// A named lint rule with a check function.
pub struct Rule {
    /// Stable rule identifier.
    pub id: &'static str,
    /// Default severity when no override is configured.
    pub severity: Severity,
    check: fn(&serde_json::Map<String, Value>) -> Vec<LintResult>,
}

impl Rule {
    /// Run the rule against a parsed connector config map.
    pub fn check(&self, config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
        (self.check)(config)
    }
}

/// Built-in rules matching the Python `ConnectorLinter` implementation.
pub fn builtin_rules() -> Vec<Rule> {
    vec![
        Rule {
            id: "required-field",
            severity: Severity::Error,
            check: check_required_fields,
        },
        Rule {
            id: "naming-convention",
            severity: Severity::Warning,
            check: check_naming_convention,
        },
        Rule {
            id: "tasks-max-value",
            severity: Severity::Warning,
            check: check_tasks_max,
        },
        Rule {
            id: "debezium-snapshot-mode",
            severity: Severity::Warning,
            check: check_snapshot_mode,
        },
        Rule {
            id: "sensitive-data",
            severity: Severity::Warning,
            check: check_sensitive_data,
        },
    ]
}

fn value_as_str(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn check_required_fields(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    let mut results = Vec::new();
    for field in ["name", "connector.class"] {
        if !config.contains_key(field) {
            results.push(LintResult::new(
                "required-field",
                Severity::Error,
                format!("Missing required field: {field}"),
                Some(field),
            ));
        }
    }
    results
}

fn check_naming_convention(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    let mut results = Vec::new();
    let Some(Value::String(name)) = config.get("name") else {
        return results;
    };

    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        results.push(LintResult::new(
            "naming-convention",
            Severity::Warning,
            format!("Connector name '{name}' contains invalid characters"),
            Some("name"),
        ));
    }

    if name.len() > 64 {
        results.push(LintResult::new(
            "naming-convention",
            Severity::Warning,
            format!("Connector name '{name}' is too long (max 64 characters)"),
            Some("name"),
        ));
    }

    results
}

fn check_tasks_max(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    let Some(raw) = config.get("tasks.max") else {
        return Vec::new();
    };

    let parsed = match raw {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse::<i64>().ok(),
        _ => None,
    };

    let Some(tasks) = parsed else {
        return vec![LintResult::new(
            "tasks-max-value",
            Severity::Error,
            "tasks.max must be a valid integer",
            Some("tasks.max"),
        )];
    };

    if tasks < 1 {
        return vec![LintResult::new(
            "tasks-max-value",
            Severity::Error,
            "tasks.max must be at least 1",
            Some("tasks.max"),
        )];
    }

    if tasks > 10 {
        return vec![LintResult::new(
            "tasks-max-value",
            Severity::Warning,
            format!("tasks.max is {tasks}, which may be too high"),
            Some("tasks.max"),
        )];
    }

    Vec::new()
}

fn check_snapshot_mode(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    const VALID: &[&str] = &["initial", "always", "never", "when_needed", "schema_only"];

    let Some(connector_class) = config.get("connector.class").and_then(value_as_str) else {
        return Vec::new();
    };

    if !connector_class.to_ascii_lowercase().contains("debezium") {
        return Vec::new();
    }

    let Some(mode) = config.get("snapshot.mode").and_then(value_as_str) else {
        return Vec::new();
    };

    if VALID.contains(&mode.as_str()) {
        return Vec::new();
    }

    vec![LintResult::new(
        "debezium-snapshot-mode",
        Severity::Warning,
        format!(
            "Unknown snapshot.mode '{mode}'. Valid: {}",
            VALID.join(", ")
        ),
        Some("snapshot.mode"),
    )]
}

fn check_sensitive_data(config: &serde_json::Map<String, Value>) -> Vec<LintResult> {
    const PATTERNS: &[&str] = &["password", "secret", "key", "token"];
    let mut results = Vec::new();

    for (key, value) in config {
        let key_lower = key.to_ascii_lowercase();
        if !PATTERNS.iter().any(|pattern| key_lower.contains(pattern)) {
            continue;
        }

        let Value::String(text) = value else {
            continue;
        };
        if text.is_empty() || text == "${env:VAR}" || text == "REPLACE_ME" {
            continue;
        }

        results.push(LintResult::new(
            "sensitive-data",
            Severity::Warning,
            format!("Potential sensitive data in '{key}'. Consider using environment variables"),
            Some(key.as_str()),
        ));
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> serde_json::Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn required_fields_detect_missing_name() {
        let config = map(json!({
            "connector.class": "io.debezium.connector.postgresql.PostgresConnector"
        }));
        let results = check_required_fields(&config);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path.as_deref(), Some("name"));
    }

    #[test]
    fn tasks_max_rejects_zero() {
        let config = map(json!({ "tasks.max": "0" }));
        let results = check_tasks_max(&config);
        assert_eq!(results[0].severity, Severity::Error);
    }
}
