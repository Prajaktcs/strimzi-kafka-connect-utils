use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use strimzi_ops_core::ConnectionSettings;
use strimzi_ui::{router, AppState};
use tokio_stream::StreamExt;
use tower::ServiceExt;

#[path = "../../strimzi-ops-core/tests/support/tls.rs"]
mod tls;

use tls::TlsMockServer;

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
    assert!(matches!(
        event,
        "connecting" | "listening" | "monitor-error"
    ));
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

#[tokio::test]
async fn share_metric_collection_across_viewers_without_kafka_bootstrap() {
    let server = TlsMockServer::start();
    let scrape = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/metrics")
            .header("authorization", "Bearer server-only-token");
        then.status(200).body(
            "kafka_connect_source_task_source_record_poll_total{connector=\"source\",task=\"0\"} 4\n",
        );
    });
    let path = std::env::temp_dir().join(format!("strimzi-metrics-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(
        &path,
        format!(
            "[[workers]]\nid = \"worker-0\"\nurl = \"{}\"\nbearer_token = \"server-only-token\"\nca_file = '{}'\n",
            server.url("/metrics"),
            server.ca_path().display()
        ),
    )
    .unwrap();
    let state = AppState::new(ConnectionSettings {
        metrics_targets_file: Some(path.clone()),
        ..ConnectionSettings::default()
    });
    let task = state.start_metrics().unwrap();
    assert!(state.clone().start_metrics().is_none());
    tokio::time::timeout(Duration::from_secs(2), async {
        while scrape.hits() == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("shared collector must start independently of viewers");
    let app = router(state);
    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/monitor/metrics")
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
        assert!(!text.contains("server-only-token"));
        assert!(!text.contains(&server.base_url()));
    }
    assert_eq!(scrape.hits(), 1, "viewers must not initiate extra scrapes");
    task.abort();
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn distinguish_unconfigured_dashboard_metrics_from_unavailable_readings() {
    let server = httpmock::MockServer::start();
    let cluster = server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/");
        then.status(200)
            .json_body(serde_json::json!({"version":"test"}));
    });
    let plugins = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/connector-plugins");
        then.status(200).json_body(serde_json::json!([]));
    });
    let inventory = server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/connectors");
        then.status(200).json_body(serde_json::json!({}));
    });
    for (targets, expected) in [
        (None, "not configured"),
        (
            Some(std::path::PathBuf::from("configured-targets.toml")),
            "unavailable",
        ),
    ] {
        let app = router(AppState::new(ConnectionSettings {
            connect_url: Some(server.base_url()),
            metrics_targets_file: targets,
            ..ConnectionSettings::default()
        }));
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
        let html = std::str::from_utf8(&body).unwrap();
        let state = html
            .split("Exporter collection:")
            .nth(1)
            .unwrap()
            .split("</strong>")
            .next()
            .unwrap();
        assert!(state.contains(expected), "{state}");
    }
    cluster.assert_hits(2);
    plugins.assert_hits(2);
    inventory.assert_hits(2);
}
