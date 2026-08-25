use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use strimzi_ops_core::LoadConfig;
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

    /// Discover Connect clusters from `KafkaConnect` custom resources
    #[arg(long = "from-k8s")]
    from_k8s: bool,

    /// Namespace for `--from-k8s` (default: all namespaces)
    #[arg(long = "k8s-namespace")]
    k8s_namespace: Option<String>,

    /// Bind address
    #[arg(long = "bind", default_value = "127.0.0.1")]
    bind: String,

    /// Listen port (default 8501)
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
    let clusters = LoadConfig {
        secrets: cli.secrets,
        connect_url: cli.connect_url,
        bootstrap_servers: cli.bootstrap_servers,
        cluster_name: cli.cluster_name,
        from_k8s: cli.from_k8s,
        k8s_namespace: cli.k8s_namespace,
        ..LoadConfig::default()
    }
    .load_clusters()?;
    let state = AppState::new(clusters);

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

    axum::serve(listener, app)
        .await
        .map_err(|source| strimzi_ui::error::Error::Internal {
            reason: format!("server error: {source}"),
        })?;
    Ok(())
}
