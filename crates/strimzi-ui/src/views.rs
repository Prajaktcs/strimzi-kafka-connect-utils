use askama::Template;
use askama_web::WebTemplate;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use strimzi_ops_core::metrics::{FailureCounter, MetricSample, MetricState};

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

#[derive(Template)]
#[template(path = "metrics.html")]
pub struct MetricsPanel<'a> {
    pub state: MetricState,
    pub workers: Vec<MetricsWorkerView<'a>>,
}

#[derive(Debug, Clone)]
pub struct ProcessingErrorRow {
    pub worker: String,
    pub connector: String,
    pub task: String,
    pub state: MetricState,
    pub counter_reset: bool,
    pub record_errors: FailureCounter,
    pub record_failures: FailureCounter,
    pub retries: FailureCounter,
    pub records_skipped: FailureCounter,
    pub errors_logged: FailureCounter,
    pub dlq_writes: FailureCounter,
    pub dlq_failures: FailureCounter,
    /// `None` means REST config state unknown; `Some(false)` means the
    /// connector sets no dead-letter topic, so zeros are not DLQ activity.
    pub dlq_configured: Option<bool>,
    pub last_error_epoch: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessingErrorsPanel {
    /// `None` when no metrics targets file is configured at all.
    pub state: Option<MetricState>,
    pub rows: Vec<ProcessingErrorRow>,
}

impl ProcessingErrorRow {
    pub fn dlq_enabled(&self) -> bool {
        self.dlq_configured == Some(true)
    }

    pub fn dlq_status(&self) -> &'static str {
        match self.dlq_configured {
            Some(false) => "not configured / unsupported",
            Some(true) if self.dlq_writes.count.is_none() && self.dlq_failures.count.is_none() => {
                "unavailable / unsupported"
            }
            Some(true) => "configured",
            None => "configuration unavailable",
        }
    }

    pub fn last_error(&self) -> String {
        match self.last_error_epoch {
            Some(epoch) if epoch > 0.0 => epoch.to_string(),
            Some(_) => "never".to_owned(),
            None => "unavailable".to_owned(),
        }
    }
}

impl ProcessingErrorsPanel {
    pub fn is_collection_error(&self) -> bool {
        self.state == Some(MetricState::CollectionError)
    }

    pub fn is_unavailable(&self) -> bool {
        self.state == Some(MetricState::Unavailable)
    }

    /// True when metrics are collected but no error series are exported.
    pub fn has_exported_state(&self) -> bool {
        self.state.is_some_and(|state| {
            state != MetricState::Unavailable && state != MetricState::CollectionError
        })
    }
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
    pub processing_errors: ProcessingErrorsPanel,
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
