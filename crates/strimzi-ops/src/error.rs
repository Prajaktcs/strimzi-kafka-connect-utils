use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] strimzi_ops_core::Error),

    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },

    #[error("cannot serialise JSON output: {reason}")]
    JsonOutput { reason: String },

    #[error("cannot load secrets from {path}: {reason}")]
    Secrets { path: PathBuf, reason: String },

    #[error("missing required option: {option}")]
    MissingOption { option: String },
}
