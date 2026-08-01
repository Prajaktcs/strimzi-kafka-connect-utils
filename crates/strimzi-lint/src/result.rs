//! CLI result alias (Canonical: binary crates define `Result` in `result.rs`).

use crate::error::Error;

/// Application result for the `strimzi-lint` binary.
pub type Result<T> = std::result::Result<T, Error>;
