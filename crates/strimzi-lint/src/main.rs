//! `strimzi-lint` — lint Kafka Connect connector configurations.
//!
//! Follows Canonical's [Rust best practices](https://canonical.github.io/rust-best-practices/introduction.html).

mod error;
mod result;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;
use strimzi_ops_core::{validate_text, ConfigFormat, ValidationReport};

use crate::error::Error;
use crate::result::Result;

#[derive(Debug, Parser)]
#[command(
    name = "strimzi-lint",
    about = "Strimzi Ops - Kafka Connect management tools",
    long_about = None
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Lint a connector configuration file
    Lint {
        /// Path to the connector configuration file (YAML or JSON)
        file: PathBuf,

        /// Path to linter configuration file (default: .lintrc.toml)
        #[arg(short = 'c', long = "config")]
        config: Option<PathBuf>,

        /// Configuration file format
        #[arg(short = 'f', long = "format", value_enum, default_value_t = FormatArg::Auto)]
        format: FormatArg,

        /// Output results in JSON format
        #[arg(long = "json")]
        json_output: bool,

        /// Treat warnings as errors (exit code 1)
        #[arg(long = "strict")]
        strict: bool,
    },
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum FormatArg {
    #[default]
    Auto,
    Yaml,
    Json,
}

impl From<FormatArg> for ConfigFormat {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Auto => Self::Auto,
            FormatArg::Yaml => Self::Yaml,
            FormatArg::Json => Self::Json,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Lint {
            file,
            config,
            format,
            json_output,
            strict,
        } => lint_command(&file, config.as_deref(), format, json_output, strict),
    }
}

fn lint_command(
    file: &Path,
    config: Option<&Path>,
    format: FormatArg,
    json_output: bool,
    strict: bool,
) -> Result<ExitCode> {
    let text = fs::read_to_string(file).map_err(|source| Error::Read {
        path: file.to_path_buf(),
        source,
    })?;

    let mut format = ConfigFormat::from(format);
    if matches!(format, ConfigFormat::Auto) {
        format = ConfigFormat::from_extension(file.extension().and_then(|ext| ext.to_str()));
    }

    let report = validate_text(&text, format, None, config)?;
    emit_report(&report, json_output)?;

    if !report.valid || (strict && report.summary.warnings > 0) {
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

fn emit_report(report: &ValidationReport, json_output: bool) -> Result<()> {
    if json_output {
        let output = json!({
            "valid": report.valid,
            "summary": {
                "errors": report.summary.errors,
                "warnings": report.summary.warnings,
                "info": report.summary.info,
            },
            "results": report.results.iter().map(|r| json!({
                "rule_id": r.rule_id,
                "severity": r.severity.as_str(),
                "message": r.message,
                "path": r.path,
            })).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&output).map_err(|err| Error::JsonOutput {
            reason: err.to_string(),
        })?;
        println!("{text}");
        return Ok(());
    }

    if report.results.is_empty() {
        println!("✅ No issues found");
    } else {
        println!("{}", report.formatted);
    }
    println!();
    println!(
        "Summary: {} errors, {} warnings, {} info",
        report.summary.errors, report.summary.warnings, report.summary.info
    );
    Ok(())
}
