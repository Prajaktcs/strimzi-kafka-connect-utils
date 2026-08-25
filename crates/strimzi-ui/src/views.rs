use askama::Template;
use askama_web::WebTemplate;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};

use crate::error::Error;

pub type HtmlResult = std::result::Result<Response, Error>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::ConfigRequired => StatusCode::SERVICE_UNAVAILABLE,
            Self::UnknownCluster { .. } => StatusCode::NOT_FOUND,
            Self::Json { .. } => StatusCode::BAD_REQUEST,
            Self::Core(strimzi_ops_core::Error::ConnectHttp { .. }) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = format!(
            "<html><body style=\"font-family: sans-serif; max-width: 48rem; margin: 2rem auto; padding: 0 1rem;\">\
             <h1>Error</h1><pre style=\"white-space: pre-wrap;\">{self}</pre>\
             <p><a href=\"/\">Back to Dashboard</a></p>\
             <p>If Connect is down locally, start port-forwards with <code>just port-forward-all</code> \
             (or <code>just setup</code> for a full local stack), then refresh.</p>\
             </body></html>"
        );
        (status, Html(body)).into_response()
    }
}

pub fn redirect(path: &str) -> Response {
    Redirect::to(path).into_response()
}

pub fn render<T: Template>(template: T) -> HtmlResult {
    let body = template.render().map_err(|err| Error::Internal {
        reason: err.to_string(),
    })?;
    Ok(Html(body).into_response())
}

#[derive(Debug, Clone)]
pub struct ClusterChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct NavContext {
    pub active: &'static str,
    pub cluster_id: String,
    pub clusters: Vec<ClusterChoice>,
}

#[derive(Template, WebTemplate)]
#[template(path = "missing_config.html")]
pub struct MissingConfigPage {
    pub nav: NavContext,
}

#[derive(Template, WebTemplate)]
#[template(path = "missing_bootstrap.html")]
pub struct MissingBootstrapPage {
    pub nav: NavContext,
}

#[derive(Template, WebTemplate)]
#[template(path = "monitor.html")]
pub struct MonitorPage {
    pub nav: NavContext,
    pub topic: String,
    pub duration: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SnapshotCard {
    pub name: String,
    pub status: String,
    pub progress: u64,
}

#[derive(Template, WebTemplate)]
#[template(path = "monitor_results.html")]
pub struct MonitorResultsPage {
    pub nav: NavContext,
    pub topic: String,
    pub duration: u64,
    pub snapshots: Vec<SnapshotCard>,
}

#[derive(Debug, Clone)]
pub struct DashboardMetrics {
    pub total_connectors: usize,
    pub running_connectors: usize,
    pub failed_connectors: usize,
    pub running_tasks: usize,
    pub total_tasks: usize,
}

#[derive(Debug, Clone)]
pub struct StatusCount {
    pub state: String,
    pub count: usize,
}

#[derive(Debug, Clone)]
pub struct FailedItem {
    pub name: String,
    pub kind: String,
    pub error: String,
}

#[derive(Debug, Clone)]
pub struct ConnectorSummary {
    pub name: String,
    pub connector_type: String,
    pub state: String,
    pub tasks_running: usize,
    pub tasks_total: usize,
}

#[derive(Template, WebTemplate)]
#[template(path = "dashboard.html")]
pub struct DashboardPage {
    pub nav: NavContext,
    pub version: String,
    pub kafka_cluster_id: String,
    pub plugins: usize,
    pub metrics: DashboardMetrics,
    pub status_distribution: Vec<StatusCount>,
    pub failures: Vec<FailedItem>,
    pub connectors: Vec<ConnectorSummary>,
    pub flash: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ControlRow {
    pub name: String,
    pub state: String,
    pub tasks_running: usize,
    pub tasks_total: usize,
    pub focused: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "control.html")]
pub struct ControlPage {
    pub nav: NavContext,
    pub connectors: Vec<ControlRow>,
    pub flash: Option<String>,
    pub focus: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "snapshot.html")]
pub struct SnapshotPage {
    pub nav: NavContext,
    pub name: String,
    pub flash: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "yaml.html")]
pub struct YamlPage {
    pub nav: NavContext,
    pub name: String,
    pub yaml: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "logs.html")]
pub struct LogsPage {
    pub nav: NavContext,
    pub name: String,
    pub log_text: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "edit.html")]
pub struct EditPage {
    pub nav: NavContext,
    pub name: String,
    pub config_json: String,
    pub validation_error: Option<String>,
    pub flash: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "create.html")]
pub struct CreatePage {
    pub nav: NavContext,
    pub config_json: String,
    pub flash: Option<String>,
    pub error: Option<String>,
}
