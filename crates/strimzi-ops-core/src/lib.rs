//! Core library for Strimzi Ops connector linting and validation.
//!
//! This crate follows Canonical's [Rust best practices](https://canonical.github.io/rust-best-practices/introduction.html).

pub mod linter;
pub mod parse;
pub mod schema;
pub mod validate;

use std::io;
use std::path::PathBuf;

/// Crate-local error type.
///
/// Messages follow Canonical guidance: concise, lowercase (unless acronyms),
/// prefer a leading verb phrased as `cannot …`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Failed to read a file from disk.
    #[error("cannot read {path}: {source}")]
    Read {
        /// Path that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// Failed to parse connector configuration text.
    #[error("cannot parse configuration as {format}: {reason}")]
    Parse {
        /// Format that was attempted (`json`, `yaml`, or `auto`).
        format: &'static str,
        /// Human-readable parse failure reason.
        reason: String,
    },

    /// Failed to load `.lintrc.toml` (or equivalent).
    #[error("cannot load linter config from {path}: {reason}")]
    LinterConfig {
        /// Path to the linter config file.
        path: PathBuf,
        /// Human-readable failure reason.
        reason: String,
    },

    /// Caller requested an unsupported configuration format.
    #[error("unknown configuration format '{format}'")]
    UnknownFormat {
        /// The unsupported format string.
        format: String,
    },
}

/// Crate-local result alias.
pub type Result<T> = std::result::Result<T, Error>;

pub use linter::{ConnectorLinter, LintResult, LinterConfig, Severity, Summary};
pub use parse::{parse_config_text, ConfigFormat};
pub use validate::{validate_config, validate_text, ValidationReport};
