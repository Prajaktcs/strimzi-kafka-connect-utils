# Strimzi Ops Platform

A unified platform to validate, monitor, and control Kafka Connect deployments.

## Overview

Strimzi Ops is a comprehensive management platform for Kafka Connect, providing three core features:

- **Linter**: Flexible validation of connector configurations with configurable rules
- **Monitor**: Real-time snapshot tracking via Debezium Notifications
- **Control**: Manage connectors - restart/pause/resume and trigger snapshots

Whether you're building a data lakehouse with Debezium and Iceberg, streaming changes with JDBC connectors, or any other Kafka Connect use case - Strimzi Ops provides the tools you need to manage your connectors effectively.

## Architecture

### Infrastructure Stack

- **Broker**: Redpanda v25.3.4 (Kafka API compatible, C++)
- **Object Storage**: Garage v2.1.0 (S3 compatible, Rust)
- **Database**: PostgreSQL with Debezium (Source & Sink)
- **Processing**: Kafka Connect with Debezium 3.4
- **App Logic**: Python 3.12+ (Streamlit + Confluent Kafka + Pydantic)

## Prerequisites

- Docker and Docker Compose
- Python 3.12 or higher
- [uv](https://github.com/astral-sh/uv) - Fast Python package manager
- Git

### Installing UV

UV is a modern, fast Python package manager created by Astral (makers of Ruff). It offers:

- 10-100x faster than pip
- Automatic virtual environment management
- Lockfile support for reproducible builds
- Drop-in replacement for pip, pip-tools, and virtualenv

Install it with:

```bash
# macOS/Linux
curl -LsSf https://astral.sh/uv/install.sh | sh

# Or via pip
pip install uv

# Or via Homebrew
brew install uv
```

## Quick Start

### Automated Setup (Recommended)

```bash
# Clone the repository
git clone <repository-url>
cd strimzi-ops

# Run the setup script
./setup.sh
```

The setup script will:
1. Start all infrastructure services
2. Install uv if not present
3. Install Python dependencies
4. Display Garage access keys

### Manual Setup

### 1. Clone the Repository

```bash
git clone <repository-url>
cd strimzi-ops
```

### 2. Start Infrastructure

```bash
# Using docker-compose
docker-compose up -d

# Or using Make
make start
```

This will start:
- Redpanda (Kafka broker) on port 9092
- Garage (S3-compatible storage) on port 3900
- PostgreSQL (source database) on port 5432
- Kafka Connect on port 8083

### 3. Configure Garage Keys

After starting the services, retrieve the Garage access keys from the setup logs:

```bash
docker-compose logs setup-garage
```

Look for the output containing the access key and secret key, then update `secrets.toml`:

```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"

[storage]
type = "s3"
endpoint_url = "http://localhost:3900"
access_key = "YOUR_GARAGE_ACCESS_KEY"
secret_key = "YOUR_GARAGE_SECRET_KEY"
bucket = "warehouse"
```

### 4. Install Python Dependencies

```bash
# Using uv (recommended)
uv sync

# Or using Make
make install
```

### 5. Start the Application

```bash
# Using uv
uv run streamlit run app.py

# Or using Make
make run
```

The application will be available at `http://localhost:8501`

## Usage

### Dashboard

View an overview of all connectors, their status, and health metrics.

### Validator

Validate your connector configurations before deploying them:

1. Navigate to the **Validator** page
2. Paste your JSON configuration or upload a file
3. Click **Validate Configuration**

Supported connector types:
- Debezium PostgreSQL Connector
- Iceberg Sink Connector
- JDBC Sink Connector

### Monitor

Track snapshot progress in real-time:

1. Navigate to the **Monitor** page
2. Configure the notification topic (default: `debezium.notifications`)
3. Set monitoring duration
4. Click **Start Monitoring**

The monitor will display real-time snapshot progress for all active connectors.

### Control

Manage your connectors:

1. Navigate to the **Control** page
2. Select a connector from the dropdown
3. Available actions:
   - **Resume**: Resume a paused connector
   - **Pause**: Pause a running connector
   - **Restart**: Restart a connector
   - **Trigger Snapshot**: Initiate a new snapshot
4. Edit configuration and update as needed

## Configuration Examples

### Debezium PostgreSQL Connector

```json
{
  "name": "postgres-source-connector",
  "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
  "tasks.max": 1,
  "database.hostname": "postgres-source",
  "database.port": 5432,
  "database.user": "postgres",
  "database.password": "password",
  "database.dbname": "source_db",
  "topic.prefix": "lakehouse",
  "plugin.name": "pgoutput",
  "slot.name": "debezium_slot",
  "publication.name": "debezium_publication",
  "schema.history.internal.kafka.bootstrap.servers": "redpanda:29092",
  "schema.history.internal.kafka.topic": "schema-history.lakehouse",
  "snapshot.mode": "initial",
  "notification.enabled.channels": "sink",
  "notification.sink.topic.name": "debezium.notifications"
}
```

### Iceberg Sink Connector

```json
{
  "name": "iceberg-sink-connector",
  "connector.class": "org.apache.iceberg.connect.IcebergSinkConnector",
  "tasks.max": 1,
  "topics": "lakehouse.public.users,lakehouse.public.orders",
  "iceberg.catalog.type": "hadoop",
  "iceberg.catalog.warehouse": "s3a://warehouse/iceberg",
  "iceberg.catalog.s3.endpoint": "http://garage:3900",
  "iceberg.catalog.s3.access-key-id": "YOUR_ACCESS_KEY",
  "iceberg.catalog.s3.secret-access-key": "YOUR_SECRET_KEY",
  "iceberg.catalog.s3.path-style-access": true
}
```

## Project Structure

```
strimzi-ops/
├── app.py                      # Streamlit application
├── docker-compose.yaml         # Infrastructure definition
├── pyproject.toml              # Project config & dependencies (uv)
├── secrets.toml                # Configuration file
├── requirements.txt            # Python dependencies (legacy)
├── Makefile                    # Development commands
├── setup.sh                    # Automated setup script
├── .gitignore                  # Git ignore rules
├── config/
│   └── garage.toml            # Garage S3 configuration
├── examples/
│   ├── debezium-postgres-connector.json
│   └── iceberg-sink-connector.json
└── strimzi_ops/
    ├── __init__.py
    ├── config.py              # Configuration management
    ├── validator.py           # Connector validation
    ├── monitor.py             # Real-time monitoring
    └── control.py             # Connector control operations
```

## Development

All development commands use `uv` for dependency management and execution.

### Install Development Dependencies

```bash
# Sync all dependencies including dev dependencies
uv sync

# Or using Make
make install
```

### Running Tests

```bash
# Using uv
uv run pytest tests/ -v

# Or using Make
make test
```

### Code Formatting

```bash
# Format code with black
uv run black strimzi_ops/ app.py

# Or using Make
make format
```

### Code Linting

```bash
# Lint with ruff
uv run ruff check strimzi_ops/ app.py

# Or using Make
make lint
```

### Run All Checks

```bash
# Run format and lint checks
make check
```

### Type Checking

```bash
uv run mypy strimzi_ops/
```

## Troubleshooting

### Kafka Connect Not Starting

If Kafka Connect fails to start, ensure Redpanda is healthy:

```bash
docker-compose ps
docker-compose logs redpanda
```

### Garage Access Keys Not Generated

Manually create keys using the Garage CLI:

```bash
docker exec -it garage /garage key create lakehouse-key
docker exec -it garage /garage bucket create warehouse
docker exec -it garage /garage bucket allow warehouse --read --write --key lakehouse-key
```

### Configuration Validation Errors

Ensure your connector configuration matches the Pydantic models in `strimzi_ops/validator.py`. Common issues:
- Missing required fields
- Incorrect data types
- Invalid connector class names

## References

- [Redpanda Documentation](https://docs.redpanda.com)
- [Garage Documentation](https://garagehq.deuxfleurs.fr)
- [Debezium Documentation](https://debezium.io)
- [Kafka Connect REST API](https://docs.confluent.io/platform/current/connect/references/restapi.html)

## License

MIT License

## Contributing

Contributions are welcome! Please submit a pull request or open an issue.
