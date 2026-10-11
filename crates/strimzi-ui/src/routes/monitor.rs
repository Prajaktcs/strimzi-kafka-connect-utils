use std::convert::Infallible;
use std::time::{Duration, Instant};

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use strimzi_ops_core::{NotificationMonitor, SnapshotTracker};
use tokio::sync::watch;
use tokio_stream::wrappers::WatchStream;
use tokio_stream::StreamExt;

use crate::blocking::spawn_blocking;
use crate::error::Error;
use crate::state::AppState;
use crate::views::{
    render, HtmlResult, MissingBootstrapPage, MonitorPage, MonitorResultsPage, SnapshotCard,
    SnapshotCards,
};

const DEFAULT_TOPIC: &str = "debezium.notifications";
const DEFAULT_DURATION: u64 = 60;
const MIN_DURATION: u64 = 10;
const MAX_DURATION: u64 = 300;

pub async fn monitor(State(state): State<AppState>) -> HtmlResult {
    if state.bootstrap_servers().is_none() {
        return render(MissingBootstrapPage { active: "monitor" });
    }
    render(MonitorPage {
        active: "monitor",
        topic: DEFAULT_TOPIC.to_owned(),
        duration: DEFAULT_DURATION,
        error: None,
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
        if let Err(err) =
            spawn_blocking(move || run_monitor_session(bootstrap, topic, duration, &worker_sender))
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
    bootstrap: String,
    topic: String,
    duration_secs: u64,
    sender: &watch::Sender<Event>,
) -> crate::result::Result<()> {
    if sender.is_closed() {
        return Ok(());
    }
    let mut monitor = NotificationMonitor::new(bootstrap, topic);
    let group = format!("strimzi-ops-monitor-{}", uuid::Uuid::new_v4());
    monitor.start(&group)?;
    let _ = sender.send_replace(Event::default().event("listening").data("Listening"));
    let mut tracker = SnapshotTracker::new();
    let started = Instant::now();
    let duration = Duration::from_secs(duration_secs);

    while !sender.is_closed() {
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        if let Some(notification) = monitor.poll(remaining.min(Duration::from_millis(250)))? {
            tracker.process_notification(&notification);
            let _ = sender.send_replace(snapshot_event(&tracker)?);
        }
    }
    // Dropping the monitor also releases the consumer on errors/disconnects.
    if !sender.is_closed() {
        let _ = sender.send_replace(
            Event::default()
                .event("complete")
                .data(snapshot_html(&tracker)?),
        );
    }
    Ok(())
}

fn snapshot_event(tracker: &SnapshotTracker) -> crate::result::Result<Event> {
    Ok(Event::default()
        .event("snapshots")
        .data(snapshot_html(tracker)?))
}

fn snapshot_html(tracker: &SnapshotTracker) -> crate::result::Result<String> {
    let mut cards: Vec<SnapshotCard<'_>> = tracker
        .get_all_snapshots()
        .iter()
        .map(|(name, state)| SnapshotCard {
            name,
            status: &state.status,
            progress: state.progress.min(100),
        })
        .collect();
    cards.sort_unstable_by(|a, b| a.name.cmp(b.name));
    SnapshotCards { snapshots: cards }
        .render()
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })
}
