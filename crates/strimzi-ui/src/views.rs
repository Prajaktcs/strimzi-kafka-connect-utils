use askama::Template;
use askama_web::WebTemplate;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use strimzi_ops_core::metrics::{
    ConnectBackpressure, ConnectorThroughput, DebeziumQueue, DebeziumStreaming, FailureCounter,
    MetricLookup, MetricSample, MetricState, ThroughputStage, ThroughputValue,
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
    fn new(connector: ConnectorThroughput, expired: bool, remaining: &mut usize) -> Self {
        let stages = bounded_stages(connector.stages, expired, true, remaining);
        let tasks = connector
            .tasks
            .into_iter()
            .take(*remaining)
            .filter_map(|task| {
                let stages = bounded_stages(task.stages, expired, false, remaining);
                (!stages.is_empty()).then_some(ThroughputTaskView {
                    task: task.task,
                    worker: task.worker,
                    stages,
                })
            })
            .collect();
        Self {
            connector: connector.connector,
            stages,
            tasks,
        }
    }
}

fn bounded_stages(
    stages: Vec<ThroughputStage>,
    expired: bool,
    aggregate: bool,
    remaining: &mut usize,
) -> Vec<ThroughputStageView> {
    let stages: Vec<_> = stages
        .into_iter()
        .take(*remaining)
        .map(|mut stage| {
            if expired && aggregate {
                stage.rate.expire();
                stage.total.expire();
                stage.rate.value = None;
                stage.total.value = None;
            }
            ThroughputStageView::new(stage, expired)
        })
        .collect();
    *remaining -= stages.len();
    stages
}

/// Bound formatted connector/task rows, never the underlying aggregation.
pub fn throughput_views(
    connectors: Vec<ConnectorThroughput>,
    expired: bool,
    row_limit: usize,
) -> (Vec<ThroughputConnectorView>, usize) {
    let mut remaining = row_limit;
    let mut omitted = 0;
    let mut views = Vec::new();
    for connector in connectors {
        let rows = connector.stages.len()
            + connector
                .tasks
                .iter()
                .map(|task| task.stages.len())
                .sum::<usize>();
        let before = remaining;
        if remaining > 0 {
            views.push(ThroughputConnectorView::new(
                connector,
                expired,
                &mut remaining,
            ));
        }
        omitted += rows - (before - remaining);
    }
    (views, omitted)
}

pub struct DebeziumStreamingView {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub domain: String,
    pub server: String,
    pub connected: String,
    pub delay_ms: String,
    pub events_total: String,
    pub creates_total: String,
    pub updates_total: String,
    pub deletes_total: String,
}

impl DebeziumStreamingView {
    fn new(streaming: DebeziumStreaming, expired: bool) -> Self {
        Self {
            connector: streaming.connector,
            task: streaming.task,
            worker: streaming.worker,
            domain: streaming.domain,
            server: streaming.server,
            connected: format_connection(streaming.connected, expired),
            delay_ms: format_streaming_measurement(streaming.delay_ms, expired, 2),
            events_total: format_streaming_measurement(streaming.events_total, expired, 0),
            creates_total: format_streaming_measurement(streaming.creates_total, expired, 0),
            updates_total: format_streaming_measurement(streaming.updates_total, expired, 0),
            deletes_total: format_streaming_measurement(streaming.deletes_total, expired, 0),
        }
    }
}

fn expire_lookup(mut measurement: MetricLookup, expired: bool) -> MetricLookup {
    if expired && measurement.state == MetricState::Available {
        measurement.state = MetricState::Stale;
    }
    measurement
}

fn format_connection(measurement: MetricLookup, expired: bool) -> String {
    let measurement = expire_lookup(measurement, expired);
    let observed = match measurement.value {
        Some(1.0) => "connected",
        Some(0.0) => "disconnected",
        _ => return measurement.state.to_string(),
    };
    match measurement.state {
        MetricState::Available => observed.to_owned(),
        MetricState::Stale => format!("last observed: {observed} (stale)"),
        state => state.to_string(),
    }
}

fn format_streaming_measurement(
    measurement: MetricLookup,
    expired: bool,
    precision: usize,
) -> String {
    let measurement = expire_lookup(measurement, expired);
    match (measurement.state, measurement.value) {
        (state @ (MetricState::Available | MetricState::Stale), Some(value)) => {
            let status = if state == MetricState::Stale {
                "last observed; stale"
            } else {
                "available"
            };
            if precision == 2 && value > 0.0 && value < 0.01 {
                format!("{value:.2e} ({status})")
            } else {
                format!("{value:.precision$} ({status})")
            }
        }
        (state, _) => state.to_string(),
    }
}

/// Bound identity rows only after projecting the complete worker snapshot.
pub fn streaming_views(
    streaming: Vec<DebeziumStreaming>,
    expired: bool,
    row_limit: usize,
) -> (Vec<DebeziumStreamingView>, usize) {
    let omitted = streaming.len().saturating_sub(row_limit);
    let views = streaming
        .into_iter()
        .take(row_limit)
        .map(|row| DebeziumStreamingView::new(row, expired))
        .collect();
    (views, omitted)
}

pub struct QueueBackpressureView {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub domain: String,
    pub server: String,
    pub context: String,
    pub total_capacity: String,
    pub remaining_capacity: String,
    pub utilization: String,
    pub max_queue_bytes: String,
    pub current_queue_bytes: String,
    pub bytes_utilization: String,
}

impl QueueBackpressureView {
    fn new(queue: DebeziumQueue, expired: bool) -> Self {
        Self {
            connector: queue.connector,
            task: queue.task,
            worker: queue.worker,
            domain: queue.domain,
            server: queue.server,
            context: queue.context,
            total_capacity: format_queue_measure(queue.total_capacity, expired, 0),
            remaining_capacity: format_queue_measure(queue.remaining_capacity, expired, 0),
            utilization: format_queue_measure(queue.utilization, expired, 2),
            max_queue_bytes: format_queue_measure(queue.max_queue_bytes, expired, 0),
            current_queue_bytes: format_queue_measure(queue.current_queue_bytes, expired, 0),
            bytes_utilization: format_queue_measure(queue.bytes_utilization, expired, 2),
        }
    }
}

pub struct ConnectBackpressureView {
    pub connector: String,
    pub task: String,
    pub worker: String,
    pub kind: &'static str,
    pub active_records: String,
    pub active_records_max: String,
    pub active_records_avg: String,
    pub consumer_lag_max: String,
}

impl ConnectBackpressureView {
    fn new(active: ConnectBackpressure, expired: bool) -> Self {
        Self {
            connector: active.connector,
            task: active.task,
            worker: active.worker,
            kind: active.kind.label(),
            active_records: format_queue_measure(active.active_records, expired, 0),
            active_records_max: format_queue_measure(active.active_records_max, expired, 0),
            active_records_avg: format_queue_measure(active.active_records_avg, expired, 2),
            consumer_lag_max: format_queue_measure(active.consumer_lag_max, expired, 0),
        }
    }
}

/// Expire and format one backpressure measurement like the streaming panel.
fn format_queue_measure(measurement: MetricLookup, expired: bool, precision: usize) -> String {
    format_streaming_measurement(expire_lookup(measurement, expired), false, precision)
}

/// Bound Debezium queue rows only after projecting the complete snapshot.
pub fn queue_views(
    queues: Vec<DebeziumQueue>,
    expired: bool,
    row_limit: usize,
) -> (Vec<QueueBackpressureView>, usize) {
    let omitted = queues.len().saturating_sub(row_limit);
    let views = queues
        .into_iter()
        .take(row_limit)
        .map(|row| QueueBackpressureView::new(row, expired))
        .collect();
    (views, omitted)
}

/// Bound Connect active-record rows only after projecting the complete
/// snapshot.
pub fn active_views(
    actives: Vec<ConnectBackpressure>,
    expired: bool,
    row_limit: usize,
) -> (Vec<ConnectBackpressureView>, usize) {
    let omitted = actives.len().saturating_sub(row_limit);
    let views = actives
        .into_iter()
        .take(row_limit)
        .map(|row| ConnectBackpressureView::new(row, expired))
        .collect();
    (views, omitted)
}

#[derive(Template)]
#[template(path = "metrics.html")]
pub struct MetricsPanel<'a> {
    pub state: MetricState,
    pub workers: Vec<MetricsWorkerView<'a>>,
    pub throughput: Vec<ThroughputConnectorView>,
    pub throughput_omitted: usize,
    pub streaming: Vec<DebeziumStreamingView>,
    pub streaming_omitted: usize,
    pub queues: Vec<QueueBackpressureView>,
    pub queues_omitted: usize,
    pub actives: Vec<ConnectBackpressureView>,
    pub actives_omitted: usize,
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
    pub omitted: usize,
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

#[cfg(test)]
#[path = "views_tests.rs"]
mod tests;
