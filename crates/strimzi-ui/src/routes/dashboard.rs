use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::Value;

use crate::blocking::with_connect_client;
use crate::state::AppState;
use crate::views::{
    render, ConnectorSummary, DashboardMetrics, DashboardPage, FailedItem, HtmlResult,
    MissingConfigPage, ProcessingErrorRow, ProcessingErrorsPanel, StatusCount,
};
use std::collections::BTreeMap;
use strimzi_ops_core::metrics::{FailureCounter, FailuresReport, MetricState, STALE_AFTER};

#[derive(Debug, Deserialize)]
pub struct FlashQuery {
    pub flash: Option<String>,
}

pub async fn dashboard(
    State(state): State<AppState>,
    Query(query): Query<FlashQuery>,
) -> HtmlResult {
    if !state.has_connect_url() {
        return render(MissingConfigPage {
            active: "dashboard",
        });
    }

    let url = state.require_connect_url()?;
    let flash = query.flash;
    let (mut page, dlq_configured) =
        with_connect_client(url, move |client| build_dashboard(client, flash)).await?;
    // Preserve "not configured" separately from configured but unavailable data.
    if state.settings.metrics_targets_file.is_some() {
        page.processing_errors =
            processing_errors_panel(&state.metrics_snapshot(), &dlq_configured);
    }
    render(page)
}

fn build_dashboard(
    client: &strimzi_ops_core::ConnectClient,
    flash: Option<String>,
) -> crate::result::Result<(DashboardPage, BTreeMap<String, Option<bool>>)> {
    let cluster = client.get_cluster_info()?;
    let plugins = client.get_connector_plugins()?;
    let all = client.get_all_connectors_status()?;

    let mut running_connectors = 0usize;
    let mut failed_connectors = 0usize;
    let mut total_tasks = 0usize;
    let mut running_tasks = 0usize;
    let mut status_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut connectors = Vec::new();

    for (name, info) in &all {
        let status = info.get("status").cloned().unwrap_or(Value::Null);
        let connector_state = status
            .pointer("/connector/state")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_owned();

        *status_map.entry(connector_state.clone()).or_default() += 1;
        if connector_state == "RUNNING" {
            running_connectors += 1;
        } else if connector_state == "FAILED" {
            failed_connectors += 1;
            failures.push(FailedItem {
                name: name.clone(),
                kind: "Connector".to_owned(),
                error: truncate_error(status.pointer("/connector/trace").and_then(Value::as_str)),
            });
        }

        let tasks = status
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        total_tasks += tasks.len();
        let mut tasks_running = 0usize;
        for task in &tasks {
            if task.get("state").and_then(Value::as_str) == Some("RUNNING") {
                running_tasks += 1;
                tasks_running += 1;
            } else if task.get("state").and_then(Value::as_str) == Some("FAILED") {
                let task_id = task
                    .get("id")
                    .map_or_else(|| "?".to_owned(), std::string::ToString::to_string);
                failures.push(FailedItem {
                    name: format!("{name} (Task {task_id})"),
                    kind: "Task".to_owned(),
                    error: truncate_error(task.get("trace").and_then(Value::as_str)),
                });
            }
        }

        connectors.push(ConnectorSummary {
            name: name.clone(),
            connector_type: info
                .pointer("/info/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            state: connector_state,
            tasks_running,
            tasks_total: tasks.len(),
        });
    }

    connectors.sort_by(|a, b| a.name.cmp(&b.name));

    // Expanded connector info already contains config; never expose the
    // dead-letter topic or make one additional REST request per connector.
    let dlq_configured = all
        .iter()
        .map(|(name, info)| (name.clone(), dlq_configuration(info)))
        .collect();

    let page = DashboardPage {
        active: "dashboard",
        version: cluster.version.unwrap_or_else(|| "Unknown".to_owned()),
        kafka_cluster_id: cluster
            .kafka_cluster_id
            .unwrap_or_else(|| "Unknown".to_owned()),
        plugins: plugins.len(),
        metrics: DashboardMetrics {
            total_connectors: all.len(),
            running_connectors,
            failed_connectors,
            running_tasks,
            total_tasks,
        },
        processing_errors: ProcessingErrorsPanel::default(),
        status_distribution: status_map
            .into_iter()
            .map(|(state, count)| StatusCount { state, count })
            .collect(),
        failures,
        connectors,
        flash,
    };
    Ok((page, dlq_configured))
}

fn dlq_configuration(info: &Value) -> Option<bool> {
    let connector_type = info.pointer("/info/type").and_then(Value::as_str)?;
    if connector_type != "sink" {
        return Some(false);
    }
    let config = info.pointer("/info/config").and_then(Value::as_object)?;
    Some(
        config
            .get("errors.deadletterqueue.topic.name")
            .and_then(Value::as_str)
            .is_some_and(|topic| !topic.trim().is_empty()),
    )
}

/// Exporter-derived processing errors read from the shared cache. Counts are
/// cumulative exporter counters; rates span consecutive available collections.
/// Collection failures surface as unavailable values, never as connector
/// failures, and rows require a live metric series.
const MAX_FAILURE_ROWS_PER_WORKER: usize = 25;
const MAX_FAILURE_ROWS_TOTAL: usize = 200;

fn processing_errors_panel(
    cached: &crate::metrics::CachedMetrics,
    dlq_configured: &BTreeMap<String, Option<bool>>,
) -> ProcessingErrorsPanel {
    let expired = cached.received.elapsed() >= STALE_AFTER;
    let interval = cached
        .previous
        .as_ref()
        .map(|previous| cached.received.duration_since(previous.received));
    let previous_snapshot = cached
        .previous
        .as_ref()
        .map(|previous| previous.snapshot.as_ref());
    let report = FailuresReport::build(previous_snapshot, &cached.snapshot, interval);
    let state = if expired && report.state == MetricState::Available {
        MetricState::Stale
    } else {
        report.state
    };
    let mut per_worker = BTreeMap::new();
    let mut displayed = 0;
    let mut omitted = 0;
    let rows = report
        .tasks
        .into_iter()
        .filter(|task| {
            let worker_rows = per_worker.get(&task.worker).copied().unwrap_or(0);
            if displayed >= MAX_FAILURE_ROWS_TOTAL || worker_rows >= MAX_FAILURE_ROWS_PER_WORKER {
                omitted += 1;
                return false;
            }
            if let Some(count) = per_worker.get_mut(&task.worker) {
                *count += 1;
            } else {
                per_worker.insert(task.worker.clone(), 1);
            }
            displayed += 1;
            true
        })
        .map(|task| {
            let state = if expired && task.state == MetricState::Available {
                MetricState::Stale
            } else {
                task.state
            };
            let dlq_configured = dlq_configured.get(&task.connector).copied().flatten();
            ProcessingErrorRow {
                worker: task.worker,
                connector: task.connector,
                task: task.task,
                state,
                counter_reset: task.counter_reset,
                record_errors: rounded(task.record_errors, state),
                record_failures: rounded(task.record_failures, state),
                retries: rounded(task.retries, state),
                records_skipped: rounded(task.records_skipped, state),
                errors_logged: rounded(task.errors_logged, state),
                dlq_writes: rounded(task.dlq_writes, state),
                dlq_failures: rounded(task.dlq_failures, state),
                dlq_configured,
                last_error_epoch: task.last_error_epoch,
            }
        })
        .collect();
    ProcessingErrorsPanel {
        state: Some(state),
        rows,
        omitted,
    }
}

fn rounded(counter: FailureCounter, state: MetricState) -> FailureCounter {
    FailureCounter {
        count: counter.count,
        rate_per_sec: counter
            .rate_per_sec
            .filter(|_| state == MetricState::Available)
            .map(|rate| (rate * 100.0).round() / 100.0),
    }
}

fn truncate_error(trace: Option<&str>) -> String {
    let text = trace.unwrap_or("Unknown error");
    if text.len() <= 200 {
        text.to_owned()
    } else {
        format!("{}...", &text[..200])
    }
}

#[cfg(test)]
mod tests {
    use askama::Template;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use strimzi_ops_core::metrics::{MetricSample, MetricsSnapshot, WorkerMetrics};

    use super::*;

    fn snapshot(state: MetricState, errors: f64) -> Arc<MetricsSnapshot> {
        let labels = BTreeMap::from([
            ("connector".to_owned(), "orders".to_owned()),
            ("task".to_owned(), "0".to_owned()),
        ]);
        Arc::new(MetricsSnapshot {
            workers: vec![WorkerMetrics {
                worker: "w0".to_owned(),
                state,
                samples: Arc::new(vec![
                    MetricSample {
                        name: "kafka_connect_task_error_total_record_errors_total".to_owned(),
                        labels: labels.clone(),
                        value: errors,
                    },
                    MetricSample {
                        name: "kafka_connect_task_error_deadletterqueue_produce_requests_total"
                            .to_owned(),
                        labels,
                        value: 0.0,
                    },
                ]),
                age_secs: Some(0),
                error: None,
            }],
            state,
        })
    }

    #[test]
    fn suppress_rates_when_cache_expires() {
        let received = Instant::now()
            .checked_sub(STALE_AFTER + Duration::from_secs(1))
            .unwrap();
        let cached = crate::metrics::CachedMetrics {
            snapshot: snapshot(MetricState::Available, 30.0),
            received,
            previous: Some(crate::metrics::PreviousMetrics {
                snapshot: snapshot(MetricState::Available, 10.0),
                received: received.checked_sub(Duration::from_secs(10)).unwrap(),
            }),
        };
        let panel = processing_errors_panel(&cached, &BTreeMap::new());
        assert_eq!(panel.state, Some(MetricState::Stale));
        let row = &panel.rows[0];
        assert_eq!(row.state, MetricState::Stale);
        assert_eq!(row.record_errors.count, Some(30.0));
        assert_eq!(row.record_errors.rate_per_sec, None);
    }

    #[test]
    fn distinguish_sink_dlq_from_source_or_missing_configuration() {
        for (info, expected) in [
            (
                json!({"info": {"type": "sink", "config": {
                    "errors.deadletterqueue.topic.name": "private-topic"
                }}}),
                Some(true),
            ),
            (json!({"info": {"type": "sink", "config": {}}}), Some(false)),
            (
                json!({"info": {"type": "source", "config": {
                    "errors.deadletterqueue.topic.name": "ignored-for-source"
                }}}),
                Some(false),
            ),
            (json!({"info": {"type": "sink"}}), None),
        ] {
            assert_eq!(dlq_configuration(&info), expected);
        }
    }

    #[test]
    fn hide_exported_dlq_zero_when_not_configured() {
        let cached = crate::metrics::CachedMetrics {
            snapshot: snapshot(MetricState::Available, 3.0),
            received: Instant::now(),
            previous: None,
        };
        let dlq_configured = BTreeMap::from([("orders".to_owned(), Some(false))]);
        let page = DashboardPage {
            active: "dashboard",
            version: "test".to_owned(),
            kafka_cluster_id: "test".to_owned(),
            plugins: 1,
            metrics: DashboardMetrics {
                total_connectors: 1,
                running_connectors: 1,
                failed_connectors: 0,
                running_tasks: 1,
                total_tasks: 1,
            },
            processing_errors: processing_errors_panel(&cached, &dlq_configured),
            status_distribution: Vec::new(),
            failures: Vec::new(),
            connectors: Vec::new(),
            flash: None,
        };
        let html = page.render().unwrap();
        let row = html
            .split("<td>orders</td>")
            .nth(1)
            .unwrap()
            .split("</tr>")
            .next()
            .unwrap();
        let cells: Vec<_> = row.split("</td>").collect();
        // After the connector: task, collection, errors, failures, retries,
        // skipped, logged, DLQ attempts, DLQ failures, DLQ status, timestamp.
        assert!(cells[2].contains('3'));
        assert!(cells[7].contains("not configured"));
        assert!(cells[8].contains("not configured"));
        assert!(!cells[7].contains(">0"));
        assert!(!cells[8].contains(">0"));
        assert!(cells[10].contains("unavailable"));
    }

    #[test]
    fn cap_each_worker_and_total_rows_without_losing_omission_counts() {
        for (workers, tasks) in [(1, 26), (9, 25)] {
            let cached = crate::metrics::CachedMetrics {
                snapshot: Arc::new(MetricsSnapshot {
                    workers: (0..workers)
                        .map(|worker| WorkerMetrics {
                            worker: format!("worker-{worker}"),
                            state: MetricState::Available,
                            samples: Arc::new(
                                (0..tasks)
                                    .map(|task| MetricSample {
                                        name: "kafka_connect_task_error_total_record_errors_total"
                                            .to_owned(),
                                        labels: BTreeMap::from([
                                            ("connector".to_owned(), "orders".to_owned()),
                                            ("task".to_owned(), task.to_string()),
                                        ]),
                                        value: 7.0,
                                    })
                                    .collect(),
                            ),
                            age_secs: Some(0),
                            error: None,
                        })
                        .collect(),
                    state: MetricState::Available,
                }),
                received: Instant::now(),
                previous: None,
            };
            let panel = processing_errors_panel(&cached, &BTreeMap::new());
            let expected =
                (workers * tasks.min(MAX_FAILURE_ROWS_PER_WORKER)).min(MAX_FAILURE_ROWS_TOTAL);
            assert_eq!(panel.rows.len(), expected);
            assert_eq!(panel.omitted, workers * tasks - expected);
            let mut per_worker = BTreeMap::new();
            for row in &panel.rows {
                *per_worker.entry(&row.worker).or_insert(0) += 1;
                assert_eq!(row.record_errors.count, Some(7.0));
            }
            assert!(per_worker
                .values()
                .all(|count| *count <= MAX_FAILURE_ROWS_PER_WORKER));
        }
    }
}
