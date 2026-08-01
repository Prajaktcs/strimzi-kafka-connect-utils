use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use strimzi_ops_core::load_settings;
use strimzi_ui::AppState;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

#[derive(Debug, Parser)]
#[command(
    name = "strimzi-ui",
    about = "Strimzi Ops web UI (Dashboard and Control)"
)]
struct Cli {
    /// Path to secrets.toml (default: ./secrets.toml when present)
    #[arg(long = "secrets")]
    secrets: Option<PathBuf>,

    /// Kafka Connect REST API URL
    #[arg(long = "connect-url")]
    connect_url: Option<String>,

    /// Kafka bootstrap servers
    #[arg(long = "bootstrap-servers")]
    bootstrap_servers: Option<String>,

    /// Strimzi `KafkaConnect` cluster name
    #[arg(long = "cluster-name")]
    cluster_name: Option<String>,

    /// Bind address
    #[arg(long = "bind", default_value = "127.0.0.1")]
    bind: String,

    /// Listen port (default 8501; conflicts with Streamlit if both run)
    #[arg(long = "port", default_value_t = 8501)]
    port: u16,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> strimzi_ui::result::Result<()> {
    let cli = Cli::parse();
    let settings = load_settings(
        cli.secrets.as_deref(),
        cli.connect_url,
        cli.bootstrap_servers,
        cli.cluster_name,
    )?;
    let state = AppState::new(settings);

    let static_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
    let app = strimzi_ui::router(state)
        .nest_service("/static", ServeDir::new(static_dir))
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = format!("{}:{}", cli.bind, cli.port)
        .parse()
        .map_err(|err| strimzi_ui::error::Error::Internal {
            reason: format!("invalid bind address: {err}"),
        })?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| strimzi_ui::error::Error::Internal {
            reason: format!("cannot bind {addr}: {source}"),
        })?;

    println!("strimzi-ui listening on http://{addr}");
    println!("Note: Streamlit (`just run`) also defaults to :8501 — only one can bind that port.");

    axum::serve(listener, app)
        .await
        .map_err(|source| strimzi_ui::error::Error::Internal {
            reason: format!("server error: {source}"),
        })?;
    Ok(())
}
