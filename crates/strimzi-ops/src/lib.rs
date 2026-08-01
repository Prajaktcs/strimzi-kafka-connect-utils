//! Shared CLI helpers for `strimzi-ops` and `strimzi-lint`.

pub mod error;
pub mod result;
pub mod settings;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Map, Value};
use strimzi_ops_core::{
    parse_config_text, to_strimzi_yaml, validate_text, ConfigFormat, ConnectClient,
    CreateConnectorRequest, NotificationMonitor, SnapshotTracker, SnapshotTrigger,
    ValidationReport,
};

use crate::error::Error;
use crate::result::Result;
use crate::settings::load_settings;

#[derive(Debug, Parser)]
#[command(
    name = "strimzi-ops",
    about = "Strimzi Ops - Kafka Connect management tools",
    long_about = None
)]
pub struct Cli {
    /// Path to secrets.toml (default: ./secrets.toml when present)
    #[arg(long = "secrets", global = true)]
    pub secrets: Option<PathBuf>,

    /// Kafka Connect REST API URL
    #[arg(long = "connect-url", global = true)]
    pub connect_url: Option<String>,

    /// Kafka bootstrap servers
    #[arg(long = "bootstrap-servers", global = true)]
    pub bootstrap_servers: Option<String>,

    /// Strimzi `KafkaConnect` cluster name (for YAML export)
    #[arg(long = "cluster-name", global = true)]
    pub cluster_name: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Parser)]
#[command(
    name = "strimzi-lint",
    about = "Lint Kafka Connect connector configurations",
    long_about = None
)]
pub struct LintCli {
    #[command(subcommand)]
    pub command: LintOnlyCommand,
}

#[derive(Debug, Subcommand)]
pub enum LintOnlyCommand {
    /// Lint a connector configuration file
    Lint {
        file: PathBuf,
        #[arg(short = 'c', long = "config")]
        config: Option<PathBuf>,
        #[arg(short = 'f', long = "format", value_enum, default_value_t = FormatArg::Auto)]
        format: FormatArg,
        #[arg(long = "json")]
        json_output: bool,
        #[arg(long = "strict")]
        strict: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Lint a connector configuration file
    Lint {
        file: PathBuf,
        #[arg(short = 'c', long = "config")]
        config: Option<PathBuf>,
        #[arg(short = 'f', long = "format", value_enum, default_value_t = FormatArg::Auto)]
        format: FormatArg,
        #[arg(long = "json")]
        json_output: bool,
        #[arg(long = "strict")]
        strict: bool,
    },
    /// Manage Kafka Connect connectors
    Connectors {
        #[command(subcommand)]
        command: ConnectorCommands,
    },
    /// Kafka Connect cluster metadata
    Cluster {
        #[command(subcommand)]
        command: ClusterCommands,
    },
    /// Trigger Debezium snapshots
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommands,
    },
    /// Monitor Debezium notification topics
    Monitor {
        #[arg(long = "topic", default_value = "debezium.notifications")]
        topic: String,
        #[arg(long = "group-id", default_value = "strimzi-ops-monitor")]
        group_id: String,
        #[arg(long = "duration-secs")]
        duration_secs: Option<u64>,
        #[arg(long = "json")]
        json_output: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConnectorCommands {
    List,
    Status {
        name: Option<String>,
    },
    Info {
        name: String,
    },
    Config {
        name: String,
    },
    Pause {
        name: String,
    },
    Resume {
        name: String,
    },
    Restart {
        name: String,
    },
    RestartTask {
        name: String,
        task_id: u32,
    },
    Create {
        #[arg(long = "file", short = 'f')]
        file: PathBuf,
    },
    Update {
        name: String,
        #[arg(long = "file", short = 'f')]
        file: PathBuf,
    },
    Delete {
        name: String,
    },
    ExportYaml {
        name: String,
        #[arg(long = "cluster")]
        cluster: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ClusterCommands {
    Info,
    Plugins,
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommands {
    Trigger {
        name: String,
        #[arg(long = "type", default_value = "incremental")]
        snapshot_type: String,
        #[arg(long = "tables")]
        tables: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
pub enum FormatArg {
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

pub fn run_ops(cli: Cli) -> Result<ExitCode> {
    let Cli {
        secrets,
        connect_url,
        bootstrap_servers,
        cluster_name,
        command,
    } = cli;

    match command {
        // Lint does not need Connect/Kafka settings; skip secrets.toml so a broken
        // local secrets file cannot fail an unrelated lint run.
        Commands::Lint {
            file,
            config,
            format,
            json_output,
            strict,
        } => lint_command(&file, config.as_deref(), format, json_output, strict),
        Commands::Connectors { command } => {
            let settings = load_settings(
                secrets.as_deref(),
                connect_url,
                bootstrap_servers,
                cluster_name,
            )?;
            let client = ConnectClient::new(settings.require_connect_url()?)?;
            connectors_command(&client, command, &settings)
        }
        Commands::Cluster { command } => {
            let settings = load_settings(
                secrets.as_deref(),
                connect_url,
                bootstrap_servers,
                cluster_name,
            )?;
            let client = ConnectClient::new(settings.require_connect_url()?)?;
            cluster_command(&client, command)
        }
        Commands::Snapshot { command } => {
            let settings = load_settings(
                secrets.as_deref(),
                connect_url,
                bootstrap_servers,
                cluster_name,
            )?;
            let client = ConnectClient::new(settings.require_connect_url()?)?;
            snapshot_command(&client, command, &settings)
        }
        Commands::Monitor {
            topic,
            group_id,
            duration_secs,
            json_output,
        } => {
            let settings = load_settings(
                secrets.as_deref(),
                connect_url,
                bootstrap_servers,
                cluster_name,
            )?;
            monitor_command(&settings, &topic, &group_id, duration_secs, json_output)
        }
    }
}

pub fn run_lint_only(cli: LintCli) -> Result<ExitCode> {
    match cli.command {
        LintOnlyCommand::Lint {
            file,
            config,
            format,
            json_output,
            strict,
        } => lint_command(&file, config.as_deref(), format, json_output, strict),
    }
}

fn connectors_command(
    client: &ConnectClient,
    command: ConnectorCommands,
    settings: &settings::ConnectionSettings,
) -> Result<ExitCode> {
    match command {
        ConnectorCommands::List => {
            print_json(&client.list_connectors()?)?;
        }
        ConnectorCommands::Status { name: None } => {
            print_json(&client.get_all_connectors_status()?)?;
        }
        ConnectorCommands::Status { name: Some(name) } => {
            print_json(&client.get_connector_status(&name)?)?;
        }
        ConnectorCommands::Info { name } => {
            print_json(&client.get_connector_info(&name)?)?;
        }
        ConnectorCommands::Config { name } => {
            print_json(&client.get_connector_config(&name)?)?;
        }
        ConnectorCommands::Pause { name } => {
            client.pause_connector(&name)?;
            println!("Paused connector: {name}");
        }
        ConnectorCommands::Resume { name } => {
            client.resume_connector(&name)?;
            println!("Resumed connector: {name}");
        }
        ConnectorCommands::Restart { name } => {
            client.restart_connector(&name)?;
            println!("Restarted connector: {name}");
        }
        ConnectorCommands::RestartTask { name, task_id } => {
            client.restart_connector_task(&name, task_id)?;
            println!("Restarted task {task_id} for connector: {name}");
        }
        ConnectorCommands::Create { file } => {
            let request = load_create_request(&file)?;
            print_json(&client.create_connector(&request)?)?;
        }
        ConnectorCommands::Update { name, file } => {
            let config = load_config_map(&file)?;
            print_json(&client.update_connector(&name, &config)?)?;
        }
        ConnectorCommands::Delete { name } => {
            client.delete_connector(&name)?;
            println!("Deleted connector: {name}");
        }
        ConnectorCommands::ExportYaml { name, cluster } => {
            let config = client.get_connector_config(&name)?;
            let cluster_name = cluster
                .as_deref()
                .unwrap_or_else(|| settings.cluster_name());
            println!("{}", to_strimzi_yaml(&name, &config, cluster_name));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cluster_command(client: &ConnectClient, command: ClusterCommands) -> Result<ExitCode> {
    match command {
        ClusterCommands::Info => print_json(&client.get_cluster_info()?)?,
        ClusterCommands::Plugins => print_json(&client.get_connector_plugins()?)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn snapshot_command(
    client: &ConnectClient,
    command: SnapshotCommands,
    settings: &settings::ConnectionSettings,
) -> Result<ExitCode> {
    match command {
        SnapshotCommands::Trigger {
            name,
            snapshot_type,
            tables,
        } => {
            let tables: Option<Vec<String>> = tables.map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_owned)
                    .collect()
            });
            let trigger = SnapshotTrigger::new(client.clone(), settings.bootstrap_servers.clone());
            let result = trigger.trigger(&name, &snapshot_type, tables.as_deref())?;
            print_json(&result)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn monitor_command(
    settings: &settings::ConnectionSettings,
    topic: &str,
    group_id: &str,
    duration_secs: Option<u64>,
    json_output: bool,
) -> Result<ExitCode> {
    let bootstrap = settings.require_bootstrap_servers()?;
    let mut monitor = NotificationMonitor::new(bootstrap, topic);
    monitor.start(group_id)?;

    let mut tracker = SnapshotTracker::new();
    let duration = duration_secs.map(Duration::from_secs);

    monitor.consume_notifications(
        |notification| {
            if json_output {
                match serde_json::to_string_pretty(&notification) {
                    Ok(text) => println!("{text}"),
                    Err(err) => eprintln!("Error: cannot serialise notification: {err}"),
                }
            } else {
                tracker.process_notification(&notification);
                if let Some(connector) = notification.get("aggregateType").and_then(Value::as_str) {
                    if let Some(state) = tracker.get_snapshot_status(connector) {
                        println!("{connector}: {} ({}%)", state.status, state.progress);
                    }
                }
            }
        },
        duration,
    )?;

    Ok(ExitCode::SUCCESS)
}

pub fn lint_command(
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
        print_json(&output)?;
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

fn load_create_request(path: &Path) -> Result<CreateConnectorRequest> {
    let map = load_config_map(path)?;
    if let (Some(Value::String(name)), Some(Value::Object(config))) =
        (map.get("name"), map.get("config"))
    {
        return Ok(CreateConnectorRequest {
            name: name.clone(),
            config: config.clone(),
        });
    }
    if let Some(Value::String(name)) = map.get("name") {
        let mut config = map.clone();
        config.remove("name");
        return Ok(CreateConnectorRequest {
            name: name.clone(),
            config,
        });
    }
    Err(Error::JsonOutput {
        reason: "create payload must include name and config".to_owned(),
    })
}

fn load_config_map(path: &Path) -> Result<Map<String, Value>> {
    let text = fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut format = ConfigFormat::from_extension(path.extension().and_then(|ext| ext.to_str()));
    if matches!(format, ConfigFormat::Auto) {
        format = ConfigFormat::Auto;
    }
    let map = parse_config_text(&text, format)?;
    if let Some(Value::Object(config)) = map.get("config") {
        if map.contains_key("name") {
            return Ok(map);
        }
        return Ok(config.clone());
    }
    Ok(map)
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|err| Error::JsonOutput {
        reason: err.to_string(),
    })?;
    println!("{text}");
    Ok(())
}
