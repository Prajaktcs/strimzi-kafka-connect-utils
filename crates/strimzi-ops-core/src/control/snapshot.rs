use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::connect::ConnectClient;
use crate::{Error, Result};

/// Outcome of attempting to trigger a Debezium snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotResult {
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal_id: Option<String>,
}

/// Triggers Debezium snapshots via Kafka signaling (with Connect task-restart fallback).
#[derive(Debug)]
pub struct SnapshotTrigger {
    client: ConnectClient,
    bootstrap_servers: Option<String>,
}

impl SnapshotTrigger {
    pub fn new(client: ConnectClient, bootstrap_servers: Option<String>) -> Self {
        Self {
            client,
            bootstrap_servers,
        }
    }

    /// Trigger a snapshot for `connector_name`.
    ///
    /// Primary path: produce an `execute-snapshot` signal to the connector's signal topic.
    /// Fallback: restart task 0 via the Connect REST API.
    pub fn trigger(
        &self,
        connector_name: &str,
        snapshot_type: &str,
        tables: Option<&[String]>,
    ) -> Result<SnapshotResult> {
        let config = self.client.get_connector_config(connector_name)?;
        let signal_topic = signal_topic_from_config(&config);

        match self.try_send_signal(connector_name, &signal_topic, snapshot_type, tables) {
            Ok(result) => Ok(result),
            Err(signal_err) => {
                self.client.restart_connector_task(connector_name, 0)?;
                Ok(SnapshotResult {
                    status: "fallback".to_owned(),
                    message: format!(
                        "Kafka signal failed ({signal_err}). Restarted Task 0 as fallback."
                    ),
                    signal_id: None,
                })
            }
        }
    }

    #[cfg(feature = "kafka")]
    fn try_send_signal(
        &self,
        connector_name: &str,
        signal_topic: &str,
        snapshot_type: &str,
        tables: Option<&[String]>,
    ) -> Result<SnapshotResult> {
        use rdkafka::config::ClientConfig;
        use rdkafka::producer::{BaseProducer, BaseRecord, Producer};
        use std::time::Duration;

        let bootstrap = self
            .bootstrap_servers
            .as_deref()
            .ok_or(Error::BootstrapServersRequired)?;

        let signal_id = uuid::Uuid::new_v4().to_string();
        let collections: Vec<Value> = match tables {
            Some(items) if !items.is_empty() => items
                .iter()
                .map(|item| Value::String(item.clone()))
                .collect(),
            _ => vec![Value::String("*".to_owned())],
        };
        let payload = serde_json::json!({
            "id": signal_id,
            "type": "execute-snapshot",
            "data": {
                "data-collections": collections,
                "type": snapshot_type,
            },
        });
        let value = serde_json::to_vec(&payload).map_err(|source| Error::Kafka {
            reason: format!("cannot serialise signal payload: {source}"),
        })?;

        let producer: BaseProducer = ClientConfig::new()
            .set("bootstrap.servers", bootstrap)
            .set("message.timeout.ms", "5000")
            .create()
            .map_err(|source| Error::Kafka {
                reason: format!("cannot create producer: {source}"),
            })?;

        producer
            .send(
                BaseRecord::to(signal_topic)
                    .key(connector_name)
                    .payload(&value),
            )
            .map_err(|(source, _)| Error::Kafka {
                reason: format!("cannot produce signal: {source}"),
            })?;
        producer
            .flush(Duration::from_secs(5))
            .map_err(|source| Error::Kafka {
                reason: format!("cannot flush producer: {source}"),
            })?;

        Ok(SnapshotResult {
            status: "success".to_owned(),
            message: format!("Signal sent to {signal_topic}"),
            signal_id: Some(signal_id),
        })
    }

    #[cfg(not(feature = "kafka"))]
    fn try_send_signal(
        &self,
        _connector_name: &str,
        _signal_topic: &str,
        _snapshot_type: &str,
        _tables: Option<&[String]>,
    ) -> Result<SnapshotResult> {
        let _ = &self.bootstrap_servers;
        Err(Error::KafkaFeatureDisabled)
    }
}

fn signal_topic_from_config(config: &Map<String, Value>) -> String {
    config
        .get("signal.kafka.topic")
        .and_then(Value::as_str)
        .unwrap_or("debezium.signals")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use httpmock::prelude::*;
    use serde_json::json;

    use super::*;

    #[test]
    fn fallback_restarts_task_when_kafka_unavailable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/connectors/demo/config");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!({
                    "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
                    "signal.kafka.topic": "signals"
                }));
        });
        let restart = server.mock(|when, then| {
            when.method(POST).path("/connectors/demo/tasks/0/restart");
            then.status(204);
        });

        let client = ConnectClient::new(&server.base_url()).expect("client");
        // No bootstrap servers → signal path fails → task restart fallback.
        let trigger = SnapshotTrigger::new(client, None);
        let result = trigger
            .trigger("demo", "incremental", None)
            .expect("trigger");
        assert_eq!(result.status, "fallback");
        assert!(result.message.contains("Restarted Task 0"));
        restart.assert();
    }
}
