# Strimzi Ops Platform

A unified platform to lint, monitor, and control Kafka Connect deployments running on Kubernetes with Strimzi.

## Overview

Strimzi Ops is a comprehensive management platform for Kafka Connect, providing three core features:

- **Linter** (CLI): Flexible validation of connector configurations with configurable rules
- **Monitor** (UI): Real-time snapshot tracking via Debezium Notifications
- **Control** (UI): Manage connectors - restart/pause/resume and trigger snapshots

This tool is designed to work with **existing Kafka Connect deployments** running on Kubernetes via Strimzi. It connects to your cluster remotely and provides a streamlined interface for managing connectors.

Whether you're building a data lakehouse with Debezium and Iceberg, streaming changes with JDBC connectors, or any other Kafka Connect use case - Strimzi Ops provides the tools you need to manage your connectors effectively.

## Architecture

### Production Setup (Kubernetes + Strimzi)

This is the recommended setup for actual use:

- **Kubernetes Cluster**: Your existing cluster running Strimzi
- **Strimzi Operator**: 0.50.0
- **Kafka**: 3.10.0 (Managed by Strimzi operator)
- **Kafka Connect**: Deployed via Strimzi KafkaConnect CRD with Debezium 3.1.2
- **Strimzi Ops**: Python 3.14+ application that connects remotely via:
  - Kafka Connect REST API (port 8083)
  - Kafka Bootstrap Servers (port 9092)

```
┌─────────────────────────────────────────┐
│   Kubernetes Cluster (Your Infra)      │
│                                         │
│  ┌──────────────────────────────────┐  │
│  │  Strimzi Kafka Connect           │  │
│  │  (port 8083)                     │  │
│  └──────────────────────────────────┘  │
│                                         │
│  ┌──────────────────────────────────┐  │
│  │  Kafka Cluster                   │  │
│  │  (port 9092)                     │  │
│  └──────────────────────────────────┘  │
└─────────────────────────────────────────┘
           ↑
           │ kubectl port-forward
           │ or LoadBalancer/Ingress
           ↓
┌─────────────────────────────────────────┐
│   Your Local Machine                    │
│                                         │
│  ┌──────────────────────────────────┐  │
│  │  Strimzi Ops (Python)            │  │
│  │  - Linter CLI                    │  │
│  │  - Monitor UI (Streamlit)        │  │
│  │  - Control UI (Streamlit)        │  │
│  └──────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

### Local Development/Testing

For testing locally, use the provided Kubernetes manifests:

- **Strimzi Operator**: 0.50.0
- **Kafka**: 3.10.0 (single-node cluster)
- **Database**: PostgreSQL 16 with CDC enabled
- **Kafka Connect**: With Debezium 3.1.2 (PostgreSQL & MySQL connectors)
- **App Logic**: Python 3.14+ (Streamlit + Confluent Kafka + Pydantic)

See the "Local Development Environment" section below for details.

## Prerequisites

- Docker and Docker Compose
- Python 3.14 or higher
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

### Prerequisites

- Python 3.12 or higher
- [uv](https://github.com/astral-sh/uv) - Fast Python package manager
- Kubernetes cluster with kubectl configured (Colima, Minikube, kind, or existing cluster)
- Access to a Kubernetes cluster with Strimzi Kafka Connect deployed (or use local dev setup)

### Option A: Complete Local Setup (Recommended for Testing)

If you want to run everything locally on Kubernetes:

```bash
# 1. Clone the repository
git clone <repository-url>
cd strimzi-ops

# 2. Start Kubernetes (Colima example)
colima start --kubernetes --cpu 4 --memory 4

# 3. Run complete setup (installs deps + deploys K8s)
make setup

# 4. In a new terminal, port forward services
make port-forward

# 5. Configure connection
cp secrets.toml.example secrets.toml
# Edit if needed (defaults work for local dev)

# 6. Start the UI (in another terminal)
make run
```

### Option B: Connect to Existing Cluster

If you already have Kafka Connect running on Kubernetes:

```bash
# 1. Clone and install
git clone <repository-url>
cd strimzi-ops
make install

# 2. Port forward to your cluster
kubectl port-forward svc/your-connect-api 8083:8083 -n your-namespace
kubectl port-forward svc/your-kafka-bootstrap 9092:9092 -n your-namespace

# 3. Configure connection
cp secrets.toml.example secrets.toml
# Edit with your cluster details

# 4. Start using the tools
make run  # Start UI
make lint-config FILE=examples/debezium-postgres-connector.yaml  # Lint configs
```

### Available Make Commands

Run `make help` to see all available commands:

```
Setup & Deployment:
  make setup         - Complete setup (install deps + deploy K8s)
  make deploy        - Deploy local Kubernetes environment
  make status        - Check deployment status
  make destroy       - Destroy local environment

Port Forwarding:
  make port-forward           - Forward Kafka Connect API (8083)
  make port-forward-kafka     - Forward Kafka bootstrap (9092)
  make port-forward-postgres  - Forward PostgreSQL (5432)

Application:
  make run           - Start Streamlit UI
  make lint-config   - Lint connector config (FILE=path)
```

## Usage

### Linter (CLI)

Validate your connector configurations before deploying them using the command-line linter:

```bash
# Using make (recommended)
make lint-config FILE=examples/debezium-postgres-connector.yaml

# Direct CLI usage
uv run strimzi-lint lint examples/debezium-postgres-connector.yaml

# With custom linter config
uv run strimzi-lint lint -c .lintrc.toml connector.yaml

# JSON output (useful for CI/CD)
uv run strimzi-lint lint --json connector.yaml

# Strict mode (warnings cause failure)
uv run strimzi-lint lint --strict connector.yaml
```

**Features:**

- Validates YAML and JSON configurations
- Comment-based rule disabling (`# lint-disable: rule-name`)
- Configurable rules via `.lintrc.toml`
- Multiple output formats (human-readable and JSON)

**Example with comment-based disabling:**

```yaml
# lint-disable: naming-convention, sensitive-data
name: legacy-connector
connector.class: io.debezium.connector.postgresql.PostgresConnector
database.password: ${env:DB_PASSWORD}
```

See `examples/` directory for more examples.

### Dashboard (UI)

View an overview of all connectors, their status, and health metrics.

1. Start the application: `make run`
2. Navigate to the **Dashboard** page

### Monitor (UI)

Track snapshot progress in real-time:

1. Start the application: `make run`
2. Navigate to the **Monitor** page
3. Configure the notification topic (default: `debezium.notifications`)
4. Set monitoring duration
5. Click **Start Monitoring**

The monitor will display real-time snapshot progress for all active connectors.

### Control (UI)

Manage your connectors:

1. Start the application: `make run`
2. Navigate to the **Control** page
3. Select a connector from the dropdown
4. Available actions:
   - **Resume**: Resume a paused connector
   - **Pause**: Pause a running connector
   - **Restart**: Restart a connector
   - **Trigger Snapshot**: Initiate a new snapshot
5. Edit configuration and update as needed

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
├── app.py                          # Streamlit application (Monitor & Control UI)
├── pyproject.toml                  # Project config & dependencies (uv)
├── secrets.toml                    # Configuration file (gitignored)
├── secrets.toml.example            # Configuration template
├── Makefile                        # Development commands
├── .gitignore                      # Git ignore rules
├── k8s/                            # Kubernetes manifests for local dev
│   ├── 00-namespace.yaml          # Kafka namespace
│   ├── 01-postgres.yaml           # PostgreSQL with CDC
│   ├── 02-kafka.yaml              # Strimzi Kafka cluster
│   ├── 03-kafka-connect.yaml      # Strimzi Kafka Connect
│   ├── deploy.sh                  # Deployment script
│   ├── destroy.sh                 # Cleanup script
│   └── status.sh                  # Status check script
├── examples/                       # Example connector configurations
│   ├── debezium-postgres-connector.json
│   ├── debezium-postgres-connector.yaml
│   ├── iceberg-sink-connector.json
│   ├── iceberg-sink-connector.yaml
│   ├── legacy-connector-with-exemptions.yaml
│   └── README.md
├── .github/
│   └── workflows/
│       └── lint-connectors.yml.example  # CI/CD example
└── strimzi_ops/                    # Python package
    ├── __init__.py
    ├── config.py                  # Configuration management
    ├── linter.py                  # Linting engine
    ├── validator.py               # Validation wrapper
    ├── monitor.py                 # Real-time monitoring
    ├── control.py                 # Connector control operations
    └── cli.py                     # Command-line interface
```

## CI/CD Integration

The linter can be integrated into your CI/CD pipeline to validate connector configurations automatically.

### GitHub Actions

See `.github/workflows/lint-connectors.yml.example` for a complete example. Basic usage:

```yaml
- name: Install uv
  uses: astral-sh/setup-uv@v1

- name: Install dependencies
  run: uv sync

- name: Lint connectors
  run: uv run strimzi-lint lint --strict connectors/my-connector.yaml
```

### GitLab CI

```yaml
lint-connectors:
  image: python:3.12
  before_script:
    - pip install uv
    - uv sync
  script:
    - uv run strimzi-lint lint --strict connectors/*.yaml
```

### Pre-commit Hook

Add to `.git/hooks/pre-commit`:

```bash
#!/bin/bash
for file in $(git diff --cached --name-only --diff-filter=ACM | grep -E '\.(yaml|yml|json)$'); do
  if [[ $file == connectors/* ]]; then
    uv run strimzi-lint lint "$file" || exit 1
  fi
done
```

## Local Development Environment

You can run a complete Kafka Connect stack locally using Kubernetes (Colima, Minikube, kind, etc.).

### Prerequisites for Local Development

- Kubernetes cluster (Colima, Minikube, kind, or Docker Desktop with Kubernetes)
- kubectl configured
- At least 4GB RAM allocated to your cluster

### Start Colima with Kubernetes (if not running)

```bash
# Start Colima with Kubernetes enabled
colima start --kubernetes --cpu 4 --memory 4

# Verify cluster is running
kubectl cluster-info
```

### Deploy Local Environment

```bash
# Option 1: Complete setup (installs deps + deploys K8s)
make setup

# Option 2: Just deploy K8s (if deps already installed)
make deploy
```

This will:

1. Install Strimzi operator (if not already installed)
2. Create namespace `kafka`
3. Deploy PostgreSQL with CDC enabled
4. Deploy Kafka cluster (single-node)
5. Deploy Kafka Connect with Debezium connectors
6. Wait for everything to be ready

The deployment takes 5-10 minutes on first run (Kafka Connect needs to build custom image).

### Check Status

```bash
make status
```

### Port Forward Services

After deployment, open separate terminals and run:

```bash
# Terminal 1: Kafka Connect REST API
make port-forward

# Terminal 2: Kafka Bootstrap Servers (if needed for monitoring)
make port-forward-kafka

# Terminal 3: PostgreSQL (optional)
make port-forward-postgres
```

### Configure Strimzi Ops

Copy secrets.toml.example and configure for localhost:

```bash
cp secrets.toml.example secrets.toml
```

The default localhost configuration works with port-forwarding:

```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"
```

### Start Using

```bash
# Start the UI
make run

# Lint a connector configuration
make lint-config FILE=examples/debezium-postgres-connector.yaml
```

### Destroy Local Environment

When you're done:

```bash
make destroy
```

This removes all resources but keeps the Strimzi operator installed for faster future deployments.

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
