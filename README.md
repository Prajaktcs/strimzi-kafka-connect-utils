# Strimzi Ops Platform

A unified platform to lint, monitor, and control Kafka Connect deployments running on Kubernetes with Strimzi.

## Overview

Strimzi Ops is a comprehensive management platform for Kafka Connect, providing three core features:

- **Linter** (CLI): Flexible validation of connector configurations with configurable rules
- **Monitor** (UI): Real-time snapshot tracking via Debezium Notifications
- **Control** (UI): Manage existing connectors - restart/pause/resume and trigger snapshots. Connector creation is not available in the UI.

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
- **Object Storage / Catalog**: RustFS 1.0.1 with built-in Iceberg REST catalog
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
  "schema.history.internal.kafka.bootstrap.servers": "my-cluster-kafka-bootstrap:9092",
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
  "topics": "lakehouse.public.users",
  "iceberg.tables": "public.users",
  "iceberg.tables.auto-create-enabled": true,
  "iceberg.catalog.type": "rest",
  "iceberg.catalog.uri": "http://rustfs:9000/iceberg",
  "iceberg.catalog.warehouse": "warehouse",
  "iceberg.catalog.prefix": "warehouse",
  "iceberg.catalog.io-impl": "org.apache.iceberg.aws.s3.S3FileIO",
  "iceberg.catalog.rest.sigv4-enabled": true,
  "iceberg.catalog.rest.signing-name": "s3",
  "iceberg.catalog.rest.signing-region": "us-east-1",
  "iceberg.catalog.rest.access-key-id": "rustfsadmin",
  "iceberg.catalog.rest.secret-access-key": "rustfsadmin",
  "iceberg.catalog.client.region": "us-east-1",
  "iceberg.catalog.s3.endpoint": "http://rustfs:9000",
  "iceberg.catalog.s3.access-key-id": "rustfsadmin",
  "iceberg.catalog.s3.secret-access-key": "rustfsadmin",
  "iceberg.catalog.s3.path-style-access": true,
  "key.converter": "org.apache.kafka.connect.json.JsonConverter",
  "value.converter": "org.apache.kafka.connect.json.JsonConverter",
  "key.converter.schemas.enable": false,
  "value.converter.schemas.enable": false
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
        └── ci.yml                  # fmt, clippy, tests, example lint
```

## CI/CD Integration

This repo runs [`.github/workflows/ci.yml`](.github/workflows/ci.yml) on pushes and pull requests to `main`:

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- lint every `examples/*.{yaml,yml,json}` connector config with `strimzi-lint`

Locally, the same gates are:

```bash
just rust-check
just rust-test
just lint-config examples/debezium-postgres-connector.yaml
```

### Lint your own connector configs

```bash
cargo run -q -p strimzi-ops --bin strimzi-lint -- lint --strict path/to/connector.yaml
```

In GitHub Actions (after installing a Rust toolchain):

```yaml
- name: Lint connectors
  run: |
    set -euo pipefail
    shopt -s nullglob globstar
    for file in connectors/**/*.{yaml,yml,json}; do
      cargo run -q -p strimzi-ops --bin strimzi-lint -- lint --strict "$file"
    done
```

Pre-commit: this repo’s [`.pre-commit-config.yaml`](.pre-commit-config.yaml) runs `just rust-check` on Rust changes.

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
3. Deploy Strimzi, PostgreSQL, RustFS with its built-in catalog, Kafka, and Kafka Connect
4. Write `secrets.toml` with the fixed local-dev RustFS credentials
5. Apply the sample Postgres source + Iceberg sink connectors
6. Start port-forwards in the background (IPv4 / `127.0.0.1`)
7. Launch the Rust web UI (`strimzi-ui`)

The first run takes 5–10 minutes.

The local Iceberg sink sends `lakehouse.public.users` to `public.users` using
RustFS's built-in Iceberg REST endpoint and `S3FileIO`. With auto-creation
enabled, the connector creates the namespace and table on the first record.
Auto-creation does not select table names; `iceberg.tables` supplies them. The
multi-topic examples use a topic field and per-table routing regexes instead
of `topic=table` entries.

RustFS is pinned to **1.0.1**. Its [S3 Tables / Iceberg REST support](https://docs.rustfs.com/en/administration/data/s3-tables)
is a preview feature; this single-node, HTTP deployment is for local development,
not production or a guarantee of complete Iceberg REST compatibility.
The `warehouse` bucket is also the REST warehouse/prefix. The `rustfs-init` Job
initializes storage; no separate catalog service is deployed.
The fixed `rustfsadmin` / `rustfsadmin` credentials are **LOCAL DEV ONLY**.
REST requests use SigV4 (`s3`, `us-east-1`), with explicit
`rest.access-key-id` and `rest.secret-access-key`; S3FileIO uses the same
credentials, region, and path-style addressing.
Connect's `offset.flush.interval.ms=10000` keeps idle Iceberg control consumers
polling below their session timeout.

This is a **fresh warehouse cutover**, not an automatic migration of Garage
objects or Nessie tables. Deployment retains legacy Garage/Nessie PVCs; do not
delete them if rollback is needed. To roll back, restore the previous git
revision, deploy its stack with the retained volumes, and restore the previous
connector configurations. Connector offsets are not reset automatically:
switching warehouses does not replay already-consumed records. Back up connector
configuration and decide on replay explicitly before cutover; namespace teardown
can destroy the retained volumes.

The local worker serializes JSON keys and values with schemas disabled, matching
the sink's explicit converter settings. Debezium CDC fields such as `after` and
`op` remain intact; the converter does not add a `{schema,payload}` wrapper.
Changing these defaults does not rewrite existing Kafka messages.

The sample source expects `source_db.public.users`; deploying the connectors
does not create that PostgreSQL table or seed records.

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

# Terminal 4: RustFS S3/catalog and console (optional)
just port-forward-rustfs
```

RustFS endpoints: S3 `http://localhost:9000`, Iceberg REST
`http://localhost:9000/iceberg`, console `http://localhost:9001`.
Inside namespace `kafka`, connectors use `http://rustfs:9000` and
`http://rustfs:9000/iceberg`; the console is `http://rustfs:9001`.

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

Check the local stack and Connect logs:

```bash
just doctor
just status
kubectl -n kafka get pods
kubectl -n kafka logs -l strimzi.io/name=my-connect-cluster-connect --tail=100
```

Ensure Kafka is ready before Connect (`my-cluster-kafka-bootstrap` in the `kafka` namespace).

### RustFS Warehouse Initialization Fails

Check the initializer and RustFS logs:

```bash
kubectl -n kafka logs job/rustfs-init
kubectl -n kafka logs -l app=rustfs
```

Verify the `warehouse` bucket and the local-dev credentials
`rustfsadmin` / `rustfsadmin`. REST and S3 credentials must both be configured;
REST signing uses service `s3` and region `us-east-1`.

### Configuration Validation Errors

Ensure your connector configuration matches the schema validated by `strimzi-ops-core` / `strimzi-lint`. Common issues:

- Missing required fields
- Incorrect data types
- Invalid connector class names

## References

- [Strimzi Documentation](https://strimzi.io/docs/operators/latest/overview.html)
- [RustFS S3 Tables / Iceberg REST Documentation](https://docs.rustfs.com/en/administration/data/s3-tables)
- [Debezium Documentation](https://debezium.io)
- [Kafka Connect REST API](https://docs.confluent.io/platform/current/connect/references/restapi.html)

## License

MIT License

## Contributing

Contributions are welcome! Please submit a pull request or open an issue.
