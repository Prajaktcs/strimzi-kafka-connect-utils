use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] strimzi_ops_core::Error),

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("cannot parse JSON: {reason}")]
    Json { reason: String },

    #[error("configuration is required; set --connect-url or kafka.connect_url in secrets.toml")]
    ConfigRequired,

    #[error("internal error: {reason}")]
    Internal { reason: String },
}
