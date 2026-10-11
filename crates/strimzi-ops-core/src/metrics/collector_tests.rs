use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use httpmock::prelude::*;

use super::*;
use crate::metrics::MetricLookup;

struct TargetsFixture {
    directory: PathBuf,
    path: PathBuf,
}

impl TargetsFixture {
    fn new(contents: &str) -> Self {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "strimzi-metrics-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("private-workers.toml");
        let fixture = Self { directory, path };
        fixture.replace(contents);
        fixture
    }

    fn replace(&self, contents: &str) {
        fs::write(&self.path, contents).unwrap();
    }

    fn collector(&self) -> MetricsCollector {
        MetricsCollector::new(Some(self.path.clone()))
    }
}

impl Drop for TargetsFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn targets(entries: &[(&str, &str)], token: Option<&str>) -> String {
    entries
        .iter()
        .map(|(id, url)| {
            let mut entry = format!("[[workers]]\nid='{id}'\nurl='{url}'\n");
            if let Some(token) = token {
                writeln!(entry, "bearer_token='{token}'").unwrap();
            }
            entry
        })
        .collect()
}

fn labels(connector: &str, task: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("connector".to_owned(), connector.to_owned()),
        ("task".to_owned(), task.to_owned()),
    ])
}

fn assert_lookup(lookup: MetricLookup, state: MetricState, value: Option<f64>) {
    assert_eq!(lookup.state, state);
    assert_eq!(lookup.value.map(f64::to_bits), value.map(f64::to_bits));
}

#[test]
fn keeps_workers_connectors_and_all_task_labels_independent() {
    let server = MockServer::start();
    let first = server.mock(|when, then| {
        when.method(GET).path("/first");
        then.status(200).body(
            "jmx_scrape_error 0\njvm_memory_bytes_used 12345\n\
             kafka_connect_source_task_source_record_poll_total{connector=\"orders\",task=\"0\"} 11\n\
             kafka_connect_source_task_source_record_poll_total{connector=\"orders\",task=\"1\"} 12\n\
             debezium_metrics_TotalNumberOfEventsSeen{connector=\"orders\",task=\"0\",context=\"streaming\"} 7\n\
             kafka_connect_worker_task_count +Inf",
        );
    });
    let second = server.mock(|when, then| {
        when.method(GET).path("/second");
        then.status(200).body(
            "kafka_connect_source_task_source_record_poll_total{task=\"0\",connector=\"orders\"} 21\n\
             kafka_connect_source_task_source_record_poll_total{connector=\"inventory\",task=\"0\"} 22",
        );
    });
    let first_url = server.url("/first");
    let second_url = server.url("/second");
    let fixture = TargetsFixture::new(&targets(
        &[("pod-a", &first_url), ("pod-b", &second_url)],
        None,
    ));
    let snapshot = fixture.collector().collect();
    let name = "kafka_connect_source_task_source_record_poll_total";
    assert_eq!(snapshot.workers.len(), 2);
    assert_lookup(
        snapshot.lookup("pod-a", name, &labels("orders", "0")),
        MetricState::Available,
        Some(11.0),
    );
    assert_lookup(
        snapshot.lookup("pod-a", name, &labels("orders", "1")),
        MetricState::Available,
        Some(12.0),
    );
    assert_lookup(
        snapshot.lookup("pod-b", name, &labels("orders", "0")),
        MetricState::Available,
        Some(21.0),
    );
    assert_lookup(
        snapshot.lookup("pod-b", name, &labels("inventory", "0")),
        MetricState::Available,
        Some(22.0),
    );
    assert_lookup(
        snapshot.lookup("pod-a", "kafka_connect_worker_task_count", &BTreeMap::new()),
        MetricState::Unavailable,
        None,
    );
    let mut streaming = labels("orders", "0");
    streaming.insert("context".to_owned(), "streaming".to_owned());
    assert_lookup(
        snapshot.lookup(
            "pod-a",
            "debezium_metrics_TotalNumberOfEventsSeen",
            &streaming,
        ),
        MetricState::Available,
        Some(7.0),
    );
    let subset = BTreeMap::from([("connector".to_owned(), "orders".to_owned())]);
    assert_lookup(
        snapshot.lookup("pod-a", name, &subset),
        MetricState::Unavailable,
        None,
    );
    assert_lookup(
        snapshot.lookup("absent-worker", name, &labels("orders", "0")),
        MetricState::Unavailable,
        None,
    );
    assert!(snapshot.workers.iter().all(|worker| worker
        .samples
        .iter()
        .all(|sample| !sample.name.starts_with("jvm_") && sample.name != "jmx_scrape_error")));
    first.assert();
    second.assert();
}

#[test]
fn ages_last_good_values_at_boundary_and_recovers_without_fabricated_zero() {
    let server = MockServer::start();
    let mut good = server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200)
            .body("kafka_connect_worker_connector_count 7");
    });
    let url = server.url("/metrics");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &url)], None));
    let mut collector = fixture.collector();
    collector.collect();
    let last_good = collector.workers["pod-a"].last_good.unwrap();
    let fresh = collector.snapshot_at(
        (last_good + STALE_AFTER)
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
    );
    let aged = collector.snapshot_at(last_good + STALE_AFTER);
    assert_eq!(fresh.workers[0].state, MetricState::Available);
    assert_eq!(aged.workers[0].age_secs, Some(30));
    assert_lookup(
        aged.workers[0].lookup("kafka_connect_worker_connector_count", &BTreeMap::new()),
        MetricState::Stale,
        Some(7.0),
    );
    good.delete();
    let mut failure = server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(503)
            .body("private-token and internal endpoint details");
    });
    let failed = collector.collect();
    assert_lookup(
        failed.workers[0].lookup("kafka_connect_worker_connector_count", &BTreeMap::new()),
        MetricState::Stale,
        Some(7.0),
    );
    assert_eq!(collector.workers["pod-a"].last_good, Some(last_good));
    assert!(!format!("{failed:?}").contains("private-token"));
    failure.delete();
    let mut recovered = server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200)
            .body("kafka_connect_worker_connector_count 0");
    });
    let snapshot = collector.collect();
    assert_lookup(
        snapshot.workers[0].lookup("kafka_connect_worker_connector_count", &BTreeMap::new()),
        MetricState::Available,
        Some(0.0),
    );
    assert_eq!(snapshot.workers[0].error, None);
    assert!(collector.workers["pod-a"].last_good.unwrap() >= last_good);
    recovered.assert();
    recovered.delete();
    server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200)
            .body("# exporter has no applicable measurements");
    });
    let unavailable = collector.collect();
    assert_lookup(
        unavailable.workers[0].lookup("kafka_connect_worker_connector_count", &BTreeMap::new()),
        MetricState::Unavailable,
        None,
    );
    assert!(unavailable.workers[0].samples.is_empty());
    assert_eq!(unavailable.workers[0].age_secs, None);
}

#[test]
fn distinguishes_first_failures_empty_exporters_and_nonfinite_measurements() {
    let server = MockServer::start();
    let mut bad = server.mock(|when, then| {
        when.method(GET).path("/bad");
        then.status(500).body("private response error");
    });
    server.mock(|when, then| {
        when.method(GET).path("/empty");
        then.status(200)
            .body("# HELP irrelevant JVM metrics\njmx_scrape_error 0\njvm_memory_bytes_used 42");
    });
    server.mock(|when, then| {
        when.method(GET).path("/exporter-error");
        then.status(200)
            .body("jmx_scrape_error 1\nkafka_connect_worker_connector_count 99");
    });
    server.mock(|when, then| {
        when.method(GET).path("/nonfinite");
        then.status(200)
            .body("kafka_connect_worker_connector_count NaN");
    });
    let urls = [
        server.url("/bad"),
        server.url("/empty"),
        server.url("/exporter-error"),
        server.url("/nonfinite"),
    ];
    let fixture = TargetsFixture::new(&targets(
        &[
            ("bad", &urls[0]),
            ("empty", &urls[1]),
            ("exporter-error", &urls[2]),
            ("nonfinite", &urls[3]),
        ],
        None,
    ));
    let mut collector = fixture.collector();
    let snapshot = collector.collect();
    let metric = "kafka_connect_worker_connector_count";
    assert_eq!(snapshot.state, MetricState::CollectionError);
    for worker in ["bad", "exporter-error"] {
        assert_lookup(
            snapshot.lookup(worker, metric, &BTreeMap::new()),
            MetricState::CollectionError,
            None,
        );
    }
    for worker in ["empty", "nonfinite"] {
        assert_lookup(
            snapshot.lookup(worker, metric, &BTreeMap::new()),
            MetricState::Unavailable,
            None,
        );
    }
    assert!(snapshot
        .workers
        .iter()
        .all(|worker| worker.age_secs.is_none()));
    assert!(!format!("{snapshot:?}").contains("private response"));
    bad.delete();
    server.mock(|when, then| {
        when.method(GET).path("/bad");
        then.status(200)
            .body("kafka_connect_worker_connector_count 4");
    });
    let recovered = collector.collect();
    assert_lookup(
        recovered.lookup("bad", metric, &BTreeMap::new()),
        MetricState::Available,
        Some(4.0),
    );
    assert_eq!(recovered.state, MetricState::CollectionError);
    let worker = recovered
        .workers
        .iter()
        .find(|worker| worker.worker == "bad")
        .unwrap();
    assert_eq!(worker.error, None);
    assert!(worker.age_secs.is_some());
}

#[test]
fn preserves_last_membership_on_file_errors_and_reloads_valid_changes() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/a");
        then.status(200)
            .body("kafka_connect_worker_connector_count 1");
    });
    server.mock(|when, then| {
        when.method(GET).path("/b");
        then.status(200)
            .body("kafka_connect_worker_connector_count 2");
    });
    let a = server.url("/a");
    let b = server.url("/b");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &a)], None));
    let mut collector = fixture.collector();
    collector.collect();
    fixture.replace("[[workers]]\nid='pod-b'\nbearer_token='private-token'\nurl = [invalid");
    let rejected = collector.collect();
    assert_eq!(rejected.workers.len(), 1);
    assert_eq!(rejected.workers[0].worker, "pod-a");
    assert_lookup(
        rejected.lookup(
            "pod-a",
            "kafka_connect_worker_connector_count",
            &BTreeMap::new(),
        ),
        MetricState::Stale,
        Some(1.0),
    );
    assert!(!format!("{rejected:?}").contains("private-token"));
    fs::remove_file(&fixture.path).unwrap();
    assert_eq!(collector.collect().workers[0].state, MetricState::Stale);
    fixture.replace(&targets(&[("pod-a", &a), ("pod-b", &b)], None));
    let expanded = collector.collect();
    assert_eq!(expanded.workers.len(), 2);
    assert_eq!(expanded.state, MetricState::Available);
    fixture.replace(&targets(&[("pod-b", &b)], None));
    let removed = collector.collect();
    assert_eq!(removed.workers.len(), 1);
    assert_eq!(removed.workers[0].worker, "pod-b");
    fixture.replace("workers=[]");
    let empty = collector.collect();
    assert!(empty.workers.is_empty());
    assert_eq!(empty.state, MetricState::Unavailable);
    fixture.replace("private-token = [invalid");
    let invalid = collector.collect();
    assert_eq!(invalid.state, MetricState::CollectionError);
    assert!(invalid.workers.is_empty());
    assert_lookup(
        invalid.lookup(
            "pod-a",
            "kafka_connect_worker_connector_count",
            &BTreeMap::new(),
        ),
        MetricState::CollectionError,
        None,
    );
    fixture.replace(&"x".repeat(128 * 1024 + 1));
    assert_eq!(collector.collect().state, MetricState::CollectionError);
    fs::remove_file(&fixture.path).unwrap();
    fs::create_dir(&fixture.path).unwrap();
    assert_eq!(collector.collect().state, MetricState::CollectionError);
    fs::remove_dir(&fixture.path).unwrap();
    assert_eq!(collector.collect().state, MetricState::Unavailable);
}

#[test]
fn refuses_redirects_and_never_publishes_auth_endpoints_or_reflected_tokens() {
    let server = MockServer::start();
    let destination = MockServer::start();
    let downstream = destination.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200)
            .body("kafka_connect_worker_connector_count 123");
    });
    let token = "private-long-bearer-token";
    let mut redirect = server.mock(|when, then| {
        when.method(GET)
            .path("/metrics")
            .header("authorization", format!("Bearer {token}"));
        then.status(302)
            .header("location", destination.url("/metrics"));
    });
    let url = server.url("/metrics");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &url)], Some(token)));
    let mut collector = fixture.collector();
    let snapshot = collector.collect();
    redirect.assert();
    downstream.assert_hits(0);
    assert_eq!(snapshot.workers[0].state, MetricState::CollectionError);
    let public = format!("{collector:?} {snapshot:?}");
    let downstream_url = destination.base_url();
    for private in [
        token,
        url.as_str(),
        downstream_url.as_str(),
        "private-workers.toml",
    ] {
        assert!(!public.contains(private), "{public}");
    }
    redirect.delete();
    let mut unauthorized = server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(401).body(format!("{token} {url}"));
    });
    let failed = collector.collect();
    assert_eq!(
        failed.workers[0].error.as_deref(),
        Some("cannot authenticate metrics request")
    );
    assert!(!format!("{failed:?}").contains(token));
    unauthorized.delete();
    server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200).body(format!(
            "kafka_connect_worker_connector_count{{secret=\"{token}\"}} 123"
        ));
    });
    let reflected = collector.collect();
    assert_eq!(reflected.workers[0].state, MetricState::CollectionError);
    assert!(reflected.workers[0].samples.is_empty());
    assert!(!format!("{reflected:?}").contains(token));
}

#[test]
fn rejects_oversized_responses_without_disabling_other_workers() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/large");
        then.status(200).body("x".repeat(MAX_RESPONSE_BYTES + 1));
    });
    server.mock(|when, then| {
        when.method(GET).path("/healthy");
        then.status(200)
            .body("kafka_connect_worker_connector_count 3");
    });
    let large = server.url("/large");
    let healthy = server.url("/healthy");
    let fixture = TargetsFixture::new(&targets(&[("large", &large), ("healthy", &healthy)], None));
    let snapshot = fixture.collector().collect();
    assert_lookup(
        snapshot.lookup(
            "healthy",
            "kafka_connect_worker_connector_count",
            &BTreeMap::new(),
        ),
        MetricState::Available,
        Some(3.0),
    );
    let large = snapshot
        .workers
        .iter()
        .find(|worker| worker.worker == "large")
        .unwrap();
    assert_eq!(large.state, MetricState::CollectionError);
    assert_eq!(
        large.error.as_deref(),
        Some("metrics response exceeds size limit")
    );
}

#[test]
fn unhealthy_port_zero_placeholders_fail_each_worker_without_losing_identity() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/a");
        then.status(200)
            .body("kafka_connect_worker_connector_count 1");
    });
    server.mock(|when, then| {
        when.method(GET).path("/b");
        then.status(200)
            .body("kafka_connect_worker_connector_count 2");
    });
    let a = server.url("/a");
    let b = server.url("/b");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &a), ("pod-b", &b)], None));
    let mut collector = fixture.collector();
    collector.collect();
    let dead = "http://127.0.0.1:0/metrics";
    fixture.replace(&targets(&[("pod-a", dead), ("pod-b", dead)], None));
    let snapshot = collector.collect();
    assert_eq!(snapshot.workers.len(), 2);
    for worker in &snapshot.workers {
        assert_eq!(worker.state, MetricState::Stale);
        assert_eq!(worker.error.as_deref(), Some("cannot fetch metrics"));
        assert_eq!(worker.samples.len(), 1);
    }
}

#[test]
fn collects_workers_concurrently_instead_of_serial_timeout_accumulation() {
    let listeners = [
        TcpListener::bind("127.0.0.1:0").unwrap(),
        TcpListener::bind("127.0.0.1:0").unwrap(),
    ];
    let urls = listeners
        .iter()
        .map(|listener| format!("http://{}/metrics", listener.local_addr().unwrap()))
        .collect::<Vec<_>>();
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &urls[0]), ("pod-b", &urls[1])], None));
    let (arrived, arrivals) = mpsc::sync_channel(2);
    thread::scope(|scope| {
        let mut servers = Vec::new();
        let mut releases = Vec::new();
        for listener in listeners {
            let arrived = arrived.clone();
            let (release, permission) = mpsc::sync_channel(1);
            releases.push(release);
            servers.push(scope.spawn(move || {
                listener.set_nonblocking(true).unwrap();
                let deadline = Instant::now() + REQUEST_TIMEOUT * 2;
                let mut socket = loop {
                    match listener.accept() {
                        Ok((socket, _)) => break socket,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "worker request did not arrive");
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => panic!("cannot accept test request: {error}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket.set_read_timeout(Some(REQUEST_TIMEOUT)).unwrap();
                let mut buffer = [0; 4096];
                assert!(socket.read(&mut buffer).unwrap() > 0);
                // Neither exporter responds until both requests have arrived.
                arrived.send(()).unwrap();
                permission.recv_timeout(REQUEST_TIMEOUT * 2).unwrap();
                let body = "kafka_connect_worker_connector_count 5\n";
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes());
            }));
        }
        let collection = scope.spawn(|| fixture.collector().collect());
        for _ in 0..2 {
            arrivals.recv_timeout(REQUEST_TIMEOUT * 2).unwrap();
        }
        for release in releases {
            release.send(()).unwrap();
        }
        let snapshot = collection.join().unwrap();
        for server in servers {
            server.join().unwrap();
        }
        assert_eq!(snapshot.workers.len(), 2);
        assert!(snapshot
            .workers
            .iter()
            .all(|worker| worker.state == MetricState::Available));
    });
}
