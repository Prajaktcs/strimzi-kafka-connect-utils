use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use strimzi_ops_core::metrics::{
    MetricState, MetricsCollector, MetricsSnapshot, COLLECTION_INTERVAL,
};
use tokio::sync::watch;

/// One cache and collector per application, independent of browser connections.
pub struct MetricsCache {
    latest: watch::Sender<Arc<CachedMetrics>>,
    started: AtomicBool,
}

pub struct CachedMetrics {
    pub snapshot: Arc<MetricsSnapshot>,
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

    pub fn start(self: &Arc<Self>, path: Option<PathBuf>) -> Option<tokio::task::JoinHandle<()>> {
        let path = path?;
        if self.started.swap(true, Ordering::AcqRel) {
            return None;
        }
        let cache = Arc::downgrade(self);
        Some(tokio::spawn(async move {
            let mut collector = MetricsCollector::new(Some(path));
            loop {
                // The blocking client is created, used, and dropped on a worker thread.
                let result = tokio::task::spawn_blocking(move || {
                    let snapshot = collector.collect();
                    (collector, snapshot)
                })
                .await;
                let Ok((next_collector, snapshot)) = result else {
                    // The cached sample ages into stale if the worker terminates.
                    break;
                };
                collector = next_collector;
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
