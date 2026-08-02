//! Helpers to run blocking work off the Tokio runtime.

use strimzi_ops_core::ConnectClient;

use crate::error::Error;
use crate::result::Result;

/// Create a Connect client and run `op` on a blocking pool thread.
pub async fn with_connect_client<T, F>(connect_url: String, op: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(&ConnectClient) -> Result<T> + Send + 'static,
{
    spawn_blocking(move || {
        let client = ConnectClient::new(&connect_url)?;
        op(&client)
    })
    .await
}

/// Run arbitrary blocking work on Tokio's blocking pool.
pub async fn spawn_blocking<T, F>(op: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })?
}
