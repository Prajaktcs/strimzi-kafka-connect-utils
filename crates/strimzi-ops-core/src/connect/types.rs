use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Kafka Connect cluster root response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ClusterInfo {
    pub version: Option<String>,
    pub commit: Option<String>,
    #[serde(default, rename = "kafka_cluster_id")]
    pub kafka_cluster_id: Option<String>,
}

/// Connector plugin metadata from `/connector-plugins`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConnectorPlugin {
    pub class: String,
    #[serde(rename = "type")]
    pub plugin_type: Option<String>,
    pub version: Option<String>,
}

/// Request body for `POST /connectors`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateConnectorRequest {
    pub name: String,
    pub config: Map<String, Value>,
}

/// Free-form connector configuration map.
pub type ConnectorConfig = Map<String, Value>;
