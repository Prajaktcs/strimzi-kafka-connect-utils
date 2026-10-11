use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use strimzi_ops_core::ConnectionSettings;
use strimzi_ui::{router, AppState};
use tokio_stream::StreamExt;
use tower::ServiceExt;

#[tokio::test]
async fn reject_monitor_stream_without_bootstrap_servers() {
    let app = router(AppState::new(ConnectionSettings::default()));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/monitor/events?topic=notifications&duration=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn reject_monitor_stream_with_blank_topic() {
    let app = router(AppState::new(ConnectionSettings {
        bootstrap_servers: Some("localhost:9092".to_owned()),
        ..ConnectionSettings::default()
    }));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/monitor/events?topic=%20%20&duration=10")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn dispatch_monitor_status_before_the_session_deadline() {
    let app = router(AppState::new(ConnectionSettings {
        bootstrap_servers: Some("127.0.0.1:1".to_owned()),
        ..ConnectionSettings::default()
    }));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/monitor/events?topic=notifications&duration=300")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");

    let mut stream = response.into_body().into_data_stream();
    let frame = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("status must arrive before the monitoring window ends")
        .expect("stream must emit status")
        .unwrap();
    let text = std::str::from_utf8(&frame).unwrap();
    let event = text
        .lines()
        .find_map(|line| line.strip_prefix("event: "))
        .expect("named SSE event");
    assert!(matches!(event, "connecting" | "listening"));
    // EventSource discards frames without nonempty data, even with an event name.
    assert!(text.lines().any(|line| line
        .strip_prefix("data: ")
        .is_some_and(|data| !data.is_empty())));
    drop(stream);
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
async fn show_snapshot_only_for_source_connectors() {
    let server = httpmock::MockServer::start();
    let status = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/connectors")
            .query_param("expand", "status")
            .query_param("expand", "info");
        then.status(200).json_body(serde_json::json!({
            "postgres-source": {"status": {"type": "source"}},
            "iceberg-sink": {"status": {"type": "sink"}},
            "unknown": {"status": {}}
        }));
    });
    let app = router(AppState::new(ConnectionSettings {
        connect_url: Some(server.base_url()),
        ..ConnectionSettings::default()
    }));
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
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("href=\"/control/postgres-source/snapshot\""));
    assert!(!text.contains("href=\"/control/iceberg-sink/snapshot\""));
    assert!(!text.contains("href=\"/control/unknown/snapshot\""));
    assert!(text.contains("action=\"/control/iceberg-sink/restart\""));
    status.assert();
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

#[tokio::test]
async fn control_logs_route_returns_page() {
    let state = AppState::new(ConnectionSettings {
        connect_cluster_name: Some("test-cluster".to_owned()),
        ..ConnectionSettings::default()
    });
    let app = router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/control/demo-connector/logs")
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
}

#[tokio::test]
async fn reject_connector_creation_for_get_and_post() {
    let app = router(AppState::new(ConnectionSettings::default()));
    for method in ["GET", "POST"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/control/create")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
    }
}
