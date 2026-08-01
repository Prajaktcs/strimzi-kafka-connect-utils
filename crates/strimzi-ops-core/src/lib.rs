//! Connector linting, Connect REST client, control, and monitoring for Strimzi Ops.

pub mod connect;
pub mod control;
pub mod k8s;
pub mod linter;
pub mod monitor;
pub mod parse;
pub mod schema;
pub mod settings;
pub mod validate;

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },

    #[error("cannot parse configuration as {format}: {reason}")]
    Parse {
        format: &'static str,
        reason: String,
    },

    #[error("cannot load linter config from {path}: {reason}")]
    LinterConfig { path: PathBuf, reason: String },

    #[error("unknown configuration format '{format}'")]
    UnknownFormat { format: String },

    #[error("cannot parse Connect URL '{url}': {reason}")]
    InvalidConnectUrl { url: String, reason: String },

    #[error("cannot {method} Connect API '{path}': {reason}")]
    ConnectHttp {
        method: String,
        path: String,
        reason: String,
    },

    #[error("bootstrap servers not configured; cannot send Kafka signals")]
    BootstrapServersRequired,

    #[error("cannot talk to Kafka: {reason}")]
    Kafka { reason: String },

    #[error("Kafka support is disabled; rebuild with the `kafka` feature")]
    KafkaFeatureDisabled,

    #[error("monitor not started; call start() first")]
    MonitorNotStarted,

    #[error("cannot load secrets from {path}: {reason}")]
    Secrets { path: PathBuf, reason: String },

    #[error("missing connection setting: {option}")]
    MissingSetting { option: String },

    #[error("kubectl error: {reason}")]
    Kubectl { reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

pub use connect::{ClusterInfo, ConnectClient, ConnectorPlugin, CreateConnectorRequest};
pub use control::{to_strimzi_yaml, SnapshotResult, SnapshotTrigger};
pub use k8s::{connect_label_selector, fetch_logs, filter_log_lines};
pub use linter::{ConnectorLinter, LintResult, LinterConfig, Severity, Summary};
pub use monitor::{NotificationMonitor, SnapshotState, SnapshotTracker};
pub use parse::{parse_config_text, ConfigFormat};
pub use settings::{load_settings, ConnectionSettings};
pub use validate::{validate_config, validate_text, ValidationReport};
