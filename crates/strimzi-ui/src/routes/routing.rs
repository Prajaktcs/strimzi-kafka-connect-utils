use axum::extract::State;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use super::{control, dashboard, monitor};

use crate::paths;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ClusterPath {
    pub cluster: String,
}

#[derive(Debug, Deserialize)]
pub struct ConnectorPath {
    pub cluster: String,
    pub name: String,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(redirect_home))
        .route("/dashboard", get(redirect_legacy_dashboard))
        .route("/monitor", get(redirect_legacy_monitor))
        .route("/control", get(redirect_legacy_control))
        .route("/c/{cluster}/dashboard", get(dashboard::dashboard))
        .route(
            "/c/{cluster}/monitor",
            get(monitor::monitor).post(monitor::monitor_submit),
        )
        .route("/c/{cluster}/control", get(control::control_list))
        .route(
            "/c/{cluster}/control/{name}/pause",
            post(control::pause_connector),
        )
        .route(
            "/c/{cluster}/control/{name}/resume",
            post(control::resume_connector),
        )
        .route(
            "/c/{cluster}/control/{name}/restart",
            post(control::restart_connector),
        )
        .route(
            "/c/{cluster}/control/{name}/snapshot",
            get(control::snapshot_form).post(control::snapshot_submit),
        )
        .route("/c/{cluster}/control/{name}/yaml", get(control::yaml_view))
        .route(
            "/c/{cluster}/control/{name}/yaml/download",
            get(control::yaml_download),
        )
        .route(
            "/c/{cluster}/control/{name}/edit",
            get(control::edit_form).post(control::edit_submit),
        )
        .route("/c/{cluster}/control/{name}/logs", get(control::logs_view))
        .with_state(state)
}

async fn redirect_home(State(state): State<AppState>) -> Redirect {
    Redirect::to(&paths::dashboard(state.default_cluster_id()))
}

async fn redirect_legacy_dashboard(State(state): State<AppState>) -> Redirect {
    Redirect::to(&paths::dashboard(state.default_cluster_id()))
}

async fn redirect_legacy_monitor(State(state): State<AppState>) -> Redirect {
    Redirect::to(&paths::monitor(state.default_cluster_id()))
}

async fn redirect_legacy_control(State(state): State<AppState>) -> Redirect {
    Redirect::to(&paths::control(state.default_cluster_id()))
}
