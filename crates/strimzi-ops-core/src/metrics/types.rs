use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Whether real exporter measurements are usable; never a synthetic zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricState {
    Available,
    Unavailable,
    Stale,
    CollectionError,
}

impl fmt::Display for MetricState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::Stale => "stale",
            Self::CollectionError => "collection error",
        })
    }
}

/// An unaggregated measurement. Every label is part of its identity.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricSample {
    pub name: String,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

/// Lookup result: unavailable/error measurements have no value; stale real
/// measurements retain their last good value and must be displayed as stale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MetricLookup {
    pub state: MetricState,
    pub value: Option<f64>,
}

/// Public worker data contains neither endpoints nor authentication material.
#[derive(Debug, Clone)]
pub struct WorkerMetrics {
    pub worker: String,
    pub state: MetricState,
    pub samples: Arc<Vec<MetricSample>>,
    pub age_secs: Option<u64>,
    pub error: Option<String>,
}

impl WorkerMetrics {
    /// Match the complete label set, including connector and task when present.
    /// Missing and nonfinite samples are unavailable, not zero. A failed first
    /// scrape remains a collection error because absence could not be checked.
    pub fn lookup(&self, name: &str, labels: &BTreeMap<String, String>) -> MetricLookup {
        let sample = self
            .samples
            .iter()
            .find(|sample| sample.name == name && sample.labels == *labels);
        match sample {
            Some(sample)
                if sample.value.is_finite()
                    && matches!(self.state, MetricState::Available | MetricState::Stale) =>
            {
                MetricLookup {
                    state: self.state,
                    value: Some(sample.value),
                }
            }
            _ => MetricLookup {
                state: if self.state == MetricState::CollectionError {
                    MetricState::CollectionError
                } else {
                    MetricState::Unavailable
                },
                value: None,
            },
        }
    }
}

/// Independent workers and the most severe collection state. No values are
/// merged across workers, connectors, or tasks.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    pub workers: Vec<WorkerMetrics>,
    pub state: MetricState,
}

impl MetricsSnapshot {
    pub fn lookup(
        &self,
        worker: &str,
        name: &str,
        labels: &BTreeMap<String, String>,
    ) -> MetricLookup {
        self.workers
            .iter()
            .find(|metrics| metrics.worker == worker)
            .map_or(
                MetricLookup {
                    state: if self.workers.is_empty() && self.state == MetricState::CollectionError
                    {
                        MetricState::CollectionError
                    } else {
                        MetricState::Unavailable
                    },
                    value: None,
                },
                |metrics| metrics.lookup(name, labels),
            )
    }
}
