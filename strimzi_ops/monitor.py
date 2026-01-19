"""Monitor for real-time snapshot tracking via Debezium Notifications."""

from confluent_kafka import Consumer, KafkaError
from typing import Dict, Any, Optional, Callable
import json
import logging

logger = logging.getLogger(__name__)


class DebeziumNotificationMonitor:
    """Monitor for Debezium notifications."""

    def __init__(self, bootstrap_servers: str, notification_topic: str = "debezium.notifications"):
        """
        Initialize the notification monitor.

        Args:
            bootstrap_servers: Kafka bootstrap servers
            notification_topic: Topic name for Debezium notifications
        """
        self.bootstrap_servers = bootstrap_servers
        self.notification_topic = notification_topic
        self.consumer: Optional[Consumer] = None

    def start(self, group_id: str = "strimzi-ops-monitor") -> None:
        """
        Start consuming notifications.

        Args:
            group_id: Consumer group ID
        """
        config = {
            'bootstrap.servers': self.bootstrap_servers,
            'group.id': group_id,
            'auto.offset.reset': 'latest',
            'enable.auto.commit': True
        }

        self.consumer = Consumer(config)
        self.consumer.subscribe([self.notification_topic])
        logger.info(f"Started monitoring notifications on topic: {self.notification_topic}")

    def stop(self) -> None:
        """Stop consuming notifications."""
        if self.consumer:
            self.consumer.close()
            logger.info("Stopped notification monitor")

    def poll(self, timeout: float = 1.0) -> Optional[Dict[str, Any]]:
        """
        Poll for a single notification message.

        Args:
            timeout: Poll timeout in seconds

        Returns:
            Notification dictionary or None
        """
        if not self.consumer:
            raise RuntimeError("Monitor not started. Call start() first.")

        msg = self.consumer.poll(timeout)

        if msg is None:
            return None

        if msg.error():
            if msg.error().code() == KafkaError._PARTITION_EOF:
                logger.debug("Reached end of partition")
            else:
                logger.error(f"Consumer error: {msg.error()}")
            return None

        try:
            notification = json.loads(msg.value().decode('utf-8'))
            return notification
        except json.JSONDecodeError as e:
            logger.error(f"Failed to decode notification: {e}")
            return None

    def consume_notifications(
        self,
        callback: Callable[[Dict[str, Any]], None],
        duration_seconds: Optional[int] = None
    ) -> None:
        """
        Consume notifications and pass them to a callback function.

        Args:
            callback: Function to call with each notification
            duration_seconds: Optional duration to consume (None = infinite)
        """
        if not self.consumer:
            raise RuntimeError("Monitor not started. Call start() first.")

        import time
        start_time = time.time()

        try:
            while True:
                if duration_seconds and (time.time() - start_time) > duration_seconds:
                    break

                notification = self.poll()
                if notification:
                    callback(notification)

        except KeyboardInterrupt:
            logger.info("Interrupted by user")
        finally:
            self.stop()


class SnapshotTracker:
    """Track snapshot progress from Debezium notifications."""

    def __init__(self):
        """Initialize the snapshot tracker."""
        self.snapshots: Dict[str, Dict[str, Any]] = {}

    def process_notification(self, notification: Dict[str, Any]) -> None:
        """
        Process a Debezium notification and update snapshot state.

        Args:
            notification: Notification dictionary
        """
        notification_type = notification.get("type")
        connector = notification.get("aggregateType")

        if notification_type == "STARTED":
            self._handle_snapshot_started(connector, notification)
        elif notification_type == "IN_PROGRESS":
            self._handle_snapshot_progress(connector, notification)
        elif notification_type == "COMPLETED":
            self._handle_snapshot_completed(connector, notification)
        elif notification_type == "ABORTED":
            self._handle_snapshot_aborted(connector, notification)

    def _handle_snapshot_started(self, connector: str, notification: Dict[str, Any]) -> None:
        """Handle snapshot started notification."""
        logger.info(f"Snapshot started for connector: {connector}")
        self.snapshots[connector] = {
            "status": "STARTED",
            "start_time": notification.get("timestamp"),
            "progress": 0,
            "notification": notification
        }

    def _handle_snapshot_progress(self, connector: str, notification: Dict[str, Any]) -> None:
        """Handle snapshot progress notification."""
        if connector in self.snapshots:
            data = notification.get("data", {})
            self.snapshots[connector]["status"] = "IN_PROGRESS"
            self.snapshots[connector]["progress"] = data.get("progress", 0)
            self.snapshots[connector]["notification"] = notification
            logger.info(f"Snapshot progress for {connector}: {self.snapshots[connector]['progress']}%")

    def _handle_snapshot_completed(self, connector: str, notification: Dict[str, Any]) -> None:
        """Handle snapshot completed notification."""
        logger.info(f"Snapshot completed for connector: {connector}")
        if connector in self.snapshots:
            self.snapshots[connector]["status"] = "COMPLETED"
            self.snapshots[connector]["progress"] = 100
            self.snapshots[connector]["end_time"] = notification.get("timestamp")
            self.snapshots[connector]["notification"] = notification

    def _handle_snapshot_aborted(self, connector: str, notification: Dict[str, Any]) -> None:
        """Handle snapshot aborted notification."""
        logger.warning(f"Snapshot aborted for connector: {connector}")
        if connector in self.snapshots:
            self.snapshots[connector]["status"] = "ABORTED"
            self.snapshots[connector]["end_time"] = notification.get("timestamp")
            self.snapshots[connector]["notification"] = notification

    def get_snapshot_status(self, connector: str) -> Optional[Dict[str, Any]]:
        """
        Get snapshot status for a connector.

        Args:
            connector: Connector name

        Returns:
            Snapshot status dictionary or None
        """
        return self.snapshots.get(connector)

    def get_all_snapshots(self) -> Dict[str, Dict[str, Any]]:
        """
        Get all snapshot statuses.

        Returns:
            Dictionary of all snapshots
        """
        return self.snapshots
