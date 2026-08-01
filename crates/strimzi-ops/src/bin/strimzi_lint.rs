use std::process::ExitCode;

use clap::Parser;
use strimzi_ops::{run_lint_only, LintCli};

fn main() -> ExitCode {
    match run_lint_only(LintCli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}
