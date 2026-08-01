//! Kafka Connect REST client.

pub mod client;
pub mod types;

pub use client::ConnectClient;
pub use types::{ClusterInfo, ConnectorPlugin, CreateConnectorRequest};
