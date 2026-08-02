//! Axum + HTMX web UI for Strimzi Ops.

pub mod blocking;
pub mod error;
pub mod result;
pub mod routes;
pub mod state;
pub mod views;

pub use routes::router;
pub use state::AppState;
