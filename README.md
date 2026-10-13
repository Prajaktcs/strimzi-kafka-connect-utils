# Strimzi Ops Platform

A unified platform to lint, monitor, and control Kafka Connect deployments running on Kubernetes with Strimzi.

## Overview

Strimzi Ops is a comprehensive management platform for Kafka Connect, providing three core features:

- **Linter** (CLI): Flexible validation of connector configurations with configurable rules
- **Monitor** (UI): Per-worker Connect/Debezium metrics and real-time snapshot tracking via Debezium Notifications
- **Control** (UI): Manage existing connectors - restart/pause/resume and trigger snapshots. Connector creation is not available in the UI.

This tool is designed to work with **existing Kafka Connect deployments** running on Kubernetes via Strimzi. It connects to your cluster remotely and provides a streamlined interface for managing connectors.

Whether you're building a data lakehouse with Debezium and Iceberg, streaming changes with JDBC connectors, or any other Kafka Connect use case - Strimzi Ops provides the tools you need to manage your connectors effectively.

## Architecture

### Production Setup (Kubernetes + Strimzi)

This is the recommended setup for actual use:

- **Kubernetes Cluster**: Your existing cluster running Strimzi
- **Strimzi Operator**: 1.2.0
- **Kafka**: 4.3.1 (Managed by Strimzi operator, KRaft)
- **Kafka Connect**: 4.3.1, deployed via Strimzi KafkaConnect CRD with Debezium 3.7.0.Final
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

- **Strimzi Operator**: 1.2.0
- **Kafka**: 4.3.1 (single-node KRaft cluster)
- **Database**: PostgreSQL 18.6-alpine with CDC enabled
- **Kafka Connect**: 4.3.1, local image `my-connect-cluster:0.0.4` with Debezium 3.7.0.Final (PostgreSQL connector) and Iceberg 1.12.0
- **Object Storage / Catalog**: RustFS 1.0.1 with built-in Iceberg REST catalog
- **App Logic**: Rust (`strimzi-ops` / `strimzi-ui`)

See the "Local Development Environment" section below for details.

See [runtime pin provenance and state-preserving rollback requirements](k8s/README.md#runtime-pin-provenance) for the selected latest-stable stack.

## Prerequisites

- Rust toolchain (stable) with Cargo
- [just](https://github.com/casey/just) — command runner (`brew install just`)
- Docker and kubectl (Colima recommended for local k8s)
- OpenSSL (local metrics CA, certificates and PKCS12 provisioning)
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

Requires [just](https://github.com/casey/just), Docker, kubectl, OpenSSL, and preferably Colima (`brew install just colima docker kubectl openssl`).
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

## Per-worker Prometheus Metrics

The local stack enables **HTTPS** on Strimzi's JMX Prometheus exporter on each
Connect pod's port 9404. `just setup` provisions/reuses a private local CA and
server certificate with `127.0.0.1`/`localhost` SANs, installs the PKCS12/password
in Kubernetes Secret `connect-metrics-tls`, then deploys Connect. Private keys
and passwords remain in ignored, restrictive-permission `.local/connect-metrics-tls/`
and the Secret; they are never embedded in manifests.
`just port-forward-all` synchronizes only the Secret's public CA (including from
another checkout), starts one supervised IPv4-loopback forward **per pod**, and
atomically publishes `.local/connect-metrics.toml` with HTTPS URLs and relative
`ca_file = "connect-metrics-tls/ca.crt"`. It refreshes all-worker membership every
15 seconds (5-second Kubernetes API timeout), handles rollouts/scale changes, and
keeps unhealthy workers visible as failed HTTPS port-0 targets rather than
sampling a Service. A pre-TLS supervisor is automatically restarted.
`just status-forwards` shows workers; `just stop-forwards` stops only tracked
metrics processes without killing unrelated metrics-port listeners.

`just sync-secrets` sets this server-side configuration:

```toml
[kafka]
connect_url = "http://127.0.0.1:8083"
metrics_targets_file = ".local/connect-metrics.toml"
```

`metrics_targets_file` resolves relative to the secrets file; targets are reread
every collection. For remote exporters, use a private, owner-readable TOML file
with one `[[workers]]` entry per worker: `id`, **HTTPS** `url`, optional
`bearer_token`, and optional `ca_file` pointing to a PEM CA certificate relative
to the targets TOML directory (absolute paths also work). Without `ca_file`,
normal public trust roots apply. CA and hostname verification are always enabled;
**all HTTP metrics URLs are rejected, including unauthenticated loopback ones**.
Do not use a load-balanced exporter URL. Credentials/endpoints remain server-side.
No Prometheus server or public metrics exposure is deployed. The local HTTPS
exporter has no client authentication and remains reachable by in-cluster pods
under Strimzi's default metrics NetworkPolicy; TLS is not an authorization boundary.

Collection runs server-side with a 3-second request timeout and a 10-second
interval **after each round**; browser refreshes use cached results. Successful
samples are available; missing/non-finite samples, unconfigured metrics and
empty exporters/membership are unavailable (never fabricated zeroes). A configured
but missing/unreadable targets file is a collection error, not unconfigured metrics.
Failures without previous data show collection error. Last good samples survive
scrape/target-file failures as stale, and become stale after 30 seconds without
refresh; their age is retained. Samples keep worker, connector/task and all
Debezium server/context labels separate. Raw UI tables show at most **25 rows per
worker and 200 total**, with truncation notices; backend samples remain complete.

The local single-task PostgreSQL source uses Debezium's
`custom.metric.tags = connector=postgres-source-connector,task=0` to preserve
explicit identity; it is not inferred from `topic.prefix`. See
[exporter configuration, authenticated remote file example, limits and network-policy notes](k8s/README.md#per-worker-connect-and-debezium-metrics).

Monitor also shows applicable **Debezium streaming health**: source database
connection, last-event source delay in milliseconds, and cumulative streaming
event/create/update/delete counts, with connector/task/worker/domain/server
identity preserved. Snapshot-context measurements are excluded; other plugins
keep the common Connect panels. Missing/invalid readings are unavailable, and
stale connection values are explicitly **last observed**, never current health.
Source delay is **not Kafka consumer-group lag** or downstream delivery latency;
database/worker clock skew affects it, and idle periods retain the last event's
delay rather than a wall-clock backlog estimate. The local PostgreSQL 3.7.0.Final
metric names and real CDC behavior were verified against the running exporter.
See [streaming metric definitions, availability and live smoke procedure](k8s/README.md#debezium-streaming-health-and-source-delay).

Monitor also shows **queues and backpressure**:

- Per Debezium connector/task/server/context: queue total and remaining record
  capacity, derived record utilization `(total − remaining) / total`, and the
  connector's byte queue measurements with derived byte utilization. Byte
  readings are shown exactly as exported (without `max.queue.size.in.bytes` a
  real zero or a missing family is kept as such); only the derived byte
  utilization additionally requires a positive maximum. Utilization is computed
  per task, never averaged or summed across tasks, and no alert threshold is
  invented. Missing, zero or inconsistent capacity makes utilization
  explicitly unavailable instead of zero; snapshot and streaming contexts keep
  separate labeled rows.
- Per Kafka Connect source and sink task: active-record count, windowed peak
  and average (Kafka's sampled `Max`/`Avg` stats, which can decay as samples
  expire). Active records are pipeline depth, **not Kafka consumer
  lag**; only the sink column labeled consumer lag shows Kafka's own
  `sink-record-lag-max`. Metrics never alter connector behavior.

Both tables share the shared display limits and state vocabulary (stale values
are historical, collection errors show no values). See [queue and backpressure
measurements, exporter notes and the induced-backlog smoke check](k8s/README.md#queue-and-backpressure-measurements).

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

View an overview of all connectors, their status, and health metrics, plus a
processing-error panel: per connector/task cumulative record errors, failures,
retries, skipped and logged counts, dead-letter writes/failures, derived rates,
and explicit availability (missing metrics or a DLQ that is not configured are
never shown as healthy zeros; metric collection failures are never reported as
connector processing failures). See [supported measurements and
semantics](k8s/README.md#processing-error-and-dead-letter-measurements) and
[metrics setup](#per-worker-prometheus-metrics).

1. Start the application: `just run`
2. Navigate to the **Dashboard** page

### Monitor (UI)

The Monitor landing page shows **live connector and task throughput**:
source records written to Kafka/sec, sink records read from Kafka/sec (before
transforms), and sink records sent to the plugin/sec (after transforms).
**Sink sent is not a durable downstream commit.** Each stage also shows its
available current task-lifetime cumulative count.

Rates use Kafka Connect's native rolling gauges, not counter differences.
Connector rates and counters are **sums of unique tasks, never averages**.
Counters can decrease after task restarts/reassignment; no delta-derived rate
spikes are introduced. Duplicate task ownership or metric series are ambiguous;
missing task measurements prevent complete sums. Any unhealthy worker prevents
complete connector aggregates because it may hide tasks; healthy task readings
remain usable. Unsupported, stale, unreachable and ambiguous readings are
explicit, not fabricated zeroes. Tiny positive rates use scientific notation
rather than rounding to zero.

Connector and task throughput tables share a **200 stage-row display limit** per
response, with an exact omitted-row notice; aggregation still uses all tasks.
When the shared cache expires, connector sums are suppressed while task values
remain explicitly historical/stale.

The page refreshes every 10 seconds through `/monitor/metrics`, without a full-page
reload; browser requests never start another collector or consume message payloads.
Worker availability, last-good sample age and expandable raw measurements remain
available. **JMX metrics** do not require Kafka bootstrap servers. See
[metrics setup](#per-worker-prometheus-metrics) and the
[real source/sink throughput smoke check](k8s/README.md#throughput-semantics-and-local-smoke-check).

Monitor also shows supported **Debezium snapshot measurements**: running,
completed, aborted, paused/skipped flags, elapsed seconds, total/remaining table
counts, per-table rows scanned and cumulative snapshot events. Every reading
preserves the tagged connector/task/worker/domain/server/exporter context.
Missing measurements are unavailable, never zero; stale values are explicitly
last observed. No row-count percentage is invented when the total is unknown.

The installed PostgreSQL connector shares its native `snapshot` metric context
between initial and incremental snapshots. The UI does not infer the snapshot
kind or a new run from that context. Row-map entries and event counters can
outlive a run and are labeled retained/historical, not a new snapshot's progress.
See [snapshot measurements and source reconciliation](k8s/README.md#debezium-snapshot-measurements-and-notifications)
for supported attributes, identity limitations and browser smoke evidence.

Monitor also shows **Kafka consumer-group committed-offset lag** for sink
connectors, independently of JMX availability. Configure both `kafka.connect_url`
and `kafka.bootstrap_servers`. The collector checks the Kafka cluster ID against
Connect, resolves the default input group `connect-{connector-name}` or
`consumer.override.group.id`, and reads committed offsets and partition log-end
offsets without subscribing, assigning, consuming payloads or committing offsets.

Each topic/partition shows its committed offset, end offset and lag; the connector
total is the checked sum of the complete subscribed partition set, not a maximum
or a sum of duplicated task measurements. Missing groups/commits, including empty
partitions without commits, remain unavailable rather than zero. Failed or
expired collection retains explicitly historical/stale readings. Offset requests
are not an atomic snapshot during activity/rebalances; commit batching,
compaction and transactions mean offset distance is not an exact record count or
proof of downstream durability. Source delay and Connect active records/native
lag-max remain separate measurements.

Collection shares the application cache: a 10-second delay between rounds,
30-second stale threshold, 3-second per-request timeouts and an 8-second offset
round deadline (client shutdown is outside the request deadline). Limits are
128 discovered connectors, 1,024 subscribed partitions per round, and Kafka
metadata of 4,096 topics/16,384 partitions; exceeding them is an explicit error.
The UI displays at most 200 connector-summary/partition rows without truncating
backend aggregation.

Explicit `topics` and a conservative full-topic `topics.regex` subset are
supported: ASCII literals, ordinary groups, `.`, `*`, `+`, `?`, `|`, `^`, `$`,
and escaped dots/hyphens. Java character classes, flags, lookarounds,
backreferences and counted/lazy/possessive repetitions are explicitly unsupported,
as are unresolved config-provider expressions. The current collector uses
plaintext Kafka; consumer security/isolation overrides and bootstrap overrides
that differ from its configured server list are unavailable, never silently read
from a guessed cluster. Endpoint configuration stays server-side.

Regex expansion respects `consumer.override.exclude.internal.topics` (default
`true`): Kafka's internal `__consumer_offsets`, `__transaction_state` and
`__share_group_state` topics are excluded unless set to `false`. This matches
[Kafka's internal-topic classification](https://github.com/apache/kafka/blob/4.3.1/clients/src/main/java/org/apache/kafka/common/internals/Topic.java),
not a blanket exclusion of names starting with `__`. Explicit `topics` may name
internal topics regardless of this setting. Invalid or unresolved overrides
make the mapping unavailable; changing the setting invalidates cached readings.

The single-broker local setup provides a private listener advertising
`127.0.0.1:9092` and a broker-specific loopback forward; the existing in-cluster
listener is unchanged. See [listener rollout, rollback, offset semantics and
isolated backlog/recovery verification](k8s/README.md#host-kafka-metadata-and-sink-consumer-group-lag).

Track snapshot lifecycle and measurements in a timed live session:

1. Start the application: `just run`
2. Navigate to the **Monitor** page
3. Configure the notification topic (default: `debezium.notifications`)
4. Set monitoring duration (10–300 seconds)
5. Click **Start Monitoring**

The page opens immediately and streams notification events **and cached snapshot
metrics** over Server-Sent Events (SSE) during the selected duration. Each viewer
has an independent Kafka consumer, so simultaneous sessions do not split
notifications. Metrics still use the one application-wide collector.

UI viewer groups disable automatic offset storage and commits, so repeated
sessions do not retain Kafka offset records. The CLI's named monitor groups
keep their existing commit behavior. Native `aggregate_type` distinguishes
initial/incremental notification contexts; `additional_data.connector_name`
identifies the logical source (`topic.prefix` for PostgreSQL), not necessarily
the Connect REST name. The last 20 accepted wire events per source/context remain
expandable, including collection/table/chunk events and terminal events received
without `STARTED`. Unknown row totals produce no progress bar.

Click **Stop Monitoring** to end a session early; navigating away also releases
its consumer. At the deadline, updates stop and the final observations remain
visible. A notification-consumer failure is shown alongside metrics and does
**not** terminate metric updates before the deadline. An SSE connection failure
ends the session without automatic reconnection; retained observations remain.
Sessions start from new notifications, retain only bounded in-session event
history, and require JavaScript.

Kafka's advertised broker addresses must be reachable from the UI process. Forwarding only the bootstrap port is insufficient if Kafka advertises cluster-only DNS names.

### Control (UI)

Manage your connectors:

1. Start the application: `just run`
2. Navigate to the **Control** page
3. Select a connector from the dropdown
4. Available actions:
   - **Resume**: Resume a paused connector
   - **Pause**: Pause a running connector
   - **Restart**: Restart a connector
   - **Trigger Snapshot**: Initiate a new snapshot (shown only for connectors Kafka Connect reports as `source`, never for sinks)
   - **Logs**: View recent Connect pod logs filtered for the connector (requires `kubectl`)
5. Edit configuration and update as needed

## Configuration Examples

### Debezium PostgreSQL Connector

```json
{
  "name": "postgres-source-connector",
  "connector.class": "io.debezium.connector.postgresql.PostgresConnector",
  "tasks.max": 1,
  "custom.metric.tags": "connector=postgres-source-connector,task=0",
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
- `strimzi-ui` — web UI for Dashboard, Control, live Monitor (SSE), and kubectl logs (Askama + HTMX)

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
- At least 8GB RAM allocated to your cluster for the stack and image build

### Start Colima with Kubernetes (if not running)

```bash
# Start Colima with Kubernetes enabled
colima start --kubernetes --cpu 4 --memory 8

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
2. Build the local Connect image (`my-connect-cluster:0.0.4`)
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
Rollback after component upgrades additionally requires state compatible with
the older versions; preserve all PVCs, database data, offsets, and RustFS tables
without deletion or reset. See [runtime pin provenance](k8s/README.md#runtime-pin-provenance).

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

# Terminal 4: RustFS S3/catalog (optional)
just port-forward-rustfs

# Terminal 5: RustFS console (optional)
just port-forward-rustfs-console
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
metrics_targets_file = ".local/connect-metrics.toml"
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
