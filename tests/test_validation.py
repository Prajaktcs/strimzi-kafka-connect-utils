"""Tests for connector configuration validation."""

import pytest

from strimzi_ops.validator import ConnectorValidator


@pytest.fixture
def validator():
    """Initialize a validator for testing."""
    return ConnectorValidator()


def test_valid_postgres_connector(validator):
    """Test a valid PostgreSQL connector configuration."""
    config = {
        "name": "test-postgres",
        "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        "tasks.max": "1",
        "database.hostname": "localhost",
        "database.port": "5432",
        "database.user": "postgres",
        "database.password": "password",
        "database.dbname": "test_db",
        "topic.prefix": "test_prefix",
    }
    result = validator.validate_config(config)
    assert result["valid"] is True
    assert result["summary"]["errors"] == 0


def test_invalid_postgres_connector_missing_fields(validator):
    """Test an invalid PostgreSQL connector with missing fields."""
    config = {
        "name": "test-postgres",
        "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        # Missing database.hostname and other required fields
    }
    result = validator.validate_config(config)
    assert result["valid"] is False
    assert result["summary"]["errors"] > 0
    # Check if Pydantic error is captured
    pydantic_errors = [r for r in result["results"] if r.rule_id == "pydantic-schema"]
    assert len(pydantic_errors) > 0


def test_invalid_postgres_snapshot_mode(validator):
    """Test invalid snapshot.mode value."""
    config = {
        "name": "test-postgres",
        "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        "tasks.max": "1",
        "database.hostname": "localhost",
        "database.user": "postgres",
        "database.password": "password",
        "database.dbname": "test_db",
        "topic.prefix": "test_prefix",
        "snapshot.mode": "invalid_mode",
    }
    result = validator.validate_config(config)
    assert result["valid"] is False
    pydantic_errors = [r for r in result["results"] if r.rule_id == "pydantic-schema"]
    assert any("snapshot.mode" in e.message for e in pydantic_errors)


def test_valid_iceberg_connector(validator):
    """Test a valid Iceberg sink connector."""
    config = {
        "name": "test-iceberg",
        "connector.class": "org.apache.iceberg.connect.IcebergSinkConnector",
        "topics": "topic1,topic2",
        "iceberg.catalog.warehouse": "s3://bucket/warehouse",
    }
    result = validator.validate_config(config)
    assert result["valid"] is True


def test_invalid_tasks_max(validator):
    """Test invalid tasks.max value (must be >= 1)."""
    config = {
        "name": "test-postgres",
        "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        "tasks.max": "0",  # Invalid, must be at least 1
    }
    result = validator.validate_config(config)
    assert result["valid"] is False
    pydantic_errors = [r for r in result["results"] if r.rule_id == "pydantic-schema"]
    assert len(pydantic_errors) > 0


def test_connect_api_config_without_name(validator):
    """Connect REST GET .../config omits name; inject via connector_name."""
    config = {
        "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
        "tasks.max": "1",
        "database.hostname": "localhost",
        "database.user": "postgres",
        "database.password": "password",
        "database.dbname": "test_db",
        "topic.prefix": "test_prefix",
    }
    missing_name = validator.validate_config(config)
    assert missing_name["valid"] is False

    with_name = validator.validate_config(config, connector_name="test-postgres")
    assert with_name["valid"] is True
