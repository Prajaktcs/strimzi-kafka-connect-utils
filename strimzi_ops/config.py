"""Configuration management for Strimzi Ops."""

import tomli
from pathlib import Path
from typing import Any


class Config:
    """Configuration manager for Strimzi Ops."""

    def __init__(self, config_path: str = "secrets.toml"):
        """
        Initialize configuration from TOML file.

        Args:
            config_path: Path to the secrets.toml file
        """
        self.config_path = Path(config_path)
        self._config = self._load_config()

    def _load_config(self) -> dict[str, Any]:
        """Load configuration from TOML file."""
        if not self.config_path.exists():
            raise FileNotFoundError(f"Config file not found: {self.config_path}")

        with open(self.config_path, "rb") as f:
            return tomli.load(f)

    @property
    def kafka_bootstrap_servers(self) -> str:
        """Get Kafka bootstrap servers."""
        return self._config["kafka"]["bootstrap_servers"]

    @property
    def kafka_connect_url(self) -> str:
        """Get Kafka Connect REST API URL."""
        return self._config["kafka"]["connect_url"]

    @property
    def storage_endpoint(self) -> str:
        """Get S3 storage endpoint URL."""
        return self._config["storage"]["endpoint_url"]

    @property
    def storage_access_key(self) -> str:
        """Get S3 access key."""
        return self._config["storage"]["access_key"]

    @property
    def storage_secret_key(self) -> str:
        """Get S3 secret key."""
        return self._config["storage"]["secret_key"]

    @property
    def storage_bucket(self) -> str:
        """Get S3 bucket name."""
        return self._config["storage"]["bucket"]
