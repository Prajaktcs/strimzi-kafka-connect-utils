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
}
