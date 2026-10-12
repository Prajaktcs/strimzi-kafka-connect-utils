//! Connection settings shared by CLI and UI (secrets.toml + overrides).

use std::fs;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Kafka Connect / bootstrap settings loaded from secrets or CLI/UI flags.
#[derive(Debug, Clone, Default)]
pub struct ConnectionSettings {
    pub connect_url: Option<String>,
    pub bootstrap_servers: Option<String>,
    pub connect_cluster_name: Option<String>,
    /// Worker exporter membership; relative paths resolve beside secrets.toml.
    pub metrics_targets_file: Option<PathBuf>,
}

impl ConnectionSettings {
    pub fn from_secrets_file(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let value: toml::Value = toml::from_str(&text).map_err(|err| Error::Secrets {
            path: path.to_path_buf(),
            reason: err.to_string(),
        })?;

        let kafka = value.get("kafka");
        Ok(Self {
            connect_url: kafka
                .and_then(|section| section.get("connect_url"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            bootstrap_servers: kafka
                .and_then(|section| section.get("bootstrap_servers"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            connect_cluster_name: kafka
                .and_then(|section| section.get("connect_cluster_name"))
                .and_then(toml::Value::as_str)
                .map(str::to_owned),
            metrics_targets_file: kafka
                .and_then(|section| section.get("metrics_targets_file"))
                .and_then(toml::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(|value| {
                    let targets = PathBuf::from(value);
                    if targets.is_absolute() {
                        targets
                    } else {
                        path.parent()
                            .unwrap_or_else(|| Path::new("."))
                            .join(targets)
                    }
                }),
        })
    }

    #[must_use]
    pub fn merge_overrides(
        mut self,
        connect_url: Option<String>,
        bootstrap_servers: Option<String>,
        cluster_name: Option<String>,
    ) -> Self {
        if connect_url.is_some() {
            self.connect_url = connect_url;
        }
        if bootstrap_servers.is_some() {
            self.bootstrap_servers = bootstrap_servers;
        }
        if cluster_name.is_some() {
            self.connect_cluster_name = cluster_name;
        }
        self
    }

    pub fn require_connect_url(&self) -> Result<&str> {
        self.connect_url
            .as_deref()
            .ok_or_else(|| Error::MissingSetting {
                option: "connect_url (flag/env or kafka.connect_url in secrets.toml)".to_owned(),
            })
    }

    pub fn require_bootstrap_servers(&self) -> Result<&str> {
        self.bootstrap_servers
            .as_deref()
            .ok_or_else(|| Error::MissingSetting {
                option: "bootstrap_servers (flag/env or kafka.bootstrap_servers in secrets.toml)"
                    .to_owned(),
            })
    }

    pub fn cluster_name(&self) -> &str {
        self.connect_cluster_name
            .as_deref()
            .unwrap_or("my-connect-cluster")
    }
}

/// Load settings from an optional secrets path (default `./secrets.toml` when present),
/// then apply overrides.
pub fn load_settings(
    secrets: Option<&Path>,
    connect_url: Option<String>,
    bootstrap_servers: Option<String>,
    cluster_name: Option<String>,
) -> Result<ConnectionSettings> {
    let mut settings = ConnectionSettings::default();
    if let Some(path) = secrets {
        settings = ConnectionSettings::from_secrets_file(path)?;
    } else {
        let default = PathBuf::from("secrets.toml");
        if default.exists() {
            settings = ConnectionSettings::from_secrets_file(&default)?;
        }
    }
    Ok(settings.merge_overrides(connect_url, bootstrap_servers, cluster_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_overrides_over_defaults() {
        let settings = ConnectionSettings {
            connect_url: Some("http://from-file:8083".to_owned()),
            bootstrap_servers: Some("file:9092".to_owned()),
            connect_cluster_name: Some("file-cluster".to_owned()),
            ..ConnectionSettings::default()
        }
        .merge_overrides(Some("http://cli:8083".to_owned()), None, None);

        assert_eq!(settings.connect_url.as_deref(), Some("http://cli:8083"));
        assert_eq!(settings.bootstrap_servers.as_deref(), Some("file:9092"));
        assert_eq!(settings.cluster_name(), "file-cluster");
    }

    #[test]
    fn resolves_metrics_targets_beside_secrets_without_changing_absolute_paths() {
        let directory = std::env::temp_dir().join(format!(
            "strimzi-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let secrets = directory.join("secrets.toml");
        fs::write(
            &secrets,
            "[kafka]\nconnect_url='http://localhost:8083'\nmetrics_targets_file='.local/connect-metrics.toml'",
        )
        .unwrap();
        let settings = ConnectionSettings::from_secrets_file(&secrets)
            .unwrap()
            .merge_overrides(Some("http://override:8083".to_owned()), None, None);
        assert_eq!(
            settings.metrics_targets_file,
            Some(directory.join(".local/connect-metrics.toml"))
        );
        let absolute = directory.join("absolute-metrics.toml");
        fs::write(
            &secrets,
            format!("[kafka]\nmetrics_targets_file='{}'", absolute.display()),
        )
        .unwrap();
        assert_eq!(
            ConnectionSettings::from_secrets_file(&secrets)
                .unwrap()
                .metrics_targets_file,
            Some(absolute)
        );
        fs::write(&secrets, "[kafka]\nconnect_url='http://localhost:8083'").unwrap();
        assert!(ConnectionSettings::from_secrets_file(&secrets)
            .unwrap()
            .metrics_targets_file
            .is_none());
        fs::remove_dir_all(directory).unwrap();
    }
}
