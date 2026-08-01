//! CLI error type (Canonical: binary crates define `Error` in `error.rs`).

use std::io;
use std::path::PathBuf;

/// Application error for the `strimzi-lint` binary.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error originating from the core library.
    #[error(transparent)]
    Core(#[from] strimzi_ops_core::Error),

    /// Failed to read the connector config file.
    #[error("cannot read {path}: {source}")]
    Read {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// Failed to serialise JSON output.
    #[error("cannot serialise JSON output: {reason}")]
    JsonOutput {
        /// Serialisation failure reason.
        reason: String,
    },
}
