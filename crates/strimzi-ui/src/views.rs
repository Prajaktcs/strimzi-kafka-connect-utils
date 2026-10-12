use askama::Template;
use askama_web::WebTemplate;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use strimzi_ops_core::metrics::{
    ConnectorThroughput, MetricSample, MetricState, ThroughputStage, ThroughputValue,
};

use crate::error::Error;

pub type HtmlResult = std::result::Result<Response, Error>;

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::ConfigRequired => StatusCode::SERVICE_UNAVAILABLE,
            Self::Json { .. } => StatusCode::BAD_REQUEST,
            Self::Core(strimzi_ops_core::Error::ConnectHttp { .. }) => StatusCode::BAD_GATEWAY,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = format!(
            "<html><body style=\"font-family: sans-serif; max-width: 48rem; margin: 2rem auto; padding: 0 1rem;\">\
             <h1>Error</h1><pre style=\"white-space: pre-wrap;\">{self}</pre>\
             <p><a href=\"/dashboard\">Back to Dashboard</a></p>\
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

#[derive(Template, WebTemplate)]
#[template(path = "missing_config.html")]
pub struct MissingConfigPage {
    pub active: &'static str,
}

#[derive(Template, WebTemplate)]
#[template(path = "missing_bootstrap.html")]
pub struct MissingBootstrapPage {
    pub active: &'static str,
}

#[derive(Template, WebTemplate)]
#[template(path = "monitor.html")]
pub struct MonitorPage {
    pub active: &'static str,
    pub topic: String,
    pub duration: u64,
    pub error: Option<String>,
    pub has_bootstrap: bool,
}

#[derive(Debug)]
pub struct SnapshotCard<'a> {
    pub name: &'a str,
    pub status: &'a str,
    pub progress: u64,
}

#[derive(Template, WebTemplate)]
#[template(path = "monitor_results.html")]
pub struct MonitorResultsPage {
    pub active: &'static str,
    pub topic: String,
    pub duration: u64,
}

#[derive(Template)]
#[template(path = "snapshot_cards.html")]
pub struct SnapshotCards<'a> {
    pub snapshots: Vec<SnapshotCard<'a>>,
}

pub struct MetricsWorkerView<'a> {
    pub worker: &'a str,
    pub state: MetricState,
    pub age: String,
    pub error: Option<&'a str>,
    pub samples: Vec<&'a MetricSample>,
    pub omitted: usize,
}

pub struct ThroughputStageView {
    pub label: &'static str,
    pub rate: String,
    pub total: String,
}

impl ThroughputStageView {
    fn new(mut stage: ThroughputStage, expired: bool) -> Self {
        if expired {
            stage.rate.expire();
            stage.total.expire();
        }
        Self {
            label: stage.label,
            rate: format_measurement(stage.rate, 2),
            total: format_measurement(stage.total, 0),
        }
    }
}

fn format_measurement(measurement: ThroughputValue, precision: usize) -> String {
    measurement.value.map_or_else(
        || measurement.state.to_string(),
        |value| {
            // Never round a small but positive rate into apparent inactivity.
            if precision == 2 && value > 0.0 && value < 0.01 {
                format!("{value:.2e} ({})", measurement.state)
            } else {
                format!("{value:.precision$} ({})", measurement.state)
            }
        },
    )
}

pub struct ThroughputTaskView {
    pub task: String,
    pub worker: String,
    pub stages: Vec<ThroughputStageView>,
}

pub struct ThroughputConnectorView {
    pub connector: String,
    pub stages: Vec<ThroughputStageView>,
    pub tasks: Vec<ThroughputTaskView>,
}

impl ThroughputConnectorView {
    pub fn new(connector: ConnectorThroughput, expired: bool) -> Self {
        Self {
            connector: connector.connector,
            stages: connector
                .stages
                .into_iter()
                .map(|stage| ThroughputStageView::new(stage, expired))
                .collect(),
            tasks: connector
                .tasks
                .into_iter()
                .map(|task| ThroughputTaskView {
                    task: task.task,
                    worker: task.worker,
                    stages: task
                        .stages
                        .into_iter()
                        .map(|stage| ThroughputStageView::new(stage, expired))
                        .collect(),
                })
                .collect(),
        }
    }
}

#[derive(Template)]
#[template(path = "metrics.html")]
pub struct MetricsPanel<'a> {
    pub state: MetricState,
    pub workers: Vec<MetricsWorkerView<'a>>,
    pub throughput: Vec<ThroughputConnectorView>,
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
    pub active: &'static str,
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
    pub is_source: bool,
}

#[derive(Template, WebTemplate)]
#[template(path = "control.html")]
pub struct ControlPage {
    pub active: &'static str,
    pub connectors: Vec<ControlRow>,
    pub flash: Option<String>,
    pub focus: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "snapshot.html")]
pub struct SnapshotPage {
    pub active: &'static str,
    pub name: String,
    pub flash: Option<String>,
}

#[derive(Template, WebTemplate)]
#[template(path = "yaml.html")]
pub struct YamlPage {
    pub active: &'static str,
    pub name: String,
    pub yaml: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "logs.html")]
pub struct LogsPage {
    pub active: &'static str,
    pub name: String,
    pub log_text: String,
}

#[derive(Template, WebTemplate)]
#[template(path = "edit.html")]
pub struct EditPage {
    pub active: &'static str,
    pub name: String,
    pub config_json: String,
    pub validation_error: Option<String>,
    pub flash: Option<String>,
}
