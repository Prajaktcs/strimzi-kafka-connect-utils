"""Pydantic models for Kafka Connect connector configurations."""

from pydantic import BaseModel, ConfigDict, Field, field_validator


class BaseConnectorConfig(BaseModel):
    """Base configuration for all Kafka Connect connectors."""

    model_config = ConfigDict(populate_by_name=True, extra="allow")

    name: str
    connector_class: str = Field(..., alias="connector.class")
    tasks_max: int = Field(1, alias="tasks.max", ge=1)


class DebeziumPostgresConfig(BaseConnectorConfig):
    """Configuration for Debezium PostgreSQL source connector."""

    database_hostname: str = Field(..., alias="database.hostname")
    database_port: int = Field(5432, alias="database.port")
    database_user: str = Field(..., alias="database.user")
    database_password: str = Field(..., alias="database.password")
    database_dbname: str = Field(..., alias="database.dbname")
    topic_prefix: str = Field(..., alias="topic.prefix")
    plugin_name: str = Field("pgoutput", alias="plugin.name")
    slot_name: str | None = Field(None, alias="slot.name")
    publication_name: str | None = Field(None, alias="publication.name")
    snapshot_mode: str = Field("initial", alias="snapshot.mode")

    @field_validator("snapshot_mode")
    @classmethod
    def validate_snapshot_mode(cls, v: str) -> str:
        valid_modes = ["initial", "always", "never", "when_needed", "schema_only"]
        if v not in valid_modes:
            raise ValueError(f"snapshot.mode must be one of {valid_modes}")
        return v


class IcebergSinkConfig(BaseConnectorConfig):
    """Configuration for Iceberg sink connector."""

    topics: str
    iceberg_catalog_type: str = Field("hadoop", alias="iceberg.catalog.type")
    iceberg_catalog_warehouse: str = Field(..., alias="iceberg.catalog.warehouse")
    iceberg_catalog_s3_endpoint: str | None = Field(None, alias="iceberg.catalog.s3.endpoint")
    iceberg_catalog_s3_access_key_id: str | None = Field(
        None, alias="iceberg.catalog.s3.access-key-id"
    )
    iceberg_catalog_s3_secret_access_key: str | None = Field(
        None, alias="iceberg.catalog.s3.secret-access-key"
    )
    iceberg_catalog_s3_path_style_access: bool = Field(
        True, alias="iceberg.catalog.s3.path-style-access"
    )


def get_model_for_class(connector_class: str) -> type[BaseConnectorConfig]:
    """Get the appropriate Pydantic model for a connector class."""
    if "PostgresConnector" in connector_class:
        return DebeziumPostgresConfig
    if "IcebergSinkConnector" in connector_class:
        return IcebergSinkConfig
    return BaseConnectorConfig
