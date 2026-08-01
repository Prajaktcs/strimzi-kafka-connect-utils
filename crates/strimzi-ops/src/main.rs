use std::process::ExitCode;

use clap::Parser;
use strimzi_ops::{run_ops, Cli};

fn main() -> ExitCode {
    match run_ops(Cli::parse()) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}
