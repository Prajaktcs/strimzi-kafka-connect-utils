//! Explicitly invoked, isolated local-stack end-to-end verification.

mod pipeline;
mod process;

pub use pipeline::{run_pipeline, PipelineOptions, PipelineReport};
pub use process::Error;
