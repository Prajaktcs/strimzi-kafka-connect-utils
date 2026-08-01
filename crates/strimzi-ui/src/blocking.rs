//! Helpers to run Connect (blocking reqwest) work off the Tokio runtime.

use strimzi_ops_core::ConnectClient;

use crate::error::Error;
use crate::result::Result;

/// Create a Connect client and run `op` on a blocking pool thread.
pub async fn with_connect_client<T, F>(connect_url: String, op: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&ConnectClient) -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let client = ConnectClient::new(&connect_url)?;
        op(&client)
    })
    .await
    .map_err(|err| Error::Internal {
        reason: err.to_string(),
    })?
}
