use strimzi_ops_core::ConnectionSettings;

use crate::error::Error;
use crate::result::Result;

/// Shared application state for Axum handlers.
///
/// Does not hold a `ConnectClient`: the blocking reqwest client must be created
/// and dropped only on worker threads (`spawn_blocking`), never on the Tokio runtime.
#[derive(Clone)]
pub struct AppState {
    pub settings: ConnectionSettings,
}

impl AppState {
    pub fn new(settings: ConnectionSettings) -> Self {
        Self { settings }
    }

    pub fn has_connect_url(&self) -> bool {
        self.settings.connect_url.is_some()
    }

    pub fn require_connect_url(&self) -> Result<String> {
        self.settings
            .require_connect_url()
            .map(str::to_owned)
            .map_err(Error::from)
    }

    pub fn cluster_name(&self) -> &str {
        self.settings.cluster_name()
    }

    pub fn bootstrap_servers(&self) -> Option<String> {
        self.settings.bootstrap_servers.clone()
    }
}
