use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use strimzi_ops_core::metrics::{
    MetricState, MetricsCollector, MetricsSnapshot, SinkLagCollector, SinkLagSnapshot,
    COLLECTION_INTERVAL,
};
use tokio::sync::watch;

/// One cache and collector per application, independent of browser connections.
pub struct MetricsCache {
    latest: watch::Sender<Arc<CachedMetrics>>,
    started: AtomicBool,
}

pub struct CachedMetrics {
    pub snapshot: Arc<MetricsSnapshot>,
    pub sink_lag: Arc<SinkLagSnapshot>,
    pub received: Instant,
    /// Exactly one preceding round; stale/error rounds break rate baselines.
    pub previous: Option<PreviousMetrics>,
}

pub struct PreviousMetrics {
    pub snapshot: Arc<MetricsSnapshot>,
    pub received: Instant,
}

impl MetricsCache {
    pub fn new() -> Arc<Self> {
        let (latest, _) = watch::channel(Arc::new(CachedMetrics {
            snapshot: Arc::new(MetricsSnapshot {
                workers: Vec::new(),
                state: MetricState::Unavailable,
            }),
            sink_lag: Arc::new(SinkLagSnapshot {
                state: MetricState::Unavailable,
                connectors: Arc::new(Vec::new()),
                age_secs: None,
                error: None,
            }),
            received: Instant::now(),
            previous: None,
        }));
        Arc::new(Self {
            latest,
            started: AtomicBool::new(false),
        })
    }

    pub fn latest(&self) -> Arc<CachedMetrics> {
        Arc::clone(&self.latest.borrow())
    }

    pub fn start(
        self: &Arc<Self>,
        path: Option<PathBuf>,
        connect_url: Option<String>,
        bootstrap_servers: Option<String>,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if path.is_none() && (connect_url.is_none() || bootstrap_servers.is_none()) {
            return None;
        }
        if self.started.swap(true, Ordering::AcqRel) {
            return None;
        }
        let cache = Arc::downgrade(self);
        Some(tokio::spawn(async move {
            let clients = tokio::task::spawn_blocking(move || {
                (
                    MetricsCollector::new(path),
                    SinkLagCollector::new(connect_url, bootstrap_servers),
                )
            })
            .await;
            let Ok((mut collector, mut lag_collector)) = clients else {
                return;
            };
            loop {
                // The blocking client is created, used, and dropped on a worker thread.
                let result = tokio::task::spawn_blocking(move || {
                    let snapshot = collector.collect();
                    let sink_lag = lag_collector.collect();
                    (collector, lag_collector, snapshot, sink_lag)
                })
                .await;
                let Ok((next_collector, next_lag_collector, snapshot, sink_lag)) = result else {
                    // The cached sample ages into stale if the worker terminates.
                    break;
                };
                collector = next_collector;
                lag_collector = next_lag_collector;
                let Some(cache) = cache.upgrade() else {
                    break;
                };
                let latest = cache.latest();
                let previous = PreviousMetrics {
                    snapshot: Arc::clone(&latest.snapshot),
                    received: latest.received,
                };
                drop(latest);
                cache.latest.send_replace(Arc::new(CachedMetrics {
                    snapshot: Arc::new(snapshot),
                    sink_lag: Arc::new(sink_lag),
                    received: Instant::now(),
                    previous: Some(previous),
                }));
                drop(cache);
                // Delay after collection: no overlap or catch-up bursts.
                tokio::time::sleep(COLLECTION_INTERVAL).await;
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_configuration_does_not_start_or_fake_zero_lag() {
        let cache = MetricsCache::new();
        for (connect, bootstrap) in [
            (None, None),
            (Some("http://localhost:8083".to_owned()), None),
            (None, Some("localhost:9092".to_owned())),
        ] {
            assert!(cache.start(None, connect, bootstrap).is_none());
        }
        assert!(!cache.started.load(Ordering::Acquire));
        let latest = cache.latest();
        assert_eq!(latest.snapshot.state, MetricState::Unavailable);
        assert_eq!(latest.sink_lag.state, MetricState::Unavailable);
        assert!(latest.sink_lag.connectors.is_empty());
        assert_eq!(latest.sink_lag.age_secs, None);
    }

    #[tokio::test]
    async fn configured_lag_starts_without_jmx_and_only_once() {
        let cache = MetricsCache::new();
        let handle = cache.start(
            None,
            Some("http://localhost:8083".to_owned()),
            Some("localhost:9092".to_owned()),
        );
        assert!(handle.is_some());
        assert!(cache
            .start(Some(PathBuf::from("unused")), None, None)
            .is_none());
        handle.unwrap().abort();
    }
}
