"""Control module for managing Kafka Connect connectors."""

import logging
from typing import Any

import requests

logger = logging.getLogger(__name__)


class ConnectorController:
    """Controller for Kafka Connect operations."""

    def __init__(self, connect_url: str):
        """
        Initialize the connector controller.

        Args:
            connect_url: Kafka Connect REST API URL
        """
        self.connect_url = connect_url.rstrip("/")
        self.session = requests.Session()
        self.session.headers.update(
            {"Content-Type": "application/json", "Accept": "application/json"}
        )

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
        self, connector_name: str, snapshot_type: str = "incremental"
    ) -> dict[str, Any]:
        """
        Trigger a snapshot for a Debezium connector.

        Args:
            connector_name: Name of the connector
            snapshot_type: Type of snapshot (incremental, blocking, etc.)

        Returns:
            Response from the API
        """
        # Debezium snapshot triggering via signal topic or REST API
        # This is a simplified example - actual implementation may vary
        endpoint = f"connectors/{connector_name}/tasks/0/restart"

        # For Debezium 2.0+, you can use the signal mechanism
        # This would typically involve sending a signal to a designated topic
        logger.info(f"Triggering {snapshot_type} snapshot for connector: {connector_name}")

        # Note: Actual snapshot triggering may require sending signals to Kafka topics
        # depending on Debezium version and configuration
        _ = self._make_request("POST", endpoint)

        return {"status": "snapshot_triggered", "connector": connector_name}

    def get_connector_plugins(self) -> list[dict[str, Any]]:
        """
        Get list of available connector plugins.

        Returns:
            List of connector plugin information
        """
        response = self._make_request("GET", "connector-plugins")
        result: list[dict[str, Any]] = response.json()
        return result

    def validate_connector_config(
        self, plugin_class: str, config: dict[str, Any]
    ) -> dict[str, Any]:
        """
        Validate connector configuration.

        Args:
            plugin_class: Connector plugin class name
            config: Configuration to validate

        Returns:
            Validation result
        """
        endpoint = f"connector-plugins/{plugin_class}/config/validate"
        response = self._make_request("PUT", endpoint, data=config)
        result: dict[str, Any] = response.json()
        return result
