use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] strimzi_ops_core::Error),

    #[error(transparent)]
    Smoke(#[from] strimzi_ops_core::smoke::Error),

    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("cannot serialise JSON output: {reason}")]
    JsonOutput { reason: String },
}
