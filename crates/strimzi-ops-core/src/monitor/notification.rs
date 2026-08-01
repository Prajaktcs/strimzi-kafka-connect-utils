use serde_json::Value;
use std::time::{Duration, Instant};

use crate::{Error, Result};

/// Consumes Debezium notification messages from a Kafka topic.
pub struct NotificationMonitor {
    bootstrap_servers: String,
    notification_topic: String,
    #[cfg(feature = "kafka")]
    consumer: Option<rdkafka::consumer::BaseConsumer>,
}

impl std::fmt::Debug for NotificationMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotificationMonitor")
            .field("bootstrap_servers", &self.bootstrap_servers)
            .field("notification_topic", &self.notification_topic)
            .field("started", &self.is_started())
            .finish_non_exhaustive()
    }
}

impl NotificationMonitor {
    pub fn new(
        bootstrap_servers: impl Into<String>,
        notification_topic: impl Into<String>,
    ) -> Self {
        Self {
            bootstrap_servers: bootstrap_servers.into(),
            notification_topic: notification_topic.into(),
            #[cfg(feature = "kafka")]
            consumer: None,
        }
    }

    pub fn notification_topic(&self) -> &str {
        &self.notification_topic
    }

    /// Start consuming notifications with the given consumer group.
    pub fn start(&mut self, group_id: &str) -> Result<()> {
        self.start_inner(group_id)
    }

    /// Stop consuming notifications.
    pub fn stop(&mut self) {
        self.stop_inner();
    }

    /// Poll for a single notification message.
    pub fn poll(&self, timeout: Duration) -> Result<Option<Value>> {
        self.poll_inner(timeout)
    }

    /// Consume notifications until `duration` elapses (or forever if `None`),
    /// invoking `callback` for each decoded message.
    pub fn consume_notifications<F>(
        &mut self,
        mut callback: F,
        duration: Option<Duration>,
    ) -> Result<()>
    where
        F: FnMut(Value),
    {
        if !self.is_started() {
            return Err(Error::MonitorNotStarted);
        }

        let started = Instant::now();
        loop {
            if let Some(limit) = duration {
                if started.elapsed() > limit {
                    break;
                }
            }

            if let Some(notification) = self.poll(Duration::from_secs(1))? {
                callback(notification);
            }
        }

        self.stop();
        Ok(())
    }

    #[cfg(feature = "kafka")]
    fn start_inner(&mut self, group_id: &str) -> Result<()> {
        use rdkafka::config::ClientConfig;
        use rdkafka::consumer::{BaseConsumer, Consumer};

        let consumer: BaseConsumer = ClientConfig::new()
            .set("bootstrap.servers", &self.bootstrap_servers)
            .set("group.id", group_id)
            .set("auto.offset.reset", "latest")
            .set("enable.auto.commit", "true")
            .create()
            .map_err(|source| Error::Kafka {
                reason: format!("cannot create consumer: {source}"),
            })?;
        consumer
            .subscribe(&[self.notification_topic.as_str()])
            .map_err(|source| Error::Kafka {
                reason: format!("cannot subscribe to {}: {source}", self.notification_topic),
            })?;
        self.consumer = Some(consumer);
        Ok(())
    }

    #[cfg(not(feature = "kafka"))]
    fn start_inner(&mut self, _group_id: &str) -> Result<()> {
        let _ = &self.bootstrap_servers;
        Err(Error::KafkaFeatureDisabled)
    }

    #[cfg(feature = "kafka")]
    fn stop_inner(&mut self) {
        if let Some(consumer) = self.consumer.take() {
            drop(consumer);
        }
    }

    #[cfg(not(feature = "kafka"))]
    fn stop_inner(&mut self) {}

    #[cfg(feature = "kafka")]
    fn is_started(&self) -> bool {
        self.consumer.is_some()
    }

    #[cfg(not(feature = "kafka"))]
    fn is_started(&self) -> bool {
        false
    }

    #[cfg(feature = "kafka")]
    fn poll_inner(&self, timeout: Duration) -> Result<Option<Value>> {
        use rdkafka::message::Message;

        let Some(consumer) = self.consumer.as_ref() else {
            return Err(Error::MonitorNotStarted);
        };

        match consumer.poll(timeout) {
            None => Ok(None),
            Some(Err(source)) => Err(Error::Kafka {
                reason: format!("consumer poll failed: {source}"),
            }),
            Some(Ok(message)) => {
                let Some(payload) = message.payload() else {
                    return Ok(None);
                };
                match serde_json::from_slice::<Value>(payload) {
                    Ok(value) => Ok(Some(value)),
                    Err(_) => Ok(None),
                }
            }
        }
    }

    #[cfg(not(feature = "kafka"))]
    fn poll_inner(&self, _timeout: Duration) -> Result<Option<Value>> {
        Err(Error::MonitorNotStarted)
    }
}
