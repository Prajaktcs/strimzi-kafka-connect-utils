//! Processing-failure reports derived from per-worker Connect metrics.
//!
//! Counts are cumulative exporter counters; rates are derived from two
//! consecutive snapshots of the same `(worker, connector, task, metric)`
//! series. A counter that decreased since the previous snapshot reset (task
//! restart or worker change), so no rate is derived for that interval and the
//! row is flagged. Missing or unreachable series stay `None`; they never
//! become zero. Collection errors are reported as collection errors, never as
//! connector processing failures.

use std::collections::BTreeMap;
use std::time::Duration;

use super::types::{MetricState, MetricsSnapshot};

const RECORD_ERRORS: &str = "kafka_connect_task_error_total_record_errors_total";
const RECORD_FAILURES: &str = "kafka_connect_task_error_total_record_failures_total";
const RECORDS_SKIPPED: &str = "kafka_connect_task_error_total_records_skipped_total";
const RETRIES: &str = "kafka_connect_task_error_total_retries_total";
const ERRORS_LOGGED: &str = "kafka_connect_task_error_total_errors_logged_total";
const DLQ_REQUESTS: &str = "kafka_connect_task_error_deadletterqueue_produce_requests_total";
const DLQ_FAILURES: &str = "kafka_connect_task_error_deadletterqueue_produce_failures_total";
const LAST_ERROR_TIMESTAMP: &str = "kafka_connect_task_error_last_error_timestamp";

type SeriesKey<'a> = (&'a str, &'a BTreeMap<String, String>, &'a str);
type SampleIndex<'a> = BTreeMap<SeriesKey<'a>, (MetricState, f64)>;

/// Borrow complete identities; index each round once, without copying labels.
fn index_samples(snapshot: &MetricsSnapshot) -> SampleIndex<'_> {
    snapshot
        .workers
        .iter()
        .filter(|worker| matches!(worker.state, MetricState::Available | MetricState::Stale))
        .flat_map(|worker| {
            worker.samples.iter().filter_map(move |sample| {
                if !is_failure_metric(&sample.name) {
                    return None;
                }
                count_value(Some(sample.value)).map(|value| {
                    (
                        (worker.worker.as_str(), &sample.labels, sample.name.as_str()),
                        (worker.state, value),
                    )
                })
            })
        })
        .collect()
}

/// A cumulative counter and its derived per-second rate, if derivable.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FailureCounter {
    /// Finite, non-negative exporter counter value; no casts, no fabrication.
    pub count: Option<f64>,
    pub rate_per_sec: Option<f64>,
}

/// One `(worker, connector, task)` row. Series identity is never merged across
/// workers, connectors, or tasks.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskFailures {
    pub worker: String,
    pub connector: String,
    pub task: String,
    pub state: MetricState,
    pub record_errors: FailureCounter,
    pub record_failures: FailureCounter,
    pub retries: FailureCounter,
    pub records_skipped: FailureCounter,
    pub errors_logged: FailureCounter,
    pub dlq_writes: FailureCounter,
    pub dlq_failures: FailureCounter,
    /// A counter decreased since the previous snapshot: restart or reset.
    pub counter_reset: bool,
    /// Exporter `last-error-timestamp` (epoch milliseconds), when available.
    pub last_error_epoch: Option<f64>,
}

/// Failure rows for every worker plus the most severe collection state.
#[derive(Debug, Clone, PartialEq)]
pub struct FailuresReport {
    pub state: MetricState,
    pub tasks: Vec<TaskFailures>,
}

impl FailuresReport {
    /// Rates require consecutive available readings of the same series.
    /// A missing baseline, collection error, or decrease suppresses the rate.
    pub fn build(
        prev: Option<&MetricsSnapshot>,
        current: &MetricsSnapshot,
        interval: Option<Duration>,
    ) -> Self {
        let mut tasks = Vec::new();
        let current_index = index_samples(current);
        let previous_index = prev.map(index_samples).unwrap_or_default();
        for worker in &current.workers {
            let identities = worker
                .samples
                .iter()
                .filter(|sample| is_failure_metric(&sample.name))
                .filter_map(|sample| {
                    task_labels(&sample.labels).map(|identity| (&sample.labels, identity))
                })
                .collect::<std::collections::BTreeSet<_>>();
            for (labels, (connector, task)) in identities {
                let mut counter_reset = false;
                let mut counter = |name| {
                    failure_counter(
                        (&current_index, &previous_index),
                        &worker.worker,
                        name,
                        labels,
                        interval,
                        &mut counter_reset,
                    )
                };
                let record_errors = counter(RECORD_ERRORS);
                let record_failures = counter(RECORD_FAILURES);
                let retries = counter(RETRIES);
                let records_skipped = counter(RECORDS_SKIPPED);
                let errors_logged = counter(ERRORS_LOGGED);
                let dlq_writes = counter(DLQ_REQUESTS);
                let dlq_failures = counter(DLQ_FAILURES);
                let last_error_epoch = current_index
                    .get(&(worker.worker.as_str(), labels, LAST_ERROR_TIMESTAMP))
                    .map(|(_, value)| *value);
                tasks.push(TaskFailures {
                    worker: worker.worker.clone(),
                    connector: connector.to_owned(),
                    task: task.to_owned(),
                    state: worker.state,
                    record_errors,
                    record_failures,
                    retries,
                    records_skipped,
                    errors_logged,
                    dlq_writes,
                    dlq_failures,
                    counter_reset,
                    last_error_epoch,
                });
            }
        }
        Self {
            state: current.state,
            tasks,
        }
    }
}

fn is_failure_metric(name: &str) -> bool {
    matches!(
        name,
        RECORD_ERRORS
            | RECORD_FAILURES
            | RECORDS_SKIPPED
            | RETRIES
            | ERRORS_LOGGED
            | DLQ_REQUESTS
            | DLQ_FAILURES
            | LAST_ERROR_TIMESTAMP
    )
}

/// Only samples carrying both `connector` and `task` labels define a row.
fn task_labels(labels: &BTreeMap<String, String>) -> Option<(&str, &str)> {
    let connector = labels.get("connector")?;
    let task = labels.get("task")?;
    if connector.is_empty() || task.is_empty() {
        return None;
    }
    Some((connector.as_str(), task.as_str()))
}

#[cfg(test)]
fn task_label_map(connector: &str, task: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("connector".to_owned(), connector.to_owned()),
        ("task".to_owned(), task.to_owned()),
    ])
}

fn count_value(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value >= 0.0)
}

fn failure_counter(
    (current, previous): (&SampleIndex<'_>, &SampleIndex<'_>),
    worker: &str,
    name: &str,
    labels: &BTreeMap<String, String>,
    interval: Option<Duration>,
    counter_reset: &mut bool,
) -> FailureCounter {
    let reading = current.get(&(worker, labels, name)).copied();
    let count = reading.map(|(_, value)| value);
    let rate_per_sec = match (reading, interval) {
        (Some((MetricState::Available, current_raw)), Some(interval)) => previous
            .get(&(worker, labels, name))
            .filter(|(state, _)| *state == MetricState::Available)
            .and_then(|(_, previous_raw)| {
                let seconds = interval.as_secs_f64();
                if seconds <= 0.0 {
                    None
                } else if current_raw >= *previous_raw {
                    Some((current_raw - previous_raw) / seconds)
                } else {
                    *counter_reset = true;
                    None
                }
            }),
        _ => None,
    };
    FailureCounter {
        count,
        rate_per_sec,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::types::{MetricSample, WorkerMetrics};
    use std::sync::Arc;

    fn sample(name: &str, connector: &str, task: &str, value: f64) -> MetricSample {
        MetricSample {
            name: name.to_owned(),
            labels: task_label_map(connector, task),
            value,
        }
    }

    fn worker(name: &str, state: MetricState, samples: Vec<MetricSample>) -> MetricsSnapshot {
        MetricsSnapshot {
            workers: vec![WorkerMetrics {
                worker: name.to_owned(),
                state,
                samples: Arc::new(samples),
                age_secs: Some(0),
                error: None,
            }],
            state,
        }
    }

    fn interval() -> Duration {
        Duration::from_secs(10)
    }

    fn find<'a>(report: &'a FailuresReport, connector: &str) -> &'a TaskFailures {
        report
            .tasks
            .iter()
            .find(|task| task.connector == connector)
            .unwrap_or_else(|| panic!("missing connector {connector}"))
    }

    #[test]
    fn builds_rows_with_counts_and_derived_rates() {
        let prev = worker(
            "w0",
            MetricState::Available,
            vec![
                sample(RECORD_ERRORS, "orders", "0", 10.0),
                sample(RETRIES, "orders", "0", 5.0),
            ],
        );
        let current = worker(
            "w0",
            MetricState::Available,
            vec![
                sample(RECORD_ERRORS, "orders", "0", 30.0),
                sample(RETRIES, "orders", "0", 15.0),
                sample(DLQ_REQUESTS, "orders", "0", 2.0),
                sample(DLQ_FAILURES, "orders", "0", 1.0),
                sample(RECORD_ERRORS, "other", "1", 7.0),
            ],
        );
        let report = FailuresReport::build(Some(&prev), &current, Some(interval()));
        assert_eq!(report.tasks.len(), 2);
        let orders = find(&report, "orders");
        assert_eq!(orders.record_errors.count, Some(30.0));
        assert_eq!(orders.record_errors.rate_per_sec, Some(2.0));
        assert_eq!(orders.retries.count, Some(15.0));
        assert_eq!(orders.retries.rate_per_sec, Some(1.0));
        assert_eq!(orders.dlq_writes.count, Some(2.0));
        assert_eq!(orders.dlq_writes.rate_per_sec, None);
        assert!(!orders.counter_reset);
    }

    #[test]
    fn missing_series_stay_unavailable_not_zero() {
        let current = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", 3.0)],
        );
        let report = FailuresReport::build(None, &current, Some(interval()));
        let orders = find(&report, "orders");
        assert_eq!(orders.record_errors.count, Some(3.0));
        assert_eq!(orders.dlq_writes.count, None);
        assert_eq!(orders.retries.count, None);
    }

    #[test]
    fn collection_error_is_not_a_processing_failure() {
        let current = worker("w0", MetricState::CollectionError, Vec::new());
        let report = FailuresReport::build(None, &current, Some(interval()));
        assert_eq!(report.state, MetricState::CollectionError);
        assert_eq!(report.tasks, Vec::<TaskFailures>::new());
    }

    #[test]
    fn counter_reset_suppresses_rate_and_flags_row() {
        let prev = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", 100.0)],
        );
        let current = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", 4.0)],
        );
        let report = FailuresReport::build(Some(&prev), &current, Some(interval()));
        let orders = find(&report, "orders");
        assert_eq!(orders.record_errors.count, Some(4.0));
        assert_eq!(orders.record_errors.rate_per_sec, None);
        assert!(orders.counter_reset);
    }

    #[test]
    fn stale_rows_keep_counts_but_derive_no_rates() {
        let prev = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", 1.0)],
        );
        let mut current = worker(
            "w0",
            MetricState::Stale,
            vec![sample(RECORD_ERRORS, "orders", "0", 5.0)],
        );
        current.state = MetricState::Stale;
        let report = FailuresReport::build(Some(&prev), &current, Some(interval()));
        let orders = find(&report, "orders");
        assert_eq!(orders.state, MetricState::Stale);
        assert_eq!(orders.record_errors.count, Some(5.0));
        assert_eq!(orders.record_errors.rate_per_sec, None);
    }

    #[test]
    fn same_connector_on_another_worker_never_merges() {
        let prev = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", 10.0)],
        );
        let current = MetricsSnapshot {
            workers: vec![WorkerMetrics {
                worker: "w1".to_owned(),
                state: MetricState::Available,
                samples: Arc::new(vec![sample(RECORD_ERRORS, "orders", "0", 10.0)]),
                age_secs: Some(0),
                error: None,
            }],
            state: MetricState::Available,
        };
        let report = FailuresReport::build(Some(&prev), &current, Some(interval()));
        assert_eq!(report.tasks.len(), 1);
        let orders = &report.tasks[0];
        assert_eq!(orders.worker, "w1");
        assert_eq!(orders.record_errors.count, Some(10.0));
        assert_eq!(orders.record_errors.rate_per_sec, None);
    }

    #[test]
    fn samples_without_task_identity_are_ignored() {
        let current = MetricsSnapshot {
            workers: vec![WorkerMetrics {
                worker: "w0".to_owned(),
                state: MetricState::Available,
                samples: Arc::new(vec![
                    MetricSample {
                        name: RECORD_ERRORS.to_owned(),
                        labels: BTreeMap::from([("connector".to_owned(), "orders".to_owned())]),
                        value: 5.0,
                    },
                    MetricSample {
                        name: RECORD_ERRORS.to_owned(),
                        labels: BTreeMap::new(),
                        value: 5.0,
                    },
                ]),
                age_secs: Some(0),
                error: None,
            }],
            state: MetricState::Available,
        };
        let report = FailuresReport::build(None, &current, Some(interval()));
        assert_eq!(report.tasks, Vec::<TaskFailures>::new());
    }

    #[test]
    fn nonfinite_counts_become_unavailable() {
        let current = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", f64::NAN)],
        );
        let report = FailuresReport::build(None, &current, Some(interval()));
        let orders = find(&report, "orders");
        assert_eq!(orders.record_errors.count, None);
    }

    #[test]
    fn preserve_multiple_tasks_for_one_connector() {
        let current = worker(
            "w0",
            MetricState::Available,
            vec![
                sample(RECORD_ERRORS, "orders", "0", 3.0),
                sample(RECORD_ERRORS, "orders", "1", 9.0),
            ],
        );
        let report = FailuresReport::build(None, &current, None);
        let counts: BTreeMap<_, _> = report
            .tasks
            .iter()
            .map(|row| (row.task.as_str(), row.record_errors.count))
            .collect();
        assert_eq!(counts.get("0"), Some(&Some(3.0)));
        assert_eq!(counts.get("1"), Some(&Some(9.0)));
    }

    #[test]
    fn discard_stale_or_error_baselines_after_recovery() {
        let current = worker(
            "w0",
            MetricState::Available,
            vec![sample(RECORD_ERRORS, "orders", "0", 14.0)],
        );
        for state in [MetricState::Stale, MetricState::CollectionError] {
            let previous = worker(
                "w0",
                state,
                vec![sample(RECORD_ERRORS, "orders", "0", 10.0)],
            );
            let report = FailuresReport::build(Some(&previous), &current, Some(interval()));
            let orders = find(&report, "orders");
            assert_eq!(orders.record_errors.count, Some(14.0));
            assert_eq!(orders.record_errors.rate_per_sec, None);
        }
    }

    #[test]
    fn index_preserves_complete_label_identity_between_rounds() {
        let mut previous_samples = Vec::new();
        let mut current_samples = Vec::new();
        for (dimension, before, after) in [("a", 10.0, 30.0), ("b", 40.0, 90.0)] {
            let mut previous = sample(RECORD_ERRORS, "orders", "0", before);
            previous
                .labels
                .insert("dimension".to_owned(), dimension.to_owned());
            let mut current = previous.clone();
            current.value = after;
            previous_samples.push(previous);
            current_samples.push(current);
        }
        let previous = worker("w0", MetricState::Available, previous_samples);
        let current = worker("w0", MetricState::Available, current_samples);
        let report = FailuresReport::build(Some(&previous), &current, Some(interval()));
        let readings: Vec<_> = report
            .tasks
            .iter()
            .map(|row| (row.record_errors.count, row.record_errors.rate_per_sec))
            .collect();
        assert_eq!(readings, [(Some(30.0), Some(2.0)), (Some(90.0), Some(5.0))]);
    }
}
