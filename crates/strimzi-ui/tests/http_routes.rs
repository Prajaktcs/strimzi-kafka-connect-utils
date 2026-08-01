use axum::body::Body;
use axum::http::{Request, StatusCode};
use strimzi_ops_core::ConnectionSettings;
use strimzi_ui::{router, AppState};
use tower::ServiceExt;

#[tokio::test]
async fn monitor_placeholder_renders() {
    let state = AppState::new(ConnectionSettings::default());
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/monitor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("coming in a follow-up"));
}

#[tokio::test]
async fn dashboard_missing_config_page() {
    let state = AppState::new(ConnectionSettings::default());
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Configuration required"));
}

#[tokio::test]
async fn control_missing_config_page() {
    let state = AppState::new(ConnectionSettings::default());
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/control")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("Configuration required"));
}

#[tokio::test]
async fn root_redirects_to_dashboard() {
    let state = AppState::new(ConnectionSettings::default());
    let app = router(state);
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get("location").unwrap(), "/dashboard");
}
