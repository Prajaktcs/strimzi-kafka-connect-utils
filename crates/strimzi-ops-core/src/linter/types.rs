//! Shared lint result types.

use std::fmt;

use serde::Serialize;

/// Rule severity levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Informational finding.
    Info,
    /// Non-fatal issue.
    Warning,
    /// Fatal issue that fails validation.
    Error,
}

impl Severity {
    /// Parse a severity string from configuration (`error`, `warning`, `info`).
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warning" => Some(Self::Warning),
            "info" => Some(Self::Info),
            _ => None,
        }
    }

    /// Stable string form used in JSON output and config.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Info => "info",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single lint finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintResult {
    /// Stable rule identifier (for example `required-field`).
    pub rule_id: String,
    /// Finding severity.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
    /// Optional config key path related to the finding.
    pub path: Option<String>,
}

impl LintResult {
    /// Construct a finding with an optional path.
    pub fn new(
        rule_id: impl Into<String>,
        severity: Severity,
        message: impl Into<String>,
        path: Option<impl Into<String>>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            severity,
            message: message.into(),
            path: path.map(Into::into),
        }
    }
}

impl fmt::Display for LintResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = match self.severity {
            Severity::Error => "❌",
            Severity::Warning => "⚠️ ",
            Severity::Info => "ℹ️ ",
        };
        write!(f, "{prefix} [{}] {}", self.rule_id, self.message)?;
        if let Some(path) = &self.path {
            write!(f, " at {path}")?;
        }
        Ok(())
    }
}

/// Aggregated counts by severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// Number of error findings.
    pub errors: usize,
    /// Number of warning findings.
    pub warnings: usize,
    /// Number of info findings.
    pub info: usize,
}

impl Summary {
    /// Build a summary from a slice of results.
    pub fn from_results(results: &[LintResult]) -> Self {
        let mut summary = Self::default();
        for result in results {
            match result.severity {
                Severity::Error => summary.errors += 1,
                Severity::Warning => summary.warnings += 1,
                Severity::Info => summary.info += 1,
            }
        }
        summary
    }
}
