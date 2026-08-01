//! Parse connector configuration text as JSON or YAML.

use serde_json::Value;

use crate::{Error, Result};

/// Supported connector configuration formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConfigFormat {
    /// Detect JSON first, then YAML.
    #[default]
    Auto,
    /// Strict JSON.
    Json,
    /// Strict YAML.
    Yaml,
}

impl ConfigFormat {
    /// Parse a CLI/format string (`auto`, `json`, `yaml`).
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "json" => Ok(Self::Json),
            "yaml" | "yml" => Ok(Self::Yaml),
            other => Err(Error::UnknownFormat {
                format: other.to_owned(),
            }),
        }
    }

    /// Infer format from a file extension.
    pub fn from_extension(ext: Option<&str>) -> Self {
        match ext.map(str::to_ascii_lowercase).as_deref() {
            Some("json") => Self::Json,
            Some("yaml" | "yml") => Self::Yaml,
            _ => Self::Auto,
        }
    }
}

/// Parse connector configuration text into a JSON object map.
pub fn parse_config_text(
    text: &str,
    format: ConfigFormat,
) -> Result<serde_json::Map<String, Value>> {
    let value = match format {
        ConfigFormat::Json => parse_json(text)?,
        ConfigFormat::Yaml => parse_yaml(text)?,
        ConfigFormat::Auto => match parse_json(text) {
            Ok(value) => value,
            Err(_) => parse_yaml(text).map_err(|err| {
                if let Error::Parse { reason, .. } = err {
                    Error::Parse {
                        format: "json or yaml",
                        reason,
                    }
                } else {
                    err
                }
            })?,
        },
    };

    match value {
        Value::Object(map) => Ok(map),
        _ => Err(Error::Parse {
            format: format_label(format),
            reason: "top-level value must be a mapping/object".to_owned(),
        }),
    }
}

fn format_label(format: ConfigFormat) -> &'static str {
    match format {
        ConfigFormat::Auto => "json or yaml",
        ConfigFormat::Json => "json",
        ConfigFormat::Yaml => "yaml",
    }
}

fn parse_json(text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|err| Error::Parse {
        format: "json",
        reason: err.to_string(),
    })
}

fn parse_yaml(text: &str) -> Result<Value> {
    // Convert via YAML → JSON value so lint rules see a uniform tree.
    let yaml_value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|err| Error::Parse {
        format: "yaml",
        reason: err.to_string(),
    })?;
    serde_json::to_value(yaml_value).map_err(|err| Error::Parse {
        format: "yaml",
        reason: format!("cannot normalise YAML to JSON: {err}"),
    })
}

/// Ensure `name` is present, injecting `connector_name` when the Connect REST
/// API omits it from `/connectors/{name}/config` payloads.
pub fn with_name(
    mut config: serde_json::Map<String, Value>,
    connector_name: Option<&str>,
) -> serde_json::Map<String, Value> {
    if config.contains_key("name") {
        return config;
    }
    if let Some(name) = connector_name {
        config.insert("name".to_owned(), Value::String(name.to_owned()));
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_yaml_connector() {
        let text = "name: demo\nconnector.class: foo.Bar\n";
        let map = parse_config_text(text, ConfigFormat::Yaml).unwrap();
        assert_eq!(map.get("name").and_then(Value::as_str), Some("demo"));
    }

    #[test]
    fn injects_connector_name() {
        let mut map = serde_json::Map::new();
        map.insert(
            "connector.class".to_owned(),
            Value::String("foo.Bar".to_owned()),
        );
        let map = with_name(map, Some("injected"));
        assert_eq!(map.get("name").and_then(Value::as_str), Some("injected"));
    }
}
