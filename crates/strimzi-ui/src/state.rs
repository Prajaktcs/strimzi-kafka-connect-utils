use std::sync::Arc;

use strimzi_ops_core::{ConnectClient, ConnectionSettings};

/// Shared application state for Axum handlers.
#[derive(Clone)]
pub struct AppState {
    pub settings: ConnectionSettings,
    pub client: Option<Arc<ConnectClient>>,
}

impl AppState {
    pub fn new(settings: ConnectionSettings) -> Self {
        let client = settings
            .connect_url
            .as_deref()
            .and_then(|url| ConnectClient::new(url).ok())
            .map(Arc::new);
        Self { settings, client }
    }

    pub fn require_client(&self) -> crate::result::Result<Arc<ConnectClient>> {
        self.client
            .clone()
            .ok_or(crate::error::Error::ConfigRequired)
    }

    pub fn cluster_name(&self) -> &str {
        self.settings.cluster_name()
    }
}
