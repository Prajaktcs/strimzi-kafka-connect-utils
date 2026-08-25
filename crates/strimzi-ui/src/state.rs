use strimzi_ops_core::ConnectCluster;

use crate::error::Error;
use crate::result::Result;
use crate::views::{ClusterChoice, NavContext};

/// Shared application state for Axum handlers.
///
/// Does not hold a `ConnectClient`: the blocking reqwest client must be created
/// and dropped only on worker threads (`spawn_blocking`), never on the Tokio runtime.
#[derive(Clone)]
pub struct AppState {
    pub clusters: Vec<ConnectCluster>,
}

impl AppState {
    pub fn new(clusters: Vec<ConnectCluster>) -> Self {
        Self { clusters }
    }

    pub fn from_single(settings: strimzi_ops_core::ConnectionSettings) -> Self {
        let id = settings
            .connect_cluster_name
            .as_deref()
            .filter(|name| !name.is_empty())
            .unwrap_or("default")
            .to_owned();
        Self::new(vec![ConnectCluster { id, settings }])
    }

    pub fn default_cluster_id(&self) -> &str {
        self.clusters
            .first()
            .map_or("default", |cluster| cluster.id.as_str())
    }

    pub fn cluster(&self, id: &str) -> Result<&ConnectCluster> {
        self.clusters
            .iter()
            .find(|cluster| cluster.id == id)
            .ok_or_else(|| Error::UnknownCluster { id: id.to_owned() })
    }

    pub fn nav(&self, active: &'static str, cluster_id: &str) -> NavContext {
        NavContext {
            active,
            cluster_id: cluster_id.to_owned(),
            clusters: self
                .clusters
                .iter()
                .map(|cluster| {
                    let label = match cluster.settings.namespace.as_deref() {
                        Some(namespace) => {
                            format!("{namespace}/{}", cluster.settings.cluster_name())
                        }
                        None => cluster.id.clone(),
                    };
                    ClusterChoice {
                        id: cluster.id.clone(),
                        label,
                    }
                })
                .collect(),
        }
    }
}
