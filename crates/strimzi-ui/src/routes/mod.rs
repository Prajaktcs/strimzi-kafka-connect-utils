pub mod control;
pub mod dashboard;
pub mod monitor;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { axum::response::Redirect::to("/dashboard") }),
        )
        .route("/dashboard", get(dashboard::dashboard))
        .route("/monitor", get(monitor::monitor))
        .route("/control", get(control::control_list))
        .route(
            "/control/create",
            get(control::create_form).post(control::create_submit),
        )
        .route("/control/{name}/pause", post(control::pause_connector))
        .route("/control/{name}/resume", post(control::resume_connector))
        .route("/control/{name}/restart", post(control::restart_connector))
        .route(
            "/control/{name}/snapshot",
            get(control::snapshot_form).post(control::snapshot_submit),
        )
        .route("/control/{name}/yaml", get(control::yaml_view))
        .route("/control/{name}/yaml/download", get(control::yaml_download))
        .route(
            "/control/{name}/edit",
            get(control::edit_form).post(control::edit_submit),
        )
        .with_state(state)
}
