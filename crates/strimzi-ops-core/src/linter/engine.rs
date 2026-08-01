//! Connector linter engine.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::Value;

use crate::linter::config::LinterConfig;
use crate::linter::directives::parse_lint_directives;
use crate::linter::rules::{builtin_rules, Rule};
use crate::linter::types::{LintResult, Summary};
use crate::parse::{parse_config_text, ConfigFormat};
use crate::Result;

/// Linter for Kafka Connect connector configurations.
pub struct ConnectorLinter {
    config: LinterConfig,
    rules: Vec<Rule>,
}

impl ConnectorLinter {
    /// Create a linter, loading optional config from `config_path`.
    ///
    /// When `config_path` is `None`, defaults to `.lintrc.toml` if present.
    pub fn new(config_path: Option<&Path>) -> Result<Self> {
        let path = config_path.unwrap_or_else(|| Path::new(".lintrc.toml"));
        let config = LinterConfig::load(path)?;
        Ok(Self {
            config,
            rules: builtin_rules(),
        })
    }

    /// Create a linter from an already-loaded config (useful in tests).
    pub fn with_config(config: LinterConfig) -> Self {
        Self {
            config,
            rules: builtin_rules(),
        }
    }

    /// Lint raw configuration text (YAML or JSON), including comment directives.
    pub fn lint_text(&self, text: &str, format: ConfigFormat) -> Result<Vec<LintResult>> {
        let disabled_inline = parse_lint_directives(text);
        let config = parse_config_text(text, format)?;
        Ok(self.lint(&config, &disabled_inline))
    }

    /// Lint a parsed connector configuration map.
    pub fn lint(
        &self,
        config: &serde_json::Map<String, Value>,
        disabled_inline: &BTreeSet<String>,
    ) -> Vec<LintResult> {
        let connector_name = config.get("name").and_then(Value::as_str);
        let mut results = Vec::new();

        for rule in &self.rules {
            if disabled_inline.contains(rule.id) {
                continue;
            }
            if !self.config.is_rule_enabled(rule.id, connector_name) {
                continue;
            }

            for mut result in rule.check(config) {
                result.severity = self.config.severity_for(rule.id, result.severity);
                results.push(result);
            }
        }

        results
    }

    /// Summarise findings by severity.
    pub fn summary(results: &[LintResult]) -> Summary {
        Summary::from_results(results)
    }

    /// Format findings for human-readable CLI output.
    pub fn format_results(results: &[LintResult]) -> String {
        if results.is_empty() {
            return "✅ No issues found".to_owned();
        }

        let mut sorted = results.to_vec();
        sorted.sort_by(|a, b| {
            (a.severity, a.rule_id.as_str()).cmp(&(b.severity, b.rule_id.as_str()))
        });

        let mut lines: Vec<String> = sorted.iter().map(ToString::to_string).collect();
        let summary = Self::summary(results);
        lines.push(String::new());
        lines.push(format!(
            "Summary: {} errors, {} warnings, {} info",
            summary.errors, summary.warnings, summary.info
        ));
        lines.join("\n")
    }
}
