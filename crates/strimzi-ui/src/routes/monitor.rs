use std::convert::Infallible;
use std::time::{Duration, Instant};

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use strimzi_ops_core::metrics::{
    connect_backpressure, connector_throughput, debezium_queues, debezium_streaming, MetricState,
    STALE_AFTER,
};
use strimzi_ops_core::{NotificationMonitor, SnapshotTracker};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

use crate::blocking::spawn_blocking;
use crate::error::Error;
use crate::state::AppState;
use crate::views::{
    active_views, queue_views, render, snapshot_metrics, streaming_views, throughput_views,
    HtmlResult, MetricsPanel, MetricsWorkerView, MissingBootstrapPage, MonitorPage,
    MonitorResultsPage, SnapshotCards,
};

const DEFAULT_TOPIC: &str = "debezium.notifications";
const DEFAULT_DURATION: u64 = 60;
const MIN_DURATION: u64 = 10;
const MAX_DURATION: u64 = 300;

pub async fn monitor(State(state): State<AppState>) -> HtmlResult {
    render(MonitorPage {
        active: "monitor",
        topic: DEFAULT_TOPIC.to_owned(),
        duration: DEFAULT_DURATION,
        error: None,
        has_bootstrap: state.bootstrap_servers().is_some(),
    })
}

const MAX_DISPLAYED_PER_WORKER: usize = 25;
const MAX_DISPLAYED_TOTAL: usize = 200;
const MAX_DISPLAYED_STREAMING_ROWS: usize = 200;

/// Browsers only read the application cache; this handler never scrapes workers.
pub async fn metrics(State(state): State<AppState>) -> HtmlResult {
    let cached = state.metrics_snapshot();
    let elapsed = cached.received.elapsed();
    let expired = elapsed >= STALE_AFTER;
    let panel_state = if expired && cached.snapshot.state == MetricState::Available {
        MetricState::Stale
    } else {
        cached.snapshot.state
    };
    let (streaming, streaming_omitted) = streaming_views(
        debezium_streaming(&cached.snapshot),
        expired,
        MAX_DISPLAYED_STREAMING_ROWS,
    );
    let mut remaining = MAX_DISPLAYED_TOTAL;
    let workers = cached
        .snapshot
        .workers
        .iter()
        .map(|worker| {
            let age = worker.age_secs.map_or_else(
                || "never".to_owned(),
                |age| format!("{} s", age.saturating_add(elapsed.as_secs())),
            );
            let mut finite = worker
                .samples
                .iter()
                .filter(|sample| sample.value.is_finite());
            let samples: Vec<_> = finite
                .by_ref()
                .take(remaining.min(MAX_DISPLAYED_PER_WORKER))
                .collect();
            remaining -= samples.len();
            let omitted = finite.count();
            MetricsWorkerView {
                worker: &worker.worker,
                state: if expired && worker.state == MetricState::Available {
                    MetricState::Stale
                } else {
                    worker.state
                },
                age,
                error: worker.error.as_deref(),
                samples,
                omitted,
            }
        })
        .collect();
    let (throughput, throughput_omitted) = throughput_views(
        connector_throughput(&cached.snapshot),
        expired,
        MAX_DISPLAYED_TOTAL,
    );
    let (queues, queues_omitted) = queue_views(
        debezium_queues(&cached.snapshot),
        expired,
        MAX_DISPLAYED_TOTAL,
    );
    let (actives, actives_omitted) = active_views(
        connect_backpressure(&cached.snapshot),
        expired,
        MAX_DISPLAYED_TOTAL,
    );
    render(MetricsPanel {
        state: panel_state,
        workers,
        throughput,
        throughput_omitted,
        streaming,
        streaming_omitted,
        snapshot_metrics: snapshot_metrics(&cached, None),
        queues,
        queues_omitted,
        actives,
        actives_omitted,
    })
}

#[derive(Debug, Deserialize)]
pub struct MonitorForm {
    pub topic: String,
    pub duration: u64,
}

pub async fn monitor_submit(
    State(state): State<AppState>,
    Form(form): Form<MonitorForm>,
) -> HtmlResult {
    if state.bootstrap_servers().is_none() {
        return render(MissingBootstrapPage { active: "monitor" });
    }

    let topic = form.topic.trim().to_owned();
    if topic.is_empty() {
        return render(MonitorPage {
            active: "monitor",
            topic: DEFAULT_TOPIC.to_owned(),
            duration: form.duration.clamp(MIN_DURATION, MAX_DURATION),
            error: Some("Notification topic is required".to_owned()),
            has_bootstrap: true,
        });
    }

    let duration = form.duration.clamp(MIN_DURATION, MAX_DURATION);
    render(MonitorResultsPage {
        active: "monitor",
        topic,
        duration,
    })
}

/// Each SSE connection owns its consumer; independent groups prevent viewers
/// from splitting notifications between them.
pub async fn monitor_events(
    State(state): State<AppState>,
    Query(form): Query<MonitorForm>,
) -> Response {
    let Some(bootstrap) = state.bootstrap_servers() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Kafka bootstrap servers are required",
        )
            .into_response();
    };
    let topic = form.topic.trim().to_owned();
    if topic.is_empty() {
        return (StatusCode::BAD_REQUEST, "Notification topic is required").into_response();
    }

    let duration = form.duration.clamp(MIN_DURATION, MAX_DURATION);
    let (sender, receiver) = watch::channel(
        Event::default()
            .event("connecting")
            .data("Connecting to Kafka"),
    );
    let worker_sender = sender.clone();
    tokio::spawn(async move {
        if let Err(err) = spawn_blocking(move || {
            run_monitor_session(state, bootstrap, topic, duration, &worker_sender)
        })
        .await
        {
            let _ = sender.send_replace(
                Event::default()
                    .event("monitor-error")
                    .data(err.to_string()),
            );
        }
    });

    // A latest-value channel keeps memory bounded without blocking consumption
    // or the deadline behind a slow browser.
    let stream = WatchStream::new(receiver).map(Ok::<_, Infallible>);
    (
        [(header::HeaderName::from_static("x-accel-buffering"), "no")],
        Sse::new(stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}

fn run_monitor_session(
    state: AppState,
    bootstrap: String,
    topic: String,
    duration_secs: u64,
    sender: &watch::Sender<Event>,
) -> crate::result::Result<()> {
    if sender.is_closed() {
        return Ok(());
    }
    let started = Instant::now();
    let duration = Duration::from_secs(duration_secs);
    let mut monitor = NotificationMonitor::new(bootstrap, topic);
    let group = format!("strimzi-ops-monitor-{}", uuid::Uuid::new_v4());
    let mut notification_error = monitor
        .start_ephemeral(&group)
        .err()
        .map(|err| err.to_string());
    let mut monitor = notification_error.is_none().then_some(monitor);
    let mut tracker = SnapshotTracker::new();
    let mut last_metrics = None;

    while !sender.is_closed() {
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let wait = remaining.min(Duration::from_millis(250));
        let mut changed = false;
        if let Some(consumer) = monitor.as_mut() {
            match consumer.poll(wait) {
                Ok(Some(notification)) => {
                    tracker.process_notification(&notification);
                    changed = true;
                }
                Ok(None) => {}
                Err(err) => {
                    notification_error = Some(err.to_string());
                    monitor = None;
                    changed = true;
                }
            }
        } else {
            std::thread::sleep(wait);
        }
        let cached = state.metrics_snapshot();
        let metrics_version = (cached.received, cached.received.elapsed() >= STALE_AFTER);
        if changed || last_metrics != Some(metrics_version) {
            let _ = sender.send_replace(Event::default().event("snapshots").data(snapshot_html(
                &tracker,
                &cached,
                notification_error.as_deref(),
            )?));
            last_metrics = Some(metrics_version);
        }
    }
    // Dropping the consumer releases it on stop, error, deadline, or disconnect.
    if !sender.is_closed() {
        let _ = sender.send_replace(Event::default().event("complete").data(snapshot_html(
            &tracker,
            &state.metrics_snapshot(),
            notification_error.as_deref(),
        )?));
    }
    Ok(())
}

fn snapshot_html(
    tracker: &SnapshotTracker,
    cached: &crate::metrics::CachedMetrics,
    notification_error: Option<&str>,
) -> crate::result::Result<String> {
    SnapshotCards::new(tracker, cached, notification_error)
        .render()
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })
}
