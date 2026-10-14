use super::{MetricLookup, MetricSample, MetricState, WorkerMetrics};

/// Track a projected measurement without choosing a winner among duplicates.
#[derive(Clone, Copy, Default)]
pub(super) enum Series<'a> {
    #[default]
    Missing,
    One(&'a MetricSample),
    Duplicate,
}

impl<'a> Series<'a> {
    pub(super) fn observe(&mut self, sample: &'a MetricSample) {
        *self = match self {
            Self::Missing => Self::One(sample),
            Self::One(_) | Self::Duplicate => Self::Duplicate,
        };
    }

    /// Duplicates remain unavailable even on failed workers. Otherwise a
    /// collection error takes precedence over missing or invalid samples.
    pub(super) fn lookup(
        self,
        worker: &WorkerMetrics,
        boolean: bool,
        label_count: usize,
    ) -> MetricLookup {
        if matches!(self, Self::Duplicate) {
            return absent(MetricState::Unavailable);
        }
        if worker.state == MetricState::CollectionError {
            return absent(MetricState::CollectionError);
        }
        match self {
            Self::One(sample)
                if sample.labels.len() == label_count
                    && sample.value.is_finite()
                    && sample.value >= 0.0
                    && (!boolean || matches!(sample.value, 0.0 | 1.0))
                    && matches!(worker.state, MetricState::Available | MetricState::Stale) =>
            {
                MetricLookup {
                    state: worker.state,
                    value: Some(sample.value),
                }
            }
            _ => absent(MetricState::Unavailable),
        }
    }
}

fn absent(state: MetricState) -> MetricLookup {
    MetricLookup { state, value: None }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;

    #[test]
    fn duplicates_override_collection_errors_without_exposing_values() {
        let worker = WorkerMetrics {
            worker: "worker-a".to_owned(),
            state: MetricState::CollectionError,
            samples: Arc::new(vec![MetricSample {
                name: "measurement".to_owned(),
                labels: BTreeMap::new(),
                value: f64::NAN,
            }]),
            age_secs: None,
            error: Some("cannot fetch metrics".to_owned()),
        };
        let mut series = Series::default();
        assert_eq!(
            series.lookup(&worker, true, 4),
            absent(MetricState::CollectionError)
        );
        series.observe(&worker.samples[0]);
        assert_eq!(
            series.lookup(&worker, true, 4),
            absent(MetricState::CollectionError)
        );
        for _ in 0..2 {
            series.observe(&worker.samples[0]);
            assert_eq!(
                series.lookup(&worker, true, 4),
                absent(MetricState::Unavailable)
            );
        }
    }
}
