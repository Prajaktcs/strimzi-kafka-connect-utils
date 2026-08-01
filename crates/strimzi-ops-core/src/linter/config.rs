use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::linter::types::Severity;
use crate::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct LinterConfig {
    pub disabled_rules: BTreeSet<String>,
    pub rule_severities: BTreeMap<String, Severity>,
    pub connector_exemptions: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Debug, Deserialize)]
struct RawLinterConfig {
    #[serde(default)]
    disabled_rules: Vec<String>,
    #[serde(default)]
    rule_severities: BTreeMap<String, String>,
    #[serde(default)]
    connector_exemptions: BTreeMap<String, Vec<String>>,
}

impl LinterConfig {
    /// Missing files yield the default (empty) configuration.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }

        let text = fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;

        Self::parse_toml(&text).map_err(|reason| Error::LinterConfig {
            path: path.to_path_buf(),
            reason,
        })
    }

    pub fn parse_toml(text: &str) -> std::result::Result<Self, String> {
        let raw: RawLinterConfig =
            toml::from_str(text).map_err(|err| format!("invalid TOML: {err}"))?;

        let mut rule_severities = BTreeMap::new();
        for (rule_id, severity) in raw.rule_severities {
            let Some(parsed) = Severity::parse(&severity) else {
                return Err(format!(
                    "invalid severity '{severity}' for rule '{rule_id}' (expected error, warning, or info)"
                ));
            };
            rule_severities.insert(rule_id, parsed);
        }

        Ok(Self {
            disabled_rules: raw.disabled_rules.into_iter().collect(),
            rule_severities,
            connector_exemptions: raw
                .connector_exemptions
                .into_iter()
                .map(|(name, rules)| (name, rules.into_iter().collect()))
                .collect(),
        })
    }

    pub fn is_rule_enabled(&self, rule_id: &str, connector_name: Option<&str>) -> bool {
        if self.disabled_rules.contains(rule_id) {
            return false;
        }

        if let Some(name) = connector_name {
            if let Some(exemptions) = self.connector_exemptions.get(name) {
                if exemptions.contains(rule_id) {
                    return false;
                }
            }
        }

        true
    }

    pub fn severity_for(&self, rule_id: &str, default: Severity) -> Severity {
        self.rule_severities
            .get(rule_id)
            .copied()
            .unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_disabled_rules_and_exemptions() {
        let config = LinterConfig::parse_toml(
            r#"
disabled_rules = ["sensitive-data"]

[rule_severities]
naming-convention = "info"

[connector_exemptions]
legacy = ["naming-convention"]
"#,
        )
        .unwrap();

        assert!(config.disabled_rules.contains("sensitive-data"));
        assert_eq!(
            config.severity_for("naming-convention", Severity::Warning),
            Severity::Info
        );
        assert!(!config.is_rule_enabled("naming-convention", Some("legacy")));
        assert!(config.is_rule_enabled("naming-convention", Some("other")));
    }
}
