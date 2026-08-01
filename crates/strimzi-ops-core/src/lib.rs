//! Connector linting and validation for Strimzi Ops.

pub mod linter;
pub mod parse;
pub mod schema;
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
}

pub type Result<T> = std::result::Result<T, Error>;

pub use linter::{ConnectorLinter, LintResult, LinterConfig, Severity, Summary};
pub use parse::{parse_config_text, ConfigFormat};
pub use validate::{validate_config, validate_text, ValidationReport};
