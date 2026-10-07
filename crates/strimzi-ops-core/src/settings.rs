//! Connection settings shared by CLI and UI (secrets.toml + overrides).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Kafka Connect / bootstrap settings for one cluster.
#[derive(Debug, Clone, Default)]
pub struct ConnectionSettings {
    pub connect_url: Option<String>,
    pub bootstrap_servers: Option<String>,
    pub connect_cluster_name: Option<String>,
    pub namespace: Option<String>,
}

/// How to load Connect clusters: `secrets.toml` or `KafkaConnect` CRs.
#[derive(Debug, Clone, Default)]
pub struct LoadConfig {
    pub secrets: Option<PathBuf>,
    pub connect_url: Option<String>,
    pub bootstrap_servers: Option<String>,
    pub cluster_name: Option<String>,
    /// Select an exact cluster ID. Discovered IDs are `namespace_name`, with dots
    /// in the Kubernetes resource name retained verbatim.
    pub cluster_id: Option<String>,
    pub from_k8s: bool,
    pub k8s_namespace: Option<String>,
}

/// A Kafka Connect cluster from `[[clusters]]`, legacy `[kafka]`, or Kubernetes.
#[derive(Debug, Clone)]
pub struct ConnectCluster {
    pub id: String,
    pub settings: ConnectionSettings,
}

impl ConnectionSettings {
    pub fn from_secrets_file(path: &Path) -> Result<Self> {
        Ok(ConnectCluster::from_secrets_file(path)?
            .into_iter()
            .next()
            .map(|cluster| cluster.settings)
            .unwrap_or_default())
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

impl ConnectCluster {
    pub fn from_secrets_file(path: &Path) -> Result<Vec<Self>> {
        let text = fs::read_to_string(path).map_err(|source| Error::Read {
            path: path.to_path_buf(),
            source,
        })?;
        clusters_from_toml(&text, path)
    }
}

fn clusters_from_toml(text: &str, path: &Path) -> Result<Vec<ConnectCluster>> {
    let value: toml::Value = toml::from_str(text).map_err(|err| Error::Secrets {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;

    if let Some(array) = value.get("clusters").and_then(toml::Value::as_array) {
        let mut clusters = Vec::with_capacity(array.len());
        let mut seen = HashSet::new();
        for (index, entry) in array.iter().enumerate() {
            let settings = settings_from_table(entry);
            let id = cluster_id(entry, &settings, index, path)?;
            if !seen.insert(id.clone()) {
                return Err(Error::Secrets {
                    path: path.to_path_buf(),
                    reason: format!("duplicate cluster id '{id}'"),
                });
            }
            clusters.push(ConnectCluster { id, settings });
        }
        return Ok(clusters);
    }

    let Some(kafka) = value.get("kafka") else {
        return Ok(Vec::new());
    };
    let settings = settings_from_table(kafka);
    let id = settings
        .connect_cluster_name
        .as_deref()
        .filter(|name| is_valid_cluster_id(name))
        .unwrap_or("default")
        .to_owned();
    Ok(vec![ConnectCluster { id, settings }])
}

fn settings_from_table(value: &toml::Value) -> ConnectionSettings {
    ConnectionSettings {
        connect_url: value
            .get("connect_url")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        bootstrap_servers: value
            .get("bootstrap_servers")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        connect_cluster_name: value
            .get("connect_cluster_name")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
        namespace: value
            .get("namespace")
            .and_then(toml::Value::as_str)
            .map(str::to_owned),
    }
}

fn cluster_id(
    entry: &toml::Value,
    settings: &ConnectionSettings,
    index: usize,
    path: &Path,
) -> Result<String> {
    if let Some(id) = entry.get("id").and_then(toml::Value::as_str) {
        if is_valid_cluster_id(id) {
            return Ok(id.to_owned());
        }
        return Err(Error::Secrets {
            path: path.to_path_buf(),
            reason: format!("cluster id '{id}' must match [A-Za-z0-9][A-Za-z0-9_-]*"),
        });
    }
    if let Some(name) = settings
        .connect_cluster_name
        .as_deref()
        .filter(|name| is_valid_cluster_id(name))
    {
        return Ok(name.to_owned());
    }
    if index == 0 {
        return Ok("default".to_owned());
    }
    Err(Error::Secrets {
        path: path.to_path_buf(),
        reason: format!("clusters[{index}] is missing id"),
    })
}

fn is_valid_cluster_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn read_secrets(secrets: Option<&Path>) -> Result<Vec<ConnectCluster>> {
    if let Some(path) = secrets {
        return ConnectCluster::from_secrets_file(path);
    }
    let default = PathBuf::from("secrets.toml");
    if default.exists() {
        return ConnectCluster::from_secrets_file(&default);
    }
    Ok(Vec::new())
}

/// Load the only configured cluster (or empty defaults), then apply overrides.
///
/// Returns [`Error::MultipleClusters`] when more than one cluster is configured.
/// Use [`LoadConfig::load_settings`] with `cluster_id` to select among them.
pub fn load_settings(
    secrets: Option<&Path>,
    connect_url: Option<String>,
    bootstrap_servers: Option<String>,
    cluster_name: Option<String>,
) -> Result<ConnectionSettings> {
    LoadConfig {
        secrets: secrets.map(Path::to_path_buf),
        connect_url,
        bootstrap_servers,
        cluster_name,
        ..LoadConfig::default()
    }
    .load_settings()
}

/// Load every named cluster, then apply CLI/UI overrides to the first cluster.
pub fn load_clusters(
    secrets: Option<&Path>,
    connect_url: Option<String>,
    bootstrap_servers: Option<String>,
    cluster_name: Option<String>,
) -> Result<Vec<ConnectCluster>> {
    LoadConfig {
        secrets: secrets.map(Path::to_path_buf),
        connect_url,
        bootstrap_servers,
        cluster_name,
        ..LoadConfig::default()
    }
    .load_clusters()
}

impl LoadConfig {
    /// Load every Connect cluster from Kubernetes when `from_k8s` is set,
    /// otherwise from the secrets file. An empty inventory yields empty defaults.
    ///
    /// Overrides apply to `cluster_id` when found, otherwise to the first cluster.
    pub fn load_clusters(&self) -> Result<Vec<ConnectCluster>> {
        let mut clusters = if self.from_k8s {
            let list = crate::k8s::fetch_kafkaconnect_list(self.k8s_namespace.as_deref())?;
            clusters_from_kafkaconnect_list(&list)?
        } else {
            read_secrets(self.secrets.as_deref())?
        };
        let has_override = self.connect_url.is_some()
            || self.bootstrap_servers.is_some()
            || self.cluster_name.is_some();
        if clusters.is_empty() {
            clusters.push(ConnectCluster {
                id: "default".to_owned(),
                settings: ConnectionSettings::default(),
            });
        }
        if has_override {
            let index = self
                .cluster_id
                .as_deref()
                .and_then(|id| clusters.iter().position(|cluster| cluster.id == id))
                .unwrap_or(0);
            let id = clusters[index].id.clone();
            let merged = clusters.remove(index).settings.merge_overrides(
                self.connect_url.clone(),
                self.bootstrap_servers.clone(),
                self.cluster_name.clone(),
            );
            clusters.insert(
                index,
                ConnectCluster {
                    id,
                    settings: merged,
                },
            );
        }
        Ok(clusters)
    }

    /// Load the selected cluster, or the only cluster when no ID is specified.
    ///
    /// Returns [`Error::UnknownCluster`] for an unknown `cluster_id` and
    /// [`Error::MultipleClusters`] when multiple clusters exist without a selector.
    pub fn load_settings(&self) -> Result<ConnectionSettings> {
        let clusters = self.load_clusters()?;
        if let Some(id) = self.cluster_id.as_deref() {
            return clusters
                .into_iter()
                .find(|cluster| cluster.id == id)
                .map(|cluster| cluster.settings)
                .ok_or_else(|| Error::UnknownCluster { id: id.to_owned() });
        }
        if clusters.len() > 1 {
            return Err(Error::MultipleClusters {
                count: clusters.len(),
            });
        }
        Ok(clusters
            .into_iter()
            .next()
            .map(|cluster| cluster.settings)
            .unwrap_or_default())
    }
}

fn is_valid_dns_name(name: &str) -> bool {
    name.split('.').all(|label| {
        let is_alphanumeric = |ch: u8| ch.is_ascii_lowercase() || ch.is_ascii_digit();
        label
            .as_bytes()
            .first()
            .copied()
            .is_some_and(is_alphanumeric)
            && label
                .as_bytes()
                .last()
                .copied()
                .is_some_and(is_alphanumeric)
            && label.bytes().all(|ch| is_alphanumeric(ch) || ch == b'-')
    })
}

/// Discovered selectors are always `namespace_name`, independent of inventory
/// order or other resources. The underscore cannot occur in Kubernetes names;
/// dots and hyphens remain verbatim and are URL-safe.
pub(crate) fn clusters_from_kafkaconnect_list(
    list: &serde_json::Value,
) -> Result<Vec<ConnectCluster>> {
    let items = list
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut clusters = Vec::with_capacity(items.len());
    let mut seen_resources = HashSet::with_capacity(items.len());
    for item in items {
        let name = item
            .pointer("/metadata/name")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::KubernetesDiscover {
                reason: "KafkaConnect is missing metadata.name".to_owned(),
            })?;
        let namespace = item
            .pointer("/metadata/namespace")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("default");
        if name.len() > 253 || !is_valid_dns_name(name) {
            return Err(Error::KubernetesDiscover {
                reason: format!(
                    "KafkaConnect metadata.name '{name}' must be a DNS subdomain of at most 253 characters"
                ),
            });
        }
        if namespace.len() > 63 || namespace.contains('.') || !is_valid_dns_name(namespace) {
            return Err(Error::KubernetesDiscover {
                reason: format!(
                    "KafkaConnect {name} metadata.namespace '{namespace}' must be a DNS label of at most 63 characters"
                ),
            });
        }
        if !seen_resources.insert((namespace, name)) {
            return Err(Error::KubernetesDiscover {
                reason: format!("duplicate KafkaConnect resource {namespace}/{name}"),
            });
        }
        let bootstrap = item
            .pointer("/spec/bootstrapServers")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let annotated_url = item
            .pointer("/metadata/annotations/strimzi-ops.io~1connect-url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let connect_url = annotated_url
            .or_else(|| Some(format!("http://{name}-connect-api.{namespace}.svc:8083")));

        let id = format!("{namespace}_{name}");
        clusters.push(ConnectCluster {
            id,
            settings: ConnectionSettings {
                connect_url,
                bootstrap_servers: bootstrap,
                connect_cluster_name: Some(name.to_owned()),
                namespace: Some(namespace.to_owned()),
            },
        });
    }
    clusters.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(clusters)
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
    fn parses_legacy_kafka_section_as_one_cluster() {
        let clusters = clusters_from_toml(
            r#"
[kafka]
connect_url = "http://localhost:8083"
bootstrap_servers = "localhost:9092"
connect_cluster_name = "my-connect-cluster"
"#,
            Path::new("secrets.toml"),
        )
        .unwrap();

        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].id, "my-connect-cluster");
        assert_eq!(
            clusters[0].settings.connect_url.as_deref(),
            Some("http://localhost:8083")
        );
    }

    #[test]
    fn parses_named_clusters_array() {
        let clusters = clusters_from_toml(
            r#"
[[clusters]]
id = "local"
connect_url = "http://127.0.0.1:8083"
bootstrap_servers = "127.0.0.1:9092"
connect_cluster_name = "my-connect-cluster"

[[clusters]]
id = "prod"
connect_url = "https://connect.example"
bootstrap_servers = "kafka.example:9092"
connect_cluster_name = "prod-connect"
"#,
            Path::new("secrets.toml"),
        )
        .unwrap();

        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].id, "local");
        assert_eq!(clusters[1].id, "prod");
        assert_eq!(
            clusters[1].settings.connect_url.as_deref(),
            Some("https://connect.example")
        );
    }

    #[test]
    fn rejects_duplicate_cluster_ids() {
        let err = clusters_from_toml(
            r#"
[[clusters]]
id = "dup"
connect_url = "http://a:8083"
[[clusters]]
id = "dup"
connect_url = "http://b:8083"
"#,
            Path::new("secrets.toml"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate cluster id 'dup'"));
    }

    #[test]
    fn discovers_kafkaconnect_resources() {
        let list = serde_json::json!({
            "items": [
                {
                    "metadata": {
                        "name": "payments-connect",
                        "namespace": "payments",
                        "annotations": {
                            "strimzi-ops.io/connect-url": "https://connect.payments.example"
                        }
                    },
                    "spec": { "bootstrapServers": "payments-kafka:9092" }
                },
                {
                    "metadata": { "name": "my-connect-cluster", "namespace": "kafka" },
                    "spec": { "bootstrapServers": "my-cluster-kafka-bootstrap:9092" }
                }
            ]
        });
        let clusters = clusters_from_kafkaconnect_list(&list).unwrap();
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].id, "kafka_my-connect-cluster");
        assert_eq!(
            clusters[0].settings.connect_url.as_deref(),
            Some("http://my-connect-cluster-connect-api.kafka.svc:8083")
        );
        assert_eq!(clusters[1].id, "payments_payments-connect");
        assert_eq!(
            clusters[1].settings.connect_url.as_deref(),
            Some("https://connect.payments.example")
        );
        assert_eq!(clusters[1].settings.namespace.as_deref(), Some("payments"));
    }

    #[test]
    fn discovers_distinct_stable_ids_despite_colliding_name_prefixes() {
        let list = serde_json::json!({
            "items": [
                {
                    "metadata": { "name": "connect", "namespace": "a" },
                    "spec": { "bootstrapServers": "kafka-a:9092" }
                },
                {
                    "metadata": { "name": "connect", "namespace": "b" },
                    "spec": { "bootstrapServers": "kafka-b:9092" }
                },
                {
                    "metadata": { "name": "a-connect", "namespace": "a" },
                    "spec": { "bootstrapServers": "kafka-a:9092" }
                }
            ]
        });
        let clusters = clusters_from_kafkaconnect_list(&list).unwrap();
        let ids: HashSet<_> = clusters.iter().map(|cluster| cluster.id.as_str()).collect();
        assert_eq!(ids.len(), 3);

        let mut reversed = list;
        reversed["items"].as_array_mut().unwrap().reverse();
        let reversed_clusters = clusters_from_kafkaconnect_list(&reversed).unwrap();
        for (cluster, reversed_cluster) in clusters.iter().zip(&reversed_clusters) {
            assert_eq!(cluster.id, reversed_cluster.id);
            assert_eq!(
                cluster.settings.connect_cluster_name,
                reversed_cluster.settings.connect_cluster_name
            );
            assert_eq!(
                cluster.settings.namespace,
                reversed_cluster.settings.namespace
            );
        }
    }

    #[test]
    fn discovers_dotted_names_without_losing_resource_identity() {
        let list = serde_json::json!({
            "items": [
                { "metadata": { "name": "my.connect", "namespace": "kafka" } },
                { "metadata": { "name": "my-connect", "namespace": "kafka" } }
            ]
        });
        let clusters = clusters_from_kafkaconnect_list(&list).unwrap();
        assert_eq!(clusters.len(), 2);
        let dotted = clusters
            .iter()
            .find(|cluster| cluster.settings.cluster_name() == "my.connect")
            .unwrap();
        assert_eq!(dotted.id, "kafka_my.connect");
        assert_eq!(dotted.settings.namespace.as_deref(), Some("kafka"));
        assert_eq!(
            dotted.settings.connect_url.as_deref(),
            Some("http://my.connect-connect-api.kafka.svc:8083")
        );
        assert_ne!(clusters[0].id, clusters[1].id);
    }

    #[test]
    fn malformed_discovery_resources_report_the_invalid_metadata() {
        for (metadata, expected) in [
            (serde_json::json!({ "namespace": "a" }), "metadata.name"),
            (
                serde_json::json!({ "name": "bad/name", "namespace": "a" }),
                "metadata.name 'bad/name'",
            ),
            (
                serde_json::json!({ "name": "connect", "namespace": "bad_namespace" }),
                "metadata.namespace 'bad_namespace'",
            ),
        ] {
            let list = serde_json::json!({ "items": [{ "metadata": metadata }] });
            let err = clusters_from_kafkaconnect_list(&list).unwrap_err();
            assert!(matches!(&err, Error::KubernetesDiscover { .. }));
            assert!(err.to_string().contains(expected));
        }
    }

    #[test]
    fn unknown_cluster_id_errors() {
        let err = LoadConfig {
            cluster_id: Some("nope".to_owned()),
            ..LoadConfig::default()
        }
        .load_settings()
        .unwrap_err();
        assert!(matches!(err, Error::UnknownCluster { id } if id == "nope"));
    }
}
