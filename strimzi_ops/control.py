"""Control module for managing Kafka Connect connectors."""

import json
import logging
import uuid
from typing import Any

import requests
from confluent_kafka import Producer

logger = logging.getLogger(__name__)


class ConnectorController:
    """Controller for Kafka Connect operations."""

    def __init__(self, connect_url: str, bootstrap_servers: str | None = None):
        """
        Initialize the connector controller.

        Args:
            connect_url: Kafka Connect REST API URL
            bootstrap_servers: Optional Kafka bootstrap servers for signaling
        """
        self.connect_url = connect_url.rstrip("/")
        self.bootstrap_servers = bootstrap_servers
        self.session = requests.Session()
        self.session.headers.update(
            {"Content-Type": "application/json", "Accept": "application/json"}
        )
        self._producer: Producer | None = None

    def _get_producer(self) -> Producer:
        """Get or create Kafka producer for signaling."""
        if not self.bootstrap_servers:
            raise ValueError("Bootstrap servers not configured. Cannot send signals.")

        if self._producer is None:
            self._producer = Producer({"bootstrap.servers": self.bootstrap_servers})
        return self._producer

    def _make_request(
        self, method: str, endpoint: str, data: dict[str, Any] | None = None
    ) -> requests.Response:
        """
        Make a request to Kafka Connect REST API.

        Args:
            method: HTTP method (GET, POST, PUT, DELETE)
            endpoint: API endpoint
            data: Optional request data

        Returns:
            Response object
        """
        url = f"{self.connect_url}/{endpoint}"
        logger.debug(f"{method} {url}")

        try:
            if method == "GET":
                response = self.session.get(url)
            elif method == "POST":
                response = self.session.post(url, json=data)
            elif method == "PUT":
                response = self.session.put(url, json=data)
            elif method == "DELETE":
                response = self.session.delete(url)
            else:
                raise ValueError(f"Unsupported HTTP method: {method}")

            response.raise_for_status()
            return response

        except requests.exceptions.RequestException as e:
            logger.error(f"Request failed: {e}")
            raise

    def list_connectors(self) -> list[str]:
        """
        List all connectors.

        Returns:
            List of connector names
        """
        response = self._make_request("GET", "connectors")
        result: list[str] = response.json()
        return result

    def get_all_connectors_status(self) -> dict[str, dict[str, Any]]:
        """
        Get status and info for all connectors.

        Returns:
            Dictionary mapping connector name to its status and info
        """
        response = self._make_request("GET", "connectors?expand=status&expand=info")
        result: dict[str, dict[str, Any]] = response.json()
        return result

    def get_connector_info(self, connector_name: str) -> dict[str, Any]:
        """
        Get connector information.

        Args:
            connector_name: Name of the connector

        Returns:
            Connector information dictionary
        """
        response = self._make_request("GET", f"connectors/{connector_name}")
        result: dict[str, Any] = response.json()
        return result

    def get_connector_status(self, connector_name: str) -> dict[str, Any]:
        """
        Get connector status.

        Args:
            connector_name: Name of the connector

        Returns:
            Connector status dictionary
        """
        response = self._make_request("GET", f"connectors/{connector_name}/status")
        result: dict[str, Any] = response.json()
        return result

    def get_connector_config(self, connector_name: str) -> dict[str, Any]:
        """
        Get connector configuration.

        Args:
            connector_name: Name of the connector

        Returns:
            Connector configuration dictionary
        """
        response = self._make_request("GET", f"connectors/{connector_name}/config")
        result: dict[str, Any] = response.json()
        return result

    def create_connector(self, config: dict[str, Any]) -> dict[str, Any]:
        """
        Create a new connector.

        Args:
            config: Connector configuration

        Returns:
            Created connector information
        """
        response = self._make_request("POST", "connectors", data=config)
        logger.info(f"Created connector: {config.get('name')}")
        result: dict[str, Any] = response.json()
        return result

    def update_connector(self, connector_name: str, config: dict[str, Any]) -> dict[str, Any]:
        """
        Update connector configuration.

        Args:
            connector_name: Name of the connector
            config: New connector configuration

        Returns:
            Updated connector information
        """
        response = self._make_request("PUT", f"connectors/{connector_name}/config", data=config)
        logger.info(f"Updated connector: {connector_name}")
        result: dict[str, Any] = response.json()
        return result

    def delete_connector(self, connector_name: str) -> None:
        """
        Delete a connector.

        Args:
            connector_name: Name of the connector
        """
        self._make_request("DELETE", f"connectors/{connector_name}")
        logger.info(f"Deleted connector: {connector_name}")

    def pause_connector(self, connector_name: str) -> None:
        """
        Pause a connector.

        Args:
            connector_name: Name of the connector
        """
        self._make_request("PUT", f"connectors/{connector_name}/pause")
        logger.info(f"Paused connector: {connector_name}")

    def resume_connector(self, connector_name: str) -> None:
        """
        Resume a connector.

        Args:
            connector_name: Name of the connector
        """
        self._make_request("PUT", f"connectors/{connector_name}/resume")
        logger.info(f"Resumed connector: {connector_name}")

    def restart_connector(self, connector_name: str) -> None:
        """
        Restart a connector.

        Args:
            connector_name: Name of the connector
        """
        self._make_request("POST", f"connectors/{connector_name}/restart")
        logger.info(f"Restarted connector: {connector_name}")

    def restart_connector_task(self, connector_name: str, task_id: int) -> None:
        """
        Restart a specific connector task.

        Args:
            connector_name: Name of the connector
            task_id: Task ID
        """
        self._make_request("POST", f"connectors/{connector_name}/tasks/{task_id}/restart")
        logger.info(f"Restarted task {task_id} for connector: {connector_name}")

    def trigger_snapshot(
        self,
        connector_name: str,
        snapshot_type: str = "incremental",
        tables: list[str] | None = None,
    ) -> dict[str, Any]:
        """
        Trigger a snapshot for a Debezium connector via signaling topic.

        Args:
            connector_name: Name of the connector
            snapshot_type: Type of snapshot (incremental, blocking)
            tables: Optional list of tables to include (fully qualified names)

        Returns:
            Status of the snapshot trigger
        """
        config = self.get_connector_config(connector_name)

        # 1. Identify signal topic
        signal_topic = config.get("signal.kafka.topic", "debezium.signals")

        # 2. Construct Debezium signal
        signal_id = str(uuid.uuid4())
        signal_payload = {
            "id": signal_id,
            "type": "execute-snapshot",
            "data": {
                "data-collections": tables or ["*"],
                "type": snapshot_type,
            },
        }

        # 3. Send signal to Kafka
        try:
            producer = self._get_producer()
            producer.produce(
                signal_topic,
                key=connector_name.encode("utf-8"),
                value=json.dumps(signal_payload).encode("utf-8"),
            )
            producer.flush()

            logger.info(
                f"Triggered {snapshot_type} snapshot for {connector_name} (ID: {signal_id})"
            )
            return {
                "status": "success",
                "message": f"Signal sent to {signal_topic}",
                "signal_id": signal_id,
            }
        except Exception as e:
            logger.error(f"Failed to trigger snapshot: {e}")
            # Fallback to legacy method if producer is not available or fails
            logger.info("Falling back to legacy task restart method")
            self.restart_connector_task(connector_name, 0)
            return {
                "status": "fallback",
                "message": f"Kafka signal failed ({e}). Restarted Task 0 as fallback.",
            }

    def get_cluster_info(self) -> dict[str, Any]:
        """
        Get Kafka Connect cluster information.

        Returns:
            Cluster info dictionary (version, commit, kafka_cluster_id, etc.)
        """
        response = self._make_request("GET", "")
        result: dict[str, Any] = response.json()
        return result

    def get_connector_plugins(self) -> list[dict[str, Any]]:
        """
        Get list of available connector plugins.

        Returns:
            List of connector plugin information
        """
        response = self._make_request("GET", "connector-plugins")
        result: list[dict[str, Any]] = response.json()
        return result

    def to_strimzi_yaml(self, connector_name: str, cluster_name: str = "my-connect-cluster") -> str:
        """
        Generate Strimzi KafkaConnector YAML for a connector.

        Args:
            connector_name: Name of the connector
            cluster_name: Name of the Strimzi KafkaConnect cluster

        Returns:
            YAML string
        """
        config = dict(self.get_connector_config(connector_name))
        connector_class = config.pop("connector.class", "unknown")
        tasks_max = int(config.pop("tasks.max", "1"))

        yaml_lines = [
            "apiVersion: kafka.strimzi.io/v1beta2",
            "kind: KafkaConnector",
            "metadata:",
            f"  name: {connector_name}",
            "  labels:",
            f"    strimzi.io/cluster: {cluster_name}",
            "spec:",
            f"  class: {connector_class}",
            f"  tasksMax: {tasks_max}",
            "  config:",
        ]

        # Add all other config fields
        for key, value in sorted(config.items()):
            # Handle string vs other types (simple conversion)
            if isinstance(value, str):
                yaml_lines.append(f'    {key}: "{value}"')
            else:
                yaml_lines.append(f"    {key}: {value}")

        return "\n".join(yaml_lines)
