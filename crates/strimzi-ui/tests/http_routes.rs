use axum::body::Body;
use axum::http::{Request, StatusCode};
use strimzi_ops_core::ConnectionSettings;
use strimzi_ui::{router, AppState};
use tower::ServiceExt;

fn app(settings: ConnectionSettings) -> axum::Router {
    router(AppState::from_single(settings))
}

#[tokio::test]
async fn monitor_requires_bootstrap_servers() {
    let response = app(ConnectionSettings::default())
        .oneshot(
            Request::builder()
                .uri("/c/default/monitor")
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
    assert!(text.contains("Bootstrap servers required"));
}

#[tokio::test]
async fn monitor_form_renders_with_bootstrap() {
    let response = app(ConnectionSettings {
        bootstrap_servers: Some("localhost:9092".to_owned()),
        ..ConnectionSettings::default()
    })
    .oneshot(
        Request::builder()
            .uri("/c/default/monitor")
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
    assert!(text.contains("Start Monitoring"));
    assert!(text.contains("debezium.notifications"));
    assert!(text.contains("/c/default/monitor"));
}

#[tokio::test]
async fn dashboard_missing_config_page() {
    let response = app(ConnectionSettings::default())
        .oneshot(
            Request::builder()
                .uri("/c/default/dashboard")
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
    let response = app(ConnectionSettings::default())
        .oneshot(
            Request::builder()
                .uri("/c/default/control")
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
async fn root_redirects_to_cluster_dashboard() {
    let response = app(ConnectionSettings::default())
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "/c/default/dashboard"
    );
}

#[tokio::test]
async fn unknown_cluster_is_not_found() {
    let response = app(ConnectionSettings::default())
        .oneshot(
            Request::builder()
                .uri("/c/nope/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn control_logs_route_returns_page() {
    let response = app(ConnectionSettings {
        connect_cluster_name: Some("test-cluster".to_owned()),
        ..ConnectionSettings::default()
    })
    .oneshot(
        Request::builder()
            .uri("/c/test-cluster/control/demo-connector/logs")
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
    assert!(text.contains("Connector logs"));
    assert!(text.contains("demo-connector"));
    assert!(text.contains("Refresh Logs"));
    assert!(text.contains("/c/test-cluster/control"));
}

#[tokio::test]
async fn sidebar_lists_multiple_clusters() {
    use strimzi_ops_core::ConnectCluster;

    let state = AppState::new(vec![
        ConnectCluster {
            id: "local".to_owned(),
            settings: ConnectionSettings::default(),
        },
        ConnectCluster {
            id: "prod".to_owned(),
            settings: ConnectionSettings::default(),
        },
    ]);
    let response = router(state)
        .oneshot(
            Request::builder()
                .uri("/c/prod/dashboard")
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
    assert!(text.contains("/c/local/dashboard"));
    assert!(text.contains("/c/prod/dashboard"));
}

#[tokio::test]
async fn reject_connector_creation_for_get_and_post() {
    let app = app(ConnectionSettings::default());
    for method in ["GET", "POST"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/c/default/control/create")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
    }
}

#[tokio::test]
async fn reject_unknown_cluster_snapshot_form() {
    let response = app(ConnectionSettings::default())
        .oneshot(
            Request::builder()
                .uri("/c/nope/control/demo/snapshot")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn escape_untrusted_cluster_in_error_page() {
    let response = app(ConnectionSettings::default())
        .oneshot(
            Request::builder()
                .uri("/c/%3Cscript%3Ealert%281%29%3C%2Fscript%3E/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(!text.contains("<script>"));
}
