use std::time::Duration;

use axum::extract::{Form, State};
use serde::Deserialize;
use strimzi_ops_core::{NotificationMonitor, SnapshotTracker};

use crate::blocking::spawn_blocking;
use crate::state::AppState;
use crate::views::{
    render, HtmlResult, MissingBootstrapPage, MonitorPage, MonitorResultsPage, SnapshotCard,
};

const DEFAULT_TOPIC: &str = "debezium.notifications";
const DEFAULT_DURATION: u64 = 60;
const MIN_DURATION: u64 = 10;
const MAX_DURATION: u64 = 300;
const DEFAULT_GROUP_ID: &str = "strimzi-ops-monitor";

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
    let Some(bootstrap) = state.bootstrap_servers() else {
        return render(MissingBootstrapPage { active: "monitor" });
    };

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
    let topic_for_run = topic.clone();
    let cards =
        spawn_blocking(move || run_monitor_session(bootstrap, topic_for_run, duration)).await?;

    render(MonitorResultsPage {
        active: "monitor",
        topic,
        duration,
        snapshots: cards,
    })
}

fn run_monitor_session(
    bootstrap: String,
    topic: String,
    duration_secs: u64,
) -> crate::result::Result<Vec<SnapshotCard>> {
    let mut monitor = NotificationMonitor::new(bootstrap, topic);
    monitor.start(DEFAULT_GROUP_ID)?;
    let mut tracker = SnapshotTracker::new();
    monitor.consume_notifications(
        |notification| {
            tracker.process_notification(&notification);
        },
        Some(Duration::from_secs(duration_secs)),
    )?;

    let mut cards: Vec<SnapshotCard> = tracker
        .get_all_snapshots()
        .iter()
        .map(|(name, state)| SnapshotCard {
            name: name.clone(),
            status: state.status.clone(),
            progress: state.progress.min(100),
        })
        .collect();
    cards.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(cards)
}
