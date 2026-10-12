# Kubernetes Deployment for Local Development

This directory contains Kubernetes manifests and scripts to deploy a complete Kafka Connect environment locally using Strimzi.

## What Gets Deployed

1. **Strimzi Operator** - v1.2.0 - Kubernetes operator for managing Kafka
2. **Kafka Cluster** - v4.3.1 - Single-node cluster using **KRaft mode** (ZooKeeper-less)
3. **Kafka Connect** - v4.3.1 - Local image `my-connect-cluster:0.0.4` with Debezium 3.7.0.Final (PostgreSQL connector) and Iceberg 1.12.0 sink
4. **PostgreSQL** - v18.6-alpine - Database configured for CDC (Change Data Capture)
5. **RustFS** - **1.0.1** - S3-compatible storage with built-in Iceberg REST catalog (preview)

## Runtime Pin Provenance

Selected latest-stable pins are based on primary upstream sources:

- RustFS **1.0.1**: [latest stable release](https://github.com/rustfs/rustfs/releases/latest).
- Strimzi **1.2.0**, Kafka/Connect **4.3.1**: [release Kafka version matrix](https://github.com/strimzi/strimzi-kafka-operator/blob/1.2.0/kafka-versions.yaml).
- Debezium **3.7.0.Final**: [3.7 release notes](https://debezium.io/releases/3.7/release-notes/) and [PostgreSQL connector Maven metadata](https://repo.maven.apache.org/maven2/io/debezium/debezium-connector-postgres/maven-metadata.xml).
- PostgreSQL **18.6-alpine**: [versioning and supported stable releases](https://www.postgresql.org/support/versioning/); PostgreSQL 19 beta is not a stable pin.
- Apache Iceberg **1.12.0**: [upstream release](https://github.com/apache/iceberg/releases/tag/apache-iceberg-1.12.0).
- curl initializer **8.22.0**: [latest stable curl release](https://github.com/curl/curl/releases/tag/curl-8_22_0).

Upgrades must preserve all PVCs, PostgreSQL data, Kafka/Connect offsets, and
RustFS tables. Rolling back upgraded components requires retained state that
is compatible with the older versions; reverting image tags alone is not a
safe downgrade. Do not delete PVCs, reset offsets, or recreate the database or
warehouse to perform an upgrade or rollback.

## Prerequisites

- Kubernetes cluster (Colima, Minikube, kind, or Docker Desktop)
- kubectl installed and configured
- [just](https://github.com/casey/just) (`brew install just`)
- OpenSSL (`brew install openssl` on macOS), used for CA/server certificates and PKCS12
- Minimum 8GB RAM allocated to your cluster for the stack and image build

## Quick Start

```bash
# From the project root — one command bring-up:
just setup

# Check status / tear down
just status
just destroy
```

`just setup` starts Colima if needed, builds the Connect image, deploys the stack,
writes `secrets.toml`, applies `test-connector.yaml`, starts background port-forwards,
and launches the UI.
You can also run the scripts directly from this directory:

```bash
./deploy.sh
./status.sh
./destroy.sh
```

## Detailed Guide

### 1. Start Your Kubernetes Cluster

#### Colima (Recommended for macOS)

```bash
# Start with Kubernetes enabled
colima start --kubernetes --cpu 4 --memory 8

# Verify
kubectl cluster-info
```

#### Minikube

```bash
minikube start --cpus 4 --memory 8192

# Verify
kubectl cluster-info
```

#### kind (Kubernetes in Docker)

```bash
kind create cluster --name strimzi-dev

# Verify
kubectl cluster-info
```

### 2. Deploy

```bash
# From project root
just deploy

# Or from k8s/ directory
./deploy.sh
```

The script will:
- Check if Strimzi operator is installed (installs if needed)
- Create namespace `kafka`
- Deploy PostgreSQL with CDC configuration
- Deploy RustFS and run the `rustfs-init` Job to initialize `warehouse`
- Deploy Kafka cluster in **KRaft mode** (takes ~3 minutes)
- Provision/reuse Secret `connect-metrics-tls` before deploying Kafka Connect with the custom local image
- Wait for everything to be ready

### 3. Verify Deployment

```bash
# From project root
just status

# Or from k8s/ directory
./status.sh
```

Expected output:
```
Pods:
  my-cluster-dual-role-0              1/1     Running
  my-connect-cluster-connect-0        1/1     Running
  postgres-0                          1/1     Running
  rustfs-0                            1/1     Running
```

### 4. Access Services

Open a separate terminal for each foreground port-forward:

```bash
just port-forward
just port-forward-kafka
just port-forward-postgres
just port-forward-rustfs
just port-forward-rustfs-console
```

Then configure `secrets.toml`:
```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"
metrics_targets_file = ".local/connect-metrics.toml"

[storage]
type = "s3"
endpoint_url = "http://localhost:9000"
access_key = "rustfsadmin"
secret_key = "rustfsadmin"
bucket = "warehouse"
```

The credentials above are **LOCAL DEV ONLY**. RustFS exposes S3 at
`http://localhost:9000`, its built-in Iceberg REST catalog at
`http://localhost:9000/iceberg`, and the console at `http://localhost:9001`.
In namespace `kafka`, use `http://rustfs:9000`, `http://rustfs:9000/iceberg`,
and `http://rustfs:9001` respectively. There is no separate catalog service.

The connector uses REST warehouse/prefix `warehouse` and S3 bucket `warehouse`.
Configure REST SigV4 with `iceberg.catalog.rest.sigv4-enabled=true`,
`iceberg.catalog.rest.signing-name=s3`,
`iceberg.catalog.rest.signing-region=us-east-1`,
`iceberg.catalog.rest.access-key-id=rustfsadmin`, and
`iceberg.catalog.rest.secret-access-key=rustfsadmin`. S3FileIO uses
`iceberg.catalog.client.region=us-east-1`, endpoint `http://rustfs:9000`,
the same credentials in `s3.access-key-id` / `s3.secret-access-key`, and
`s3.path-style-access=true` (all under `iceberg.catalog`).

RustFS is pinned to **1.0.1**. Its
[S3 Tables / Iceberg REST support](https://docs.rustfs.com/en/administration/data/s3-tables)
is a preview feature, not a guarantee of full REST compatibility. The single-node
HTTP deployment and fixed administrator credentials are not production settings.

### Fresh Warehouse Cutover and Rollback

This deployment starts a **fresh warehouse**, not an automatic migration from
Garage/Nessie. Deployment retains their legacy PVCs; preserve those volumes and
save the previous connector configurations. To roll back, restore the previous
git revision, deploy its stack against the retained volumes, and restore its
connector configurations. Connector offsets are not reset automatically, so
switching catalog endpoints does not replay already-consumed records. Decide
explicitly whether replay is needed; deleting the namespace or retained PVCs
can destroy rollback data.

### Verify the Iceberg Sink Locally

The sample source expects `source_db.public.users`; setup does not seed it.
On a fresh database, create it and insert records:

```bash
kubectl exec -n kafka postgres-0 -- psql -U postgres -d source_db -v ON_ERROR_STOP=1 -c \
  "CREATE TABLE public.users (id bigint PRIMARY KEY, name text NOT NULL, email text NOT NULL);
   INSERT INTO public.users VALUES
   (101, 'RustFS smoke alpha', 'rustfs-alpha@example.test'),
   (102, 'RustFS smoke beta', 'rustfs-beta@example.test');"
```

Allow a sink commit cycle (configured interval: 60 seconds), then use an
independent DuckDB client with the `iceberg` and `httpfs` extensions:

```sql
INSTALL iceberg;
LOAD iceberg;
INSTALL httpfs;
LOAD httpfs;
CREATE SECRET rustfs_s3 (
  TYPE s3, KEY_ID 'rustfsadmin', SECRET 'rustfsadmin',
  REGION 'us-east-1', ENDPOINT '127.0.0.1:9000',
  URL_STYLE 'path', USE_SSL false
);
ATTACH 'warehouse' AS rustfs (
  TYPE iceberg, ENDPOINT 'http://127.0.0.1:9000/iceberg',
  AUTHORIZATION_TYPE 'sigv4', SECRET 'rustfs_s3',
  SIGV4_REGION 'us-east-1', SIGV4_SERVICE 's3',
  ACCESS_DELEGATION_MODE 'none'
);
SELECT "after".id AS id, "after".name AS name, "after".email AS email, op
FROM rustfs.public.users ORDER BY id;
```

The two inserted records should appear with `op = 'c'`. This checks PostgreSQL
CDC, Kafka Connect's Iceberg commit, REST catalog discovery, and S3 file reads,
not just connector `RUNNING` status. The CDC envelope is retained by design.
DuckDB is a verification tool, not a dependency of the application or setup.

Local verification used RustFS **1.0.1**, Strimzi **1.2.0**, Kafka/Connect
**4.3.1**, Debezium **3.7.0.Final**, PostgreSQL **18.6**, Iceberg Connect
**1.12.0**, and DuckDB **1.5.6**. The upgraded plugin loaded and both connectors
ran successfully. Three existing CDC rows remained byte-for-byte equivalent
through the catalog and S3 after the upgrade; a fourth PostgreSQL insert
committed with its exact ID, name, email, and `op = 'c'`. The replication slot
remained active, and existing PostgreSQL, Kafka, RustFS, and legacy
Garage/Nessie PVCs remained bound. Before the upgrade, the existing rows also
survived a RustFS pod restart and initializer rerun.

## Per-worker Connect and Debezium Metrics

`deploy.sh` provisions/reuses TLS Secret `connect-metrics-tls` before applying
`03-kafka-connect.yaml`. The manifest applies the exporter ConfigMap **before**
KafkaConnect and enables JMX Exporter 1.6.0 **HTTPS** on pod port **9404** at
`/metrics`. `httpServer.ssl.keyStore` loads a mounted PKCS12 file from
`/mnt/connect-metrics-tls/server.p12`; its `password:
"${CONNECT_METRICS_KEYSTORE_PASSWORD}"` resolves the Secret-provided environment
variable inside JMX Exporter, not through a shell or a plaintext ConfigMap.
The exporter exposes actual Connect worker/rebalance, connector/task status,
source/sink record totals/task errors, and numeric Debezium snapshot/streaming
attributes. It does not deploy Prometheus, compute rates or derive health from REST.

Use the background supervisor, not a Service forward (a Service selects just
one worker):

```bash
just sync-secrets
just port-forward-all
just status-forwards
# Inspect current per-pod addresses; use one of those HTTPS URLs (port may differ):
cat .local/connect-metrics.toml
curl --fail --cacert .local/connect-metrics-tls/ca.crt https://127.0.0.1:9404/metrics
just stop-forwards
```

The trusted request must succeed without `--insecure`. The same exporter must
reject plain `http://` requests; an HTTPS request without the local CA must fail
certificate verification. After changing a targets file to HTTP, the UI reports
`metrics endpoints require HTTPS` and retains only explicitly stale last-good
samples. Restoring the HTTPS URL and its CA recovers on the next collection.

The supervisor discovers **all** pods with
`strimzi.io/name=my-connect-cluster-connect` every 15 seconds (API requests have a
5-second deadline). Each pod retains a dedicated localhost port starting at
9404, advancing past occupied ports without killing unrelated listeners. It
tracks pod UIDs to restart forwards after same-name recreation, adds workers on
scale-up, and removes them only after a successful discovery proves them absent.
Pending, unready and terminating pods remain in the targets file: an unreachable
HTTPS localhost port 0 represents their collection failure instead of hiding them.
Discovery failures retain known identities, stop their forwards and publish
unreachable targets; the next successful discovery repairs them. An empty
successful discovery publishes `workers = []`. Stopping forwards also retains
known identities with unreachable targets. Background PIDs/logs live under
`.local/port-forwards/metrics`; `stop-forwards` manages the supervisor and its
children, and `status-forwards` shows each worker. Use
`NAMESPACE=<namespace> CONNECT_CLUSTER=<cluster> bash scripts/local-dev.sh port-forward-all`
to change metrics discovery (the other local service names remain unchanged).
The TOML file is atomically replaced with mode 0600 on each discovery.
Every worker entry includes `ca_file = "connect-metrics-tls/ca.crt"`, resolved
relative to `.local/connect-metrics.toml`. `port-forward-all` retrieves only the
public CA from the existing Secret, so a different checkout can connect without
copying server private keys. Old HTTP supervisors are restarted automatically.

### Local TLS Provisioning, Renewal and Rollback

`just provision-metrics-tls` (also called by `just deploy`/`just setup`) needs
namespace `kafka` and OpenSSL. On first use it creates a 10-year local CA and a
one-year server certificate with IP SAN `127.0.0.1` and DNS SAN `localhost`.
The CA/server private keys, PKCS12, password and certificates are stored under
ignored `.local/connect-metrics-tls/` with directory mode 0700/file mode 0600.
Only `server.p12`, `password` and public `ca.crt` enter the Kubernetes Secret;
the CA private key remains local. `issuer.crt` is the local signing CA certificate;
`ca.crt` is synchronized public cluster trust. Never commit/share private material,
dump the Secret or enable shell tracing while handling it.

Provisioning is idempotent: an existing complete Secret is never replaced, even
if this checkout has no private keys. API/RBAC errors fail rather than generating
an unrelated CA. If the Secret is absent, complete valid local material is reused;
partial/public-only or expired material fails with an explicit recovery error.
Concurrent Secret creation cannot overwrite another provisioner's identity.
CA synchronization retains a changed previous trust certificate as
`ca.crt.retained.*` and leaves local signing/server keys untouched.

For renewal, back up the **entire** private TLS directory and current Secret to
restricted, uncommitted storage first; do not delete the Secret to trigger setup.
Use the matching `ca.key`/`issuer.crt` to sign a new server certificate with the
same SANs and `serverAuth` usage, rebuild `server.p12` with alias `localhost`
and the retained password using OpenSSL `-passout file:<password-file>`, then
explicitly update the Secret from files. Retain the prior PKCS12/password/CA.
For CA rotation, stage a new CA/certificate/keystore as a deliberate operator
change, publish its public CA to clients, and replace all three Secret entries
together. A synced public-only checkout cannot issue a renewal; use the original
signing keys or an explicitly managed new issuer.

After any Secret update, restart Connect via Strimzi's rolling-update annotation:
`kubectl annotate strimzipodset my-connect-cluster-connect -n kafka
strimzi.io/manual-rolling-update=true --overwrite`, wait for Connect readiness,
then run `just port-forward-all` to synchronize trust and verify with the
`curl --cacert` command above. Secret environment variables are fixed at pod
startup; never depend on live password reload. Rollback restores the retained
Secret PKCS12/password/CA together, rolls Connect again, and resynchronizes trust.
None of these procedures requires deleting PVCs, Kafka offsets or existing data.
The metrics HTTPS-only client policy remains in effect during rollback.

Configure `[kafka] metrics_targets_file = ".local/connect-metrics.toml"` in
`secrets.toml` (`just sync-secrets` does this). Relative paths resolve against the
secrets file directory. The server rereads the targets file on every collection,
so membership changes do not require a UI restart. For an existing cluster,
provide one private exporter endpoint per worker in a separate, owner-readable
file instead of an aggregate/load-balanced endpoint:

```toml
[[workers]]
id = "connect-worker-0"
url = "https://worker-0.metrics.example.internal/metrics"
bearer_token = "replace-with-private-token" # optional, sent only by the server
ca_file = "certificates/company-ca.pem" # optional PEM CA, relative to this TOML

[[workers]]
id = "connect-worker-1"
url = "https://worker-1.metrics.example.internal/metrics"
```

All metrics URLs must be HTTPS, **including unauthenticated loopback URLs**.
Without `ca_file` the collector uses default public roots. Both certificate-chain
trust and URL hostname/IP verification are enforced; there is no insecure bypass.
The local certificate SANs cover loopback forwarding, not arbitrary pod DNS names;
production/in-cluster collectors must use certificates for their real hostnames.

Protect that file (`chmod 600`) and do not commit tokens. Endpoint URLs, bearer
tokens and raw exporter/error responses are not sent to the browser. Local metrics
are encrypted HTTPS but have **no client authentication**: they are reachable
through the cluster and Kubernetes-authorized IPv4-loopback forwards. There is no
NodePort, Ingress, hostPort or public metrics Service in these manifests.
**Strimzi 1.2.0's generated NetworkPolicy permits port 9404 from any in-cluster
pod.** Kubernetes policies are additive, so adding another restrictive policy
does not revoke that allow. Production operators must manage the generated
policy/cluster network controls or put authenticated private endpoints in front
of exporters; the local manifest is not an in-cluster authentication boundary.

The UI server collects at most 32 workers concurrently, caps each response at
2 MiB, uses a 3-second request timeout, and waits 10 seconds after each collection
round before the next. Browser refreshes read the cached snapshot, not exporters.
Samples keep the complete identity `(worker, metric name, all labels)`, including
Connect `connector`/`task` and Debezium `server`/`context`; raw samples are never
silently combined. Missing or non-finite values are unavailable, not zero.
Kafka task-error `total-*` and `deadletterqueue-produce-*` cumulative metrics are
exported as counters; `last-error-timestamp` remains a gauge.
Raw UI tables render at most **25 rows per worker and 200 rows total**, with
explicit truncation notices. Collection/cache data remains complete; these are
display limits, not scrape/sample limits.

| State | Meaning |
| --- | --- |
| available | A successful scrape produced usable samples; a real reported zero remains zero. |
| unavailable | Metrics are not configured, membership/exporter is empty, or a requested sample is missing/non-finite. |
| collection error | A configured target or targets file (including a missing/unreadable configured file) failed before any successful data was collected. |
| stale | A failed scrape/file refresh retains previously good samples, or cached samples are older than 30 seconds. Sample age is shown; retained values are not current. |

The local PostgreSQL source explicitly uses
`custom.metric.tags: connector=postgres-source-connector,task=0`. This is
Debezium's supported property name, **not** `metrics.custom.tags`. PostgreSQL
has one Connect task, so `task=0` is accurate here; do not copy this static task
tag to a multi-task connector without its own valid task identity. Keep
`connector` equal to the KafkaConnector name. The exporter captures tags
regardless of ObjectName property order and retains Debezium's logical
`server` (`topic.prefix`), `context` and domain; untagged Debezium MBeans are
not assigned a guessed connector/task.

### Throughput Semantics and Local Smoke Check

Monitor projects the common Connect task metrics below, independent of plugin:

| UI stage | Native task metrics (exported with underscores and `kafka_connect_` prefix) | Meaning |
| --- | --- | --- |
| Source records written | `source-task` / `source-record-write-rate`, `source-record-write-total` | Kafka output after transforms/filtering. |
| Sink records read | `sink-task` / `sink-record-read-rate`, `sink-record-read-total` | Kafka input before transforms. |
| Sink records sent | `sink-task` / `sink-record-send-rate`, `sink-record-send-total` | Records sent/put to the sink plugin after transforms; **not proof of durable downstream commits**. |

See [Kafka 4.3 Connect metric definitions](https://kafka.apache.org/43/generated/connect_metrics.html).
Rates are native rolling records/sec gauges, not successive counter differences.
The UI sums each unique connector/task exactly once across workers, never
averages rates or includes connector/worker aggregates. Cumulative values sum
the current tasks' counters, not connector lifetime history: restarts and
reassignment can reduce them without producing negative rates or reset spikes.
Rates and counters are independently available; an absent counter does not
invalidate an otherwise complete rate.

Throughput readings use these states (separate from raw collection states):

| State | Meaning |
| --- | --- |
| available | Finite, nonnegative native reading; genuine zero means measured inactivity in the rolling window. |
| unsupported | A required metric is missing/invalid or has unsupported extra dimensions. No complete sum is shown when any known task lacks the metric. |
| stale | Historical task reading after failed/expired collection; never current throughput. Stale worker membership or shared-cache expiry suppresses connector sums because tasks may be hidden. |
| unreachable | Collection failed without a usable reading. An unreachable worker suppresses complete connector sums, but other healthy task readings remain available. |
| ambiguous | More than one worker claims a connector/task, or multiple series claim a task measurement. No arbitrary owner is selected and duplicates are not summed. |

Raw and throughput display truncation do not affect throughput aggregation.
Connector/task tables share a separate **200 stage-row limit per response**:
each rendered connector-stage or task-stage counts once. An exact omitted-row
notice covers hidden tasks, stages and connectors. All task measurements still
contribute to complete connector totals before truncation. Status metrics retain
known connector/task identities even when throughput is unsupported.
The backend cannot infer tasks whose identities and metrics are absent from
every exporter; configure every worker with the supplied exporter rules.

To exercise the existing local Debezium → Kafka → Iceberg path without consuming
payloads:

1. Run `just port-forward-all`, `just sync-secrets`, then `just ui`.
   Confirm both connectors/tasks are running and open `/monitor`.
2. Record the source write and sink read/send counters and task rows.
3. Insert a uniquely tagged batch into the existing `source_db.public.users`:

   ```bash
   SMOKE_TAG="issue17-$(uuidgen)"
   SMOKE_BASE="$(date +%s)000"
   kubectl exec -n kafka postgres-0 -- \
     psql -U postgres -d source_db -v ON_ERROR_STOP=1 -c \
     "INSERT INTO public.users (id, name, email)
      SELECT $SMOKE_BASE + n, '$SMOKE_TAG', '$SMOKE_TAG-' || n || '@example.test'
      FROM generate_series(1, 10) AS n;"
   ```

   The insert aborts on any ID collision instead of overwriting existing rows.
4. Leave Monitor open. Within collection/browser refresh cycles, source written
   and sink read/sent counters should increase by 10 and native rates should
   become positive. Check task rows and confirm their sums match connector rows.
   Rates reflect rolling windows, not batch size/elapsed browser time.
5. For optional source-only cleanup, retain the same shell variables:

   ```bash
   kubectl exec -n kafka postgres-0 -- \
     psql -U postgres -d source_db -v ON_ERROR_STOP=1 -c \
     "DELETE FROM public.users
      WHERE id BETWEEN $SMOKE_BASE + 1 AND $SMOKE_BASE + 10
        AND name = '$SMOKE_TAG'
        AND email = '$SMOKE_TAG-' || (id - $SMOKE_BASE) || '@example.test';"
   ```

   Cleanup itself produces CDC traffic and does not erase Kafka history or
   append-only Iceberg records. Do not infer durable Iceberg commits from these
   throughput readings; use the independent catalog/S3 check above if needed.

Issue #17 local smoke: ten committed PostgreSQL inserts moved source written
from **1 to 11**, sink read/sent from **0 to 10**, with positive native rates in
both connector and task tables. The browser retained the same document and
observed HTMX swaps, proving updates without full-page navigation.

### Debezium Streaming Health and Source Delay

Monitor adds a Debezium panel only for explicitly tagged exporter identities.
Common Connect throughput remains available for other plugins. Each row preserves
`connector`, `task`, worker, Debezium domain and logical `server`; no values are
summed across these identities. Only **`context="streaming"`** supplies values.
Snapshot/queue attributes can establish identity when streaming metrics are
missing, but snapshot counters never contribute to this panel. Snapshot
notification monitoring remains independent and unchanged.

The installed local **Debezium 3.7.0.Final PostgreSQL** connector and JMX Exporter
1.6.0 were checked against the running HTTPS endpoint, including its native
attribute names in `HELP` and exported labels. The streaming MBean is
`debezium.postgres:type=connector-metrics,context=streaming,server=lakehouse`
with custom tags `connector=postgres-source-connector,task=0`; ObjectName property
order is not significant. All readings below retain those four labels.

| UI measurement | Native MBean attribute | Actual PostgreSQL exported metric | Unit / meaning |
| --- | --- | --- | --- |
| Database connection | `Connected` | `debezium_postgres_connected` | Boolean: 1 connected, 0 disconnected from the source database; not proof of downstream delivery or catch-up. |
| Source delay | `MilliSecondsBehindSource` | `debezium_postgres_milli_seconds_behind_source` | Milliseconds from the last timestamped source change to Debezium processing it. |
| Streaming events | `TotalNumberOfEventsSeen` | `debezium_postgres_total_number_of_events_seen_total` | Cumulative source data-change events seen during streaming since start/reset. |
| Creates | `TotalNumberOfCreateEventsSeen` | `debezium_postgres_total_number_of_create_events_seen_total` | Cumulative create events since start/reset. |
| Updates | `TotalNumberOfUpdateEventsSeen` | `debezium_postgres_total_number_of_update_events_seen_total` | Cumulative update events since start/reset. |
| Deletes | `TotalNumberOfDeleteEventsSeen` | `debezium_postgres_total_number_of_delete_events_seen_total` | Cumulative delete events since start/reset. |

Other Debezium domains use the same supported attribute suffixes **only when
actually exported**; this is not a claim that every database connector/version
exposes every metric. Untagged beans are not matched to Connect names by guessing
from `server` or `topic.prefix`. With no identifiable Debezium samples the panel
is absent, and common panels/collection diagnostics remain visible.

**Source delay is not Kafka consumer-group lag.** It compares the database event
timestamp with the Connect JVM processing clock, not Kafka offsets, unconsumed
records, WAL backlog size or sink commit latency. Clock skew, timestamp precision
and clock adjustments affect the result. Synchronize database and worker clocks
before interpreting it. The last delay can remain unchanged while idle: even a
fresh scrape does not make that historical event delay a wall-clock backlog
estimate. It also does not establish end-to-end delivery to Kafka or a sink.
The tagged [3.7.0.Final `StreamingMeter` implementation](https://github.com/debezium/debezium/blob/v3.7.0.Final/debezium-connector-common/src/main/java/io/debezium/pipeline/meters/StreamingMeter.java)
returns **-1 before a timestamped measurement is available**; this was observed
on the live local connector before the smoke inserts.

Availability is independent for each field:

- A missing, nonfinite or negative reading is **unavailable**, never a fabricated
  zero. Negative delay, including the -1 sentinel or negative clock skew, is not
  clamped into apparent health. `Connected` must be exactly 0 or 1.
- Duplicate worker ownership/measurement series and unsupported extra dimensions
  are unavailable rather than selecting an arbitrary healthy reading.
- Failed collection retains good samples as **stale**; a shared cache older than
  30 seconds also expires available readings. The connection explicitly says
  **last observed: connected/disconnected (stale)**, not current health. Historical
  delay/counters are marked last observed/stale; worker sample age is shown below.
  Collection failure without a previous reading remains a collection error.
- Genuine reported zeros remain available. Counters can fall after task restart,
  reassignment or metrics reset; they are not durable Kafka output totals.

Streaming tables have an independent **200 identity-row display limit**, with an
exact omitted-row notice. Projection uses the complete cached snapshot before
truncation; raw/throughput display limits cannot hide supported streaming fields.

#### Real CDC Smoke Check

1. Start private per-worker forwards and the UI as described above. Confirm the
   PostgreSQL task is running, open `/monitor`, and record the streaming row.
2. Use a uniquely tagged insert into `source_db.public.users`, following the
   collision-safe insert and ID/tag tracking in the throughput smoke procedure.
   Verify streaming events/creates increase in the **same open browser page**
   after server collection and HTMX refresh (each normally about 10 seconds).
3. Update only those tagged rows, then delete only those rows with matching IDs
   and tag/email. Verify updates and deletes increase independently. Cleanup
   produces CDC traffic; it does not remove Kafka history or append-only sink data.
4. Confirm the snapshot-context counters are separate and the snapshot notification
   form remains usable. To check staleness without stopping the live connector,
   use a separate UI instance with its own targets file; make that file's HTTPS
   worker URL unreachable, observe last-observed/stale values, then restore it and
   verify recovery. Do not modify another UI's targets or stop shared forwards.

Issue #19 live smoke: three inserts changed streaming events/creates from 0 to 3
and delay from -1 to 509 ms. Three updates changed events to 6 and updates to 3;
the same browser document refreshed via HTMX and showed connected with 155 ms
delay. Snapshot-context event counters remained zero, while both PostgreSQL and
Iceberg common throughput panels remained present.
Deleting only those three smoke rows changed events to 9 and deletes to 3, with
354 ms delay. The isolated unreachable-target check showed **last observed:
connected (stale)** and historical counters; restoring the target recovered in
the same document. A throwaway live-snapshot rendering check also verified missing
connection/delay fields, cache expiry and exact row-cap omission.

The snapshot notification form was retained. A live notification session on this
host reached the existing error surface because Kafka advertises internal broker
DNS names that the host cannot resolve through the localhost bootstrap forward.
Snapshot tracker/session regression tests passed; successful live notification
consumption was not established by this smoke check.


Primary configuration references:
- [Strimzi 1.2.0 Kafka Connect exporter example](https://github.com/strimzi/strimzi-kafka-operator/blob/1.2.0/examples/metrics/kafka-connect-metrics.yaml).
- [JMX Exporter 1.6.0 rule input, ordering, labels and snake-case semantics](https://prometheus.github.io/jmx_exporter/1.6.0/configuration/rules/).
- [JMX Exporter 1.6.0 HTTP TLS configuration](https://prometheus.github.io/jmx_exporter/1.6.0/configuration/ssl/).
- [1.6.0 keystore password variable-resolution call](https://github.com/prometheus/jmx_exporter/blob/1.6.0/jmx_prometheus_common/src/main/java/io/prometheus/jmx/common/HTTPServerFactory.java#L1391-L1400) and [exact `${ENV_NAME}` resolver syntax](https://github.com/prometheus/jmx_exporter/blob/1.6.0/collector/src/main/java/io/prometheus/jmx/variable/VariableResolver.java).
- [Debezium PostgreSQL custom metric tags and monitoring](https://debezium.io/documentation/reference/3.7/connectors/postgresql.html#postgresql-monitoring).
- [Strimzi 1.2.0 generated Connect NetworkPolicy](https://github.com/strimzi/strimzi-kafka-operator/blob/1.2.0/cluster-operator/src/main/java/io/strimzi/operator/cluster/model/KafkaConnectCluster.java).

## Manifest Details

### 00-namespace.yaml
Creates dedicated `kafka` namespace for all resources.

### 01-postgres.yaml
Deploys PostgreSQL 18.6 Alpine with logical replication enabled.

### 02-kafka.yaml
Deploys single-node Kafka 4.3.1 cluster via Strimzi using **KRaft** and **KafkaNodePool**.

### 03-kafka-connect.yaml
Deploys Kafka Connect 4.3.1 with Debezium 3.7.0.Final PostgreSQL + Iceberg 1.12.0 sink connectors. Uses a custom local image `my-connect-cluster:0.0.4`.
Also applies `connect-metrics` and enables the private pod exporter; see
[per-worker metrics](#per-worker-connect-and-debezium-metrics).

`Dockerfile.connect` assembles the Iceberg 1.12.0 plugin and runtime dependencies
from Maven Central in a pinned Maven build stage rather than using the older
Confluent Hub 1.9.2 package. The upstream [Kafka Connect build](https://github.com/apache/iceberg/blob/apache-iceberg-1.12.0/kafka-connect/build.gradle)
defines the plugin dependencies; its runtime distribution is not published to
Maven Central. Building requires Docker BuildKit and network access to Docker
Hub and Maven Central, but no host Maven or JDK.

### 04-rustfs.yaml
Deploys RustFS 1.0.1 with persistent storage and the `rustfs` service exposing
S3 and built-in Iceberg REST on port 9000, and console on port 9001.

### 05-rustfs-init.yaml
Runs the `rustfs-init` Job to initialize the `warehouse` bucket with the fixed
local-dev credentials. Check failures with `kubectl -n kafka logs job/rustfs-init`.

## Troubleshooting

### Kafka Connect build takes too long

First build can take 5-10 minutes as it downloads connector plugins and builds a custom image. Subsequent deployments use cached image.

Check build progress:
```bash
kubectl logs -f my-connect-cluster-connect-build -n kafka
```

### Pods not starting

Check resource availability:
```bash
kubectl top nodes
kubectl describe pod <pod-name> -n kafka
```

### Port forward connection refused

Ensure pod is running:
```bash
kubectl get pods -n kafka
```

If pod is ready but connection fails, the service might not be fully initialized. Wait 30 seconds and try again.

### Strimzi operator issues

Check operator logs:
```bash
kubectl logs -f deployment/strimzi-cluster-operator -n kafka
```

## Customization

### Add More Connectors

Edit `03-kafka-connect.yaml` and add to the `plugins` section:

```yaml
- name: my-connector
  artifacts:
    - type: tgz
      url: https://example.com/connector.tar.gz
```

Then apply:
```bash
kubectl apply -f 03-kafka-connect.yaml
```

### Increase Resources

For larger deployments, edit the resource requests/limits in the YAML files.

### Use Different Kafka Version

Update `spec.kafka.version` in `02-kafka.yaml`.

## Cleanup

### Remove All Resources

```bash
./destroy.sh
```

This removes:
- All pods, services, and resources in `kafka` namespace
- Persistent Volume Claims
- The `kafka` namespace itself

**Warning:** Namespace teardown can also delete retained Garage/Nessie PVCs and
eliminate rollback data. Back up any data you need before running it.

### Remove Strimzi Operator

The operator is installed into the `kafka` namespace and is removed with `just destroy` / `./destroy.sh`.
To remove only the operator resources while keeping the namespace, delete the Strimzi Deployment/CRDs manually.
## Production Considerations

**This setup is for local development only.** For production:

1. Use multiple replicas for high availability
2. Configure proper resource requests/limits
3. Enable TLS/authentication
4. Use proper storage classes with backups
5. Configure monitoring (Prometheus/Grafana)
6. Use dedicated namespaces per environment
7. Implement proper secrets management
8. Configure network policies

See [Strimzi Documentation](https://strimzi.io/docs/operators/latest/overview.html) for production setup.
