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
- **Strimzi Operator**: 1.1.0
- **Kafka**: 4.3.0 (Managed by Strimzi operator, KRaft)
- **Kafka Connect**: Deployed via Strimzi KafkaConnect CRD with Debezium 3.6.0
- **Strimzi Ops**: Rust CLI + web UI that connect remotely via:
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
│  │  Strimzi Ops                     │  │
│  │  - Linter / ops CLI (Rust)       │  │
│  │  - Web UI (strimzi-ui / Axum)    │  │
│  └──────────────────────────────────┘  │
└─────────────────────────────────────────┘
```

### Local Development/Testing

For testing locally, use the provided Kubernetes manifests:

- **Strimzi Operator**: 1.1.0
- **Kafka**: 4.3.0 (single-node KRaft cluster)
- **Database**: PostgreSQL 18.4 with CDC enabled
- **Kafka Connect**: With Debezium 3.6.0 (PostgreSQL connector)
- **Object Storage / Catalog**: Garage 2.3.0 + Nessie 0.108.4
- **App Logic**: Rust (`strimzi-ops` / `strimzi-ui`)

See the "Local Development Environment" section below for details.

## Prerequisites

- Rust toolchain (stable) with Cargo
- [just](https://github.com/casey/just) — command runner (`brew install just`)
- Docker and kubectl (Colima recommended for local k8s)
- librdkafka (for Kafka features): `brew install librdkafka cmake pkg-config`
- Git

## Quick Start

### Prerequisites

- Rust / Cargo
- [just](https://github.com/casey/just) (`brew install just`)
- Kubernetes cluster with kubectl configured (Colima, Minikube, kind, or existing cluster)
- Access to a Kubernetes cluster with Strimzi Kafka Connect deployed (or use local dev setup)

### Option A: Complete Local Setup (Recommended for Testing)

If you want to run everything locally on Kubernetes:

```bash
# 1. Clone the repository
git clone <repository-url>
cd strimzi-ops

# 2. One command: Colima (if needed), deps, Connect image, K8s stack,
#    secrets.toml, sample connector, port-forwards, and Rust UI
just setup
```

Requires [just](https://github.com/casey/just), Docker, kubectl, and preferably Colima (`brew install just colima docker kubectl`).
First run can take 5–10 minutes. Tear down with `just destroy`.

### Option B: Connect to Existing Cluster

If you already have Kafka Connect running on Kubernetes:

```bash
# 1. Clone the repository
git clone <repository-url>
cd strimzi-ops

# 2. Port forward to your cluster (or use just port-forward-all for local stack)
kubectl port-forward --address 127.0.0.1 svc/your-connect-api 8083:8083 -n your-namespace
kubectl port-forward --address 127.0.0.1 svc/your-kafka-bootstrap 9092:9092 -n your-namespace

# 3. Configure connection
cp secrets.toml.example secrets.toml
# Edit with your cluster details

# 4. Start using the tools
cargo run -q -p strimzi-ui -- --port 8501
just lint-config examples/debezium-postgres-connector.yaml
```

(`just ui` also starts local-stack port-forwards; use `cargo run -p strimzi-ui` when pointing at an existing remote Connect.)

### Available Just Recipes

Run `just --list` to see all recipes. The important ones:

```
  just setup              - Full local bring-up (infra + UI)
  just destroy            - Tear down k8s resources + stop port-forwards
  just status             - Check deployment status
  just port-forward-all   - (Re)start background port-forwards; health-check Connect
  just status-forwards    - Show tracked port-forward PIDs
  just stop-forwards      - Stop background port-forwards only
  just run                - Ensure forwards, then start Rust web UI
  just ui                 - Same as just run (strimzi-ui on :8501)
  just doctor             - Pods + forwards + HTTP health checks
  just lint-config <file> - Lint a connector config
```

## Usage

### Linter (CLI)

Validate your connector configurations before deploying them using the command-line linter:

```bash
# Using just (recommended)
just lint-config examples/debezium-postgres-connector.yaml

# Direct CLI usage
cargo run -q -p strimzi-ops --bin strimzi-lint -- lint examples/debezium-postgres-connector.yaml

# With custom linter config
cargo run -q -p strimzi-ops --bin strimzi-lint -- lint -c .lintrc.toml connector.yaml

# JSON output (useful for CI/CD)
cargo run -q -p strimzi-ops --bin strimzi-lint -- lint --json connector.yaml

# Strict mode (warnings cause failure)
cargo run -q -p strimzi-ops --bin strimzi-lint -- lint --strict connector.yaml
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

1. Start the application: `just run`
2. Navigate to the **Dashboard** page

### Monitor (UI)

Track snapshot progress in real-time:

1. Start the application: `just run`
2. Navigate to the **Monitor** page
3. Configure the notification topic (default: `debezium.notifications`)
4. Set monitoring duration
5. Click **Start Monitoring**

The monitor consumes Debezium notifications for the selected duration, then shows snapshot status cards for connectors seen during the session.

### Control (UI)

Manage your connectors:

1. Start the application: `just run`
2. Navigate to the **Control** page
3. Select a connector from the dropdown
4. Available actions:
   - **Resume**: Resume a paused connector
   - **Pause**: Pause a running connector
   - **Restart**: Restart a connector
   - **Trigger Snapshot**: Initiate a new snapshot
   - **Logs**: View recent Connect pod logs filtered for the connector (requires `kubectl`)
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

## Rust CLI and UI

Cargo workspace (application is Rust-only):

- `strimzi-ops-core` — lint, Connect client, control (snapshots / YAML export), monitor, k8s helpers, shared settings
- `strimzi-ops` — primary CLI (`lint`, `connectors`, `cluster`, `snapshot`, `monitor`)
- `strimzi-lint` — compatibility binary that only exposes `lint`
- `strimzi-ui` — web UI for Dashboard, Control, timed Monitor, and kubectl logs (Askama + HTMX)

```bash
cargo run -p strimzi-ops --bin strimzi-lint -- lint examples/debezium-postgres-connector.yaml
# or
just lint-config examples/debezium-postgres-connector.yaml

# Control examples (port-forward Connect first)
cargo run -p strimzi-ops -- --connect-url http://127.0.0.1:8083 connectors list
cargo run -p strimzi-ops -- --connect-url http://127.0.0.1:8083 --bootstrap-servers 127.0.0.1:9092 \
  snapshot trigger my-connector --type incremental

# Web UI (ensures port-forwards; default port 8501)
just run
# or
just ui
```

Kafka-backed commands need **librdkafka** (`brew install librdkafka cmake pkg-config` on macOS).

Rust code follows [Canonical Rust best practices](https://canonical.github.io/rust-best-practices/introduction.html); see [docs/rust-best-practices.md](docs/rust-best-practices.md) and [AGENTS.md](AGENTS.md). Cursor loads `.cursor/rules/` automatically in future sessions.

## Project Structure

```
strimzi-ops/
├── Cargo.toml                      # Rust workspace (core + CLI + UI)
├── crates/                         # Rust crates (see docs/rust-best-practices.md)
│   ├── strimzi-ops-core/           # lint, connect, control, monitor, k8s, settings
│   ├── strimzi-ops/                # strimzi-ops + strimzi-lint binaries
│   └── strimzi-ui/                 # Axum Dashboard/Control/Monitor UI (just run)
├── secrets.toml                    # Configuration file (gitignored)
├── secrets.toml.example            # Configuration template
├── justfile                        # Development commands (just)
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
└── .github/
    └── workflows/
        └── lint-connectors.yml.example  # CI/CD example
```

## CI/CD Integration

The linter can be integrated into your CI/CD pipeline to validate connector configurations automatically.

### GitHub Actions

See `.github/workflows/lint-connectors.yml.example` for a complete example. Basic usage:

```yaml
- name: Install Rust
  uses: dtolnay/rust-toolchain@stable

- name: Lint connectors
  run: cargo run -q -p strimzi-ops --bin strimzi-lint -- lint --strict connectors/my-connector.yaml
```

### GitLab CI

```yaml
lint-connectors:
  image: rust:1.85
  script:
    - cargo run -q -p strimzi-ops --bin strimzi-lint -- lint --strict connectors/*.yaml
```

### Pre-commit Hook

Add to `.git/hooks/pre-commit` (or use the repo `.pre-commit-config.yaml`):

```bash
#!/bin/bash
for file in $(git diff --cached --name-only --diff-filter=ACM | grep -E '\.(yaml|yml|json)$'); do
  if [[ $file == connectors/* ]]; then
    cargo run -q -p strimzi-ops --bin strimzi-lint -- lint "$file" || exit 1
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
# One command (preferred)
just setup

# Or deploy only (if image/deps already ready)
just deploy
```

`just setup` will:

1. Start Colima with Kubernetes if no cluster is reachable
2. Build the local Connect image (`my-connect-cluster:0.0.3`)
3. Deploy Strimzi, PostgreSQL, Garage S3, Nessie, Kafka, and Kafka Connect
4. Write `secrets.toml` with Garage credentials from the setup job
5. Apply the sample Postgres source + Iceberg sink connectors
6. Start port-forwards in the background (IPv4 / `127.0.0.1`)
7. Launch the Rust web UI (`strimzi-ui`)

The first run takes 5–10 minutes.

### Check Status

```bash
just status
```

### Port Forward Services

After deployment, open separate terminals and run:

```bash
# Terminal 1: Kafka Connect REST API
just port-forward

# Terminal 2: Kafka Bootstrap Servers (if needed for monitoring)
just port-forward-kafka

# Terminal 3: PostgreSQL (optional)
just port-forward-postgres

# Terminal 4: Garage S3 (optional)
just port-forward-garage

# Terminal 5: Nessie catalog (optional)
just port-forward-nessie
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
just run

# Lint a connector configuration
just lint-config examples/debezium-postgres-connector.yaml
```

### Destroy Local Environment

When you're done:

```bash
just destroy
```

This removes all resources but keeps the Strimzi operator installed for faster future deployments.

## Development

### Running Tests

```bash
just rust-test
# or
just test
```

### Format and lint (Canonical)

```bash
just rust-fmt
just rust-check   # also aliased as just lint / just check
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

Ensure your connector configuration matches the schema validated by `strimzi-ops-core` / `strimzi-lint`. Common issues:

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
