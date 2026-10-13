use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use httpmock::prelude::*;

use super::*;
use crate::metrics::MetricLookup;

#[path = "../../tests/support/tls.rs"]
mod tls;
use tls::TlsMockServer;

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

fn targets(entries: &[(&str, &str)], token: Option<&str>, ca_path: &std::path::Path) -> String {
    entries
        .iter()
        .map(|(id, url)| {
            let mut entry = format!("[[workers]]\nid='{id}'\nurl='{url}'\n");
            writeln!(entry, "ca_file={:?}", ca_path.to_str().unwrap()).unwrap();
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
    let server = TlsMockServer::start();
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
        server.ca_path(),
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
    let server = TlsMockServer::start();
    let mut good = server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200)
            .body("kafka_connect_worker_connector_count 7");
    });
    let url = server.url("/metrics");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &url)], None, server.ca_path()));
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
    let server = TlsMockServer::start();
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
        server.ca_path(),
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
    let server = TlsMockServer::start();
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
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &a)], None, server.ca_path()));
    let mut collector = fixture.collector();
    fs::remove_file(&fixture.path).unwrap();
    let missing = collector.collect();
    assert_eq!(missing.state, MetricState::CollectionError);
    assert_lookup(
        missing.lookup(
            "pod-a",
            "kafka_connect_worker_connector_count",
            &BTreeMap::new(),
        ),
        MetricState::CollectionError,
        None,
    );
    fixture.replace(&targets(&[("pod-a", &a)], None, server.ca_path()));
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
    fixture.replace(&targets(
        &[("pod-a", &a), ("pod-b", &b)],
        None,
        server.ca_path(),
    ));
    let expanded = collector.collect();
    assert_eq!(expanded.workers.len(), 2);
    assert_eq!(expanded.state, MetricState::Available);
    fixture.replace(&targets(&[("pod-b", &b)], None, server.ca_path()));
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
    assert_eq!(collector.collect().state, MetricState::CollectionError);
}

#[test]
fn refuses_redirects_and_never_publishes_auth_endpoints_or_reflected_tokens() {
    let server = TlsMockServer::start();
    let destination = TlsMockServer::start();
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
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &url)], Some(token), server.ca_path()));
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
    let server = TlsMockServer::start();
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
    let fixture = TargetsFixture::new(&targets(
        &[("large", &large), ("healthy", &healthy)],
        None,
        server.ca_path(),
    ));
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
    let server = TlsMockServer::start();
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
    let fixture = TargetsFixture::new(&targets(
        &[("pod-a", &a), ("pod-b", &b)],
        None,
        server.ca_path(),
    ));
    let mut collector = fixture.collector();
    collector.collect();
    let dead = "https://127.0.0.1:0/metrics";
    fixture.replace(&targets(
        &[("pod-a", dead), ("pod-b", dead)],
        None,
        server.ca_path(),
    ));
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
    let tls_servers = listeners
        .iter()
        .map(|listener| TlsMockServer::start_forwarding_to(listener.local_addr().unwrap()))
        .collect::<Vec<_>>();
    let urls = tls_servers
        .iter()
        .map(|server| server.url("/metrics"))
        .collect::<Vec<_>>();
    let text = format!(
        "{}{}",
        targets(&[("pod-a", &urls[0])], None, tls_servers[0].ca_path()),
        targets(&[("pod-b", &urls[1])], None, tls_servers[1].ca_path()),
    );
    let fixture = TargetsFixture::new(&text);
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

#[test]
fn no_configuration_is_unavailable_but_a_missing_configured_file_is_an_error() {
    let unconfigured = MetricsCollector::new(None).collect();
    assert_eq!(unconfigured.state, MetricState::Unavailable);
    assert!(unconfigured.workers.is_empty());
    let fixture = TargetsFixture::new("workers=[]");
    fs::remove_file(&fixture.path).unwrap();
    let missing = fixture.collector().collect();
    assert_eq!(missing.state, MetricState::CollectionError);
    assert!(missing.workers.is_empty());
}

#[test]
fn custom_roots_are_never_shared_between_workers_and_hostname_checks_remain_enabled() {
    let server = TlsMockServer::start();
    let unrelated = TlsMockServer::start();
    let wrong_name = TlsMockServer::start_with_names(vec!["wrong.example".to_owned()]);
    let trusted = server.mock(|when, then| {
        when.method(GET).path("/trusted");
        then.status(200).body("kafka_connect_worker_task_count 9");
    });
    let untrusted = server.mock(|when, then| {
        when.method(GET).path("/untrusted");
        then.status(200).body("kafka_connect_worker_task_count 99");
    });
    let default_roots = server.mock(|when, then| {
        when.method(GET).path("/default");
        then.status(200).body("kafka_connect_worker_task_count 99");
    });
    let hostname = wrong_name.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200).body("kafka_connect_worker_task_count 99");
    });
    let localhost = server.url("/trusted").replace("127.0.0.1", "localhost");
    let text = format!(
        "{}{}{}[[workers]]\nid='default'\nurl='{}'\n",
        targets(&[("trusted", &localhost)], None, server.ca_path()),
        targets(
            &[("untrusted", &server.url("/untrusted"))],
            None,
            unrelated.ca_path()
        ),
        targets(
            &[("hostname", &wrong_name.url("/metrics"))],
            None,
            wrong_name.ca_path()
        ),
        server.url("/default"),
    );
    let fixture = TargetsFixture::new(&text);
    let snapshot = fixture.collector().collect();
    assert_lookup(
        snapshot.lookup(
            "trusted",
            "kafka_connect_worker_task_count",
            &BTreeMap::new(),
        ),
        MetricState::Available,
        Some(9.0),
    );
    for id in ["untrusted", "hostname", "default"] {
        let worker = snapshot
            .workers
            .iter()
            .find(|worker| worker.worker == id)
            .unwrap();
        assert_eq!(worker.state, MetricState::CollectionError);
        assert_eq!(worker.error.as_deref(), Some("cannot fetch metrics"));
        assert!(worker.samples.is_empty());
    }
    trusted.assert();
    untrusted.assert_hits(0);
    default_roots.assert_hits(0);
    hostname.assert_hits(0);
}

#[test]
fn rereads_relative_ca_files_and_isolates_redacted_bounded_file_errors() {
    let server = TlsMockServer::start();
    let unrelated = TlsMockServer::start();
    server.mock(|when, then| {
        when.method(GET);
        then.status(200).body("kafka_connect_worker_task_count 6");
    });
    let fixture = TargetsFixture::new("");
    let relative = std::path::Path::new("private-ca-path.crt");
    let local_ca = fixture.directory.join(relative);
    fs::copy(server.ca_path(), &local_ca).unwrap();
    fixture.replace(&format!(
        "{}{}",
        targets(&[("rotating", &server.url("/rotating"))], None, relative),
        targets(
            &[("healthy", &server.url("/healthy"))],
            None,
            server.ca_path()
        ),
    ));
    let mut collector = fixture.collector();
    assert_eq!(collector.collect().state, MetricState::Available);
    fs::copy(unrelated.ca_path(), &local_ca).unwrap();
    let wrong_root = collector.collect();
    assert_eq!(wrong_root.workers[1].worker, "rotating");
    assert_eq!(wrong_root.workers[1].state, MetricState::Stale);
    assert_eq!(
        wrong_root.workers[1].error.as_deref(),
        Some("cannot fetch metrics")
    );
    fs::copy(server.ca_path(), &local_ca).unwrap();
    assert_eq!(collector.collect().state, MetricState::Available);
    for (contents, reason) in [
        (
            b"not a PEM certificate".to_vec(),
            "cannot parse metrics CA certificate",
        ),
        (Vec::new(), "cannot parse metrics CA certificate"),
        (
            b"-----BEGIN CERTIFICATE-----\nAQID\n-----END CERTIFICATE-----\n".to_vec(),
            "cannot initialize metrics collection",
        ),
        (
            vec![b'x'; 64 * 1024 + 1],
            "metrics CA certificate exceeds size limit",
        ),
    ] {
        fs::write(&local_ca, contents).unwrap();
        let snapshot = collector.collect();
        assert_eq!(snapshot.workers[0].state, MetricState::Available);
        assert_eq!(snapshot.workers[1].state, MetricState::Stale);
        assert_eq!(snapshot.workers[1].error.as_deref(), Some(reason));
        assert_eq!(
            snapshot.workers[1].samples[0].value.to_bits(),
            6.0_f64.to_bits()
        );
        assert!(!format!("{snapshot:?} {collector:?}").contains("private-ca-path"));
    }
    fs::remove_file(&local_ca).unwrap();
    let missing = collector.collect();
    assert_eq!(missing.workers[0].state, MetricState::Available);
    assert_eq!(
        missing.workers[1].error.as_deref(),
        Some("cannot read metrics CA certificate")
    );
    fs::create_dir(&local_ca).unwrap();
    assert_eq!(
        collector.collect().workers[1].error.as_deref(),
        Some("cannot read metrics CA certificate"),
    );
    fs::remove_dir(&local_ca).unwrap();
    fs::copy(server.ca_path(), &local_ca).unwrap();
    assert_eq!(collector.collect().state, MetricState::Available);
}

#[test]
fn rejects_short_tokens_before_any_request_without_echoing_them() {
    let server = TlsMockServer::start();
    let protected = server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200)
            .body("kafka_connect_worker_connector_count 1\n");
    });
    let url = server.url("/metrics");
    let ca = server.ca_path().display().to_string();
    // A successful first round retains the worker identity, so the later
    // rejection is visible on that identity without any additional request.
    let working = TargetsFixture::new(&format!(
        "[[workers]]\nid='pod-a'\nurl='{url}'\nca_file='{ca}'\n"
    ));
    let mut collector = working.collector();
    assert_eq!(collector.collect().workers[0].state, MetricState::Available);
    protected.assert_hits(1);
    for token in ["a", "short", "1234567890123456", "1.2e+10"] {
        working.replace(&format!(
            "[[workers]]\nid='pod-a'\nurl='{url}'\nbearer_token='{token}'\nca_file='{ca}'\n"
        ));
        let snapshot = collector.collect();
        assert!(
            snapshot.workers[0]
                .error
                .as_deref()
                .is_some_and(|reason| reason.contains("collision resistant")),
            "expected rejected token, got {snapshot:?}",
        );
        assert_eq!(snapshot.state, MetricState::Stale);
        assert_lookup(
            snapshot.lookup(
                "pod-a",
                "kafka_connect_worker_connector_count",
                &BTreeMap::new(),
            ),
            MetricState::Stale,
            Some(1.0),
        );
        // For tokens shorter than a few characters, "echoed in errors" is not
        // textually decidable (any text contains them); assert echo absence
        // for the ones whose phrase is distinctive.
        if token.len() > 4 {
            assert!(!format!("{snapshot:?}").contains(token));
        }
        protected.assert_hits(1);
    }
    drop(working);
    // A rejection from the very first round keeps last-known identities empty.
    let rejected = TargetsFixture::new(&format!(
        "[[workers]]\nid='pod-a'\nurl='{url}'\nbearer_token='a'\nca_file='{ca}'\n"
    ));
    let snapshot = MetricsCollector::new(Some(rejected.path.clone())).collect();
    assert_eq!(snapshot.state, MetricState::CollectionError);
    assert!(snapshot.workers.is_empty());
    protected.assert_hits(1);
}

#[test]
fn tokens_colliding_with_metric_names_and_values_still_scrape() {
    let server = TlsMockServer::start();
    let token = "collision-resistant-bearer-token";
    server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200).body(format!(
            "kafka_connect_worker_task_count{{task=\"0\"}} 1\n\
             kafka_connect_worker_connector_count 1234567890123456\n\
             # HELP note token {token} mentioned in HELP\n\
             jmx_scrape_error 0"
        ));
    });
    let url = server.url("/metrics");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &url)], Some(token), server.ca_path()));
    let mut collector = fixture.collector();
    let snapshot = collector.collect();
    assert_eq!(snapshot.workers[0].state, MetricState::Available);
    assert_eq!(snapshot.workers[0].samples.len(), 2);
}

#[test]
fn reflected_tokens_in_published_names_or_labels_fail_the_worker() {
    let server = TlsMockServer::start();
    let token = "private-long-bearer-token";
    let url = server.url("/metrics");
    let fixture = TargetsFixture::new(&targets(&[("pod-a", &url)], Some(token), server.ca_path()));
    let mut collector = fixture.collector();

    server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200).body(format!(
            "kafka_connect_worker_{token} 1\njmx_scrape_error 0"
        ));
    });
    let reflected = collector.collect();
    assert_eq!(reflected.workers[0].state, MetricState::CollectionError);
    assert!(reflected.workers[0].samples.is_empty());
    assert!(!format!("{reflected:?}").contains(token));

    server.mock(|when, then| {
        when.method(GET).path("/metrics");
        then.status(200).body(format!(
            "kafka_connect_worker_connector_count{{secret=\"{token}\"}} 123\njmx_scrape_error 0"
        ));
    });
    let reflected = collector.collect();
    assert_eq!(reflected.workers[0].state, MetricState::CollectionError);
    assert!(reflected.workers[0].samples.is_empty());
    assert!(!format!("{reflected:?}").contains(token));
}

#[test]
fn plaintext_targets_are_rejected_before_any_collection() {
    for auth in ["", "\nbearer_token='private-token'"] {
        let fixture = TargetsFixture::new(&format!(
            "[[workers]]\nid='pod-a'\nurl='http://127.0.0.1:1/metrics'{auth}"
        ));
        let snapshot = fixture.collector().collect();
        assert_eq!(snapshot.state, MetricState::CollectionError);
        assert!(snapshot.workers.is_empty());
        assert!(!format!("{snapshot:?}").contains("private-token"));
    }
}
