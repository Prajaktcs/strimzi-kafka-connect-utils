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
bootstrap_servers = "127.0.0.1:9092"
connect_url = "http://127.0.0.1:8083"
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

### Host Kafka Metadata and Sink Consumer-group Lag

Kafka bootstrap connectivity alone is insufficient: clients subsequently connect
to the broker and group coordinator addresses returned in metadata. The existing
`plain` listener on **9092** advertises Kubernetes DNS and remains unchanged for
Connect (`my-cluster-kafka-bootstrap:9092`). Host clients use a separate **internal**
`local` listener on **9094**, with broker **0** advertising **127.0.0.1:9092**.
The private ClusterIP Service `my-cluster-kafka-local-0` selects only
`my-cluster-dual-role-0`, never a load-balanced bootstrap listener:

```bash
# Foreground, after applying the listener manifest and waiting for reconciliation:
kubectl port-forward --address 127.0.0.1 -n kafka \
  svc/my-cluster-kafka-local-0 9092:9094
# In another terminal (optional kcat; metadata only, no group join):
kcat -b 127.0.0.1:9092 -L -m 5
```

The metadata must identify broker 0 at `127.0.0.1:9092`. Forwarding
`svc/my-cluster-kafka-bootstrap 9092:9092` cannot repair its advertised DNS.
This configuration is **single-broker local development only**. Scaling the
node pool or changing broker IDs requires separate reachable addresses/forwards
for every advertised broker; do not reuse this mapping for multiple brokers.
Strimzi's [listener configuration reference](https://strimzi.io/docs/operators/latest/full/configuring.html)
documents per-broker `advertisedHost` and `advertisedPort`.

No public NodePort, LoadBalancer or ingress is created. Both the local listener
and forward carry **plaintext, unauthenticated Kafka traffic**. Binding to
127.0.0.1 prevents remote host access, not access by other local processes;
cluster workloads may also reach the listener. Kubernetes API/RBAC controls who
can forward. This is not a production security configuration. Keep settings and
credentials private; use separately designed TLS/authentication/network policy
for non-local deployment.

#### Listener Rollout and Rollback Without Resetting State

Do not run `destroy`, `destroy-hard`, delete the namespace/PVCs, recreate the
KafkaNodePool, reset group offsets or delete Connect internal topics. This change
adds a listener and Service; it does not change the internal Connect endpoint or
group identity. Operator reconciliation can roll broker 0 and briefly interrupt
clients. Schedule it accordingly and retain existing volumes/groups/offsets.

For an existing cluster, from the project root:

```bash
mkdir -p .local
# Retain the exact pre-change listener configuration, not a guessed old manifest.
kubectl get kafka my-cluster -n kafka -o json \
  | jq '{spec:{kafka:{listeners:.spec.kafka.listeners}}}' \
  > .local/kafka-listeners.before.json
kubectl get pvc -n kafka -o wide > .local/kafka-pvcs.before.txt
# Record important group's offsets/members before rollout with the read-only
# kafka-consumer-groups commands below; save connector configs privately too.
kubectl apply -f k8s/02-kafka.yaml
generation="$(kubectl get kafka my-cluster -n kafka -o jsonpath='{.metadata.generation}')"
kubectl wait kafka/my-cluster -n kafka \
  --for="jsonpath={.status.observedGeneration}=${generation}" --timeout=300s
kubectl wait kafka/my-cluster -n kafka --for=condition=Ready --timeout=300s
kubectl get svc my-cluster-kafka-local-0 -n kafka -o wide
kubectl get endpointslice -n kafka \
  -l kubernetes.io/service-name=my-cluster-kafka-local-0 -o yaml
# Expect broker 0's endpoint and port 9094, then restart stale pre-change forwards.
just port-forward-all
just status-forwards
just doctor
# Optional: overwrites secrets.toml with repo LOCAL DEV ONLY credentials/settings.
just sync-secrets
```

`port-forward-all` detects/replaces the tracked old Kafka bootstrap forward;
`status-forwards` flags a wrong target rather than treating its listening TCP
port as healthy. `doctor` shows the listener/Service mapping and runs bounded
metadata inspection if `kcat` is installed; without it, metadata is explicitly
not checked. `stop-forwards` stops local processes, not the broker or its state.
The generated bootstrap setting stays `127.0.0.1:9092`.

To roll back only this listener change, retain the checked-out manifest change
until an intentional source rollback, and restore the captured listener list:

```bash
just stop-forwards
kubectl patch kafka my-cluster -n kafka --type=merge \
  --patch-file .local/kafka-listeners.before.json
generation="$(kubectl get kafka my-cluster -n kafka -o jsonpath='{.metadata.generation}')"
kubectl wait kafka/my-cluster -n kafka \
  --for="jsonpath={.status.observedGeneration}=${generation}" --timeout=300s
kubectl wait kafka/my-cluster -n kafka --for=condition=Ready --timeout=300s
# Delete only this manually managed Service, never Kafka/PVC/group resources.
kubectl delete service my-cluster-kafka-local-0 -n kafka
kubectl get pvc -n kafka -o wide
```

After rollback, Connect still uses internal 9092, but host Kafka metrics will be
unavailable until a reachable advertised listener is restored. Do not restore
the old bootstrap forward and claim host Kafka connectivity works. Reapplying
the new manifest intentionally enables the local listener again.

#### Offset Meaning and Read-only Observation

Sink lag uses the connector's **Kafka input consumer group**, default
`connect-{connector-name}`; `consumer.override.group.id` takes precedence. This
is not the Connect worker group, a Debezium source offset, an Iceberg control
group, a task's current position, a records/sec counter or delivery latency.
`committed` is the group's next committed input offset; `end` is the broker's
next log-end offset. For valid offsets, partition lag is `end - committed`.
Offset gaps/compaction and transactional visibility mean this offset distance
is not necessarily an exact count of records available to an application.
Commit batching means lag can persist after records are processed.

The collector requests metadata/end offsets and the group's committed offsets;
application code never subscribes, assigns, polls, consumes, commits or joins the
group. librdkafka's destructor may drain close callbacks, but the observer has no
subscription, assignment or payload, and automatic offset storage/commits are
disabled. Requests share an eight-second round deadline with three-second
per-call timeouts; finite client shutdown waits occur outside that deadline.
The application waits 10 seconds between rounds; cached data is stale after
30 seconds. A total is shown only when every configured subscribed partition
has valid known offsets. Missing commits, unavailable groups/topics,
invalid/out-of-range offsets and collection errors are explicit, not zero-filled.
Historical cached values are not current evidence. Connector/group/topic or known
partition-set changes do not combine old and new partitions. Reads are not atomic
during production, commits or rebalances; they measure committed progress without
observing or changing assignment membership.

The current collector connects using plaintext Kafka and validates Kafka's cluster
ID against Connect. Differing bootstrap overrides, consumer security/isolation
overrides, unresolved config-provider values and unsupported Java regex syntax
remain explicitly unavailable. For regex syntax and collection/display bounds,
see [Monitor configuration and limitations](../README.md#monitor-ui).

#### Reproducible Isolated Backlog / Recovery Smoke

These are **execution instructions, not a claim of a completed smoke run**.
Require a healthy deployed sample Iceberg sink/RustFS, `jq`, `kcat`, reachable
host metadata and an available UI `/monitor`. Use a new name per run, and keep
the existing sample source/sink and all existing groups/offsets untouched.
This recipe creates a separate KafkaConnector, two-partition input topic,
Iceberg table, input group and control topic/group.

Iceberg **1.12.0** also needs `iceberg.connect.group-id` equal to
`consumer.override.group.id`: its coordinator describes that input group's
members. Setting only the Connect override can leave Iceberg looking for
`connect-{connector-name}` and fail the task. This plugin-specific setting
does not change the observer's generic mapping rule; see
[the pinned IcebergSinkConfig source](https://github.com/apache/iceberg/blob/apache-iceberg-1.12.0/kafka-connect/kafka-connect/src/main/java/org/apache/iceberg/connect/IcebergSinkConfig.java).

```bash
suffix="$(date +%Y%m%d%H%M%S)"
connector="sink-lag-smoke-${suffix}"
topic="sink-lag-smoke-${suffix}"
group="sink-lag-smoke-group-${suffix}"
broker="my-cluster-dual-role-0"
bootstrap="my-cluster-kafka-bootstrap:9092"
mkdir -p .local
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-topics.sh \
  --bootstrap-server "$bootstrap" --create --topic "$topic" \
  --partitions 2 --replication-factor 1
# Clone only the sample sink's config; no source connector or existing group change.
kubectl get kafkaconnector iceberg-sink-connector -n kafka -o json \
  | jq --arg name "$connector" --arg topic "$topic" --arg group "$group" \
       --arg table "public.sink_lag_smoke_${suffix}" '
      .metadata = {name:$name,namespace:"kafka",
                   labels:{"strimzi.io/cluster":"my-connect-cluster"}}
      | del(.status)
      | .spec.tasksMax = 1
      | .spec.state = "running"
      | .spec.config += {
          "topics":$topic,
          "consumer.override.group.id":$group,
          "consumer.override.auto.offset.reset":"earliest",
          "iceberg.connect.group-id":$group,
          "iceberg.tables":$table,
          "iceberg.control.topic":($name + "-control"),
          "iceberg.control.group-id-prefix":($name + "-control-group"),
          "iceberg.coordinator.transactional.prefix":($name + "-"),
          "iceberg.control.commit.interval-ms":"10000"
        }' > .local/sink-lag-smoke.json
kubectl apply -f .local/sink-lag-smoke.json
kubectl wait kafkaconnector/"$connector" -n kafka \
  --for=condition=Ready --timeout=180s
curl --fail --silent "http://127.0.0.1:8083/connectors/${connector}/status" | jq .
# Seed both partitions so both have real commits before measuring backlog.
for partition in 0 1; do
  printf '{"id":%s,"phase":"seed"}\n' "$partition" \
    | kcat -b 127.0.0.1:9092 -P -t "$topic" -p "$partition"
done
```

Before proceeding, confirm the task is `RUNNING`, both partitions have non-missing
commits, and the UI/group CLI reaches zero lag. Poll the following **read-only**
CLI commands until that baseline is established (allow sink commit cycles):

```bash
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --offsets
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --members --verbose
```

Then pause the **isolated** connector through its operator-managed desired state
(do not use REST pause, which the operator can reconcile away), wait for REST
connector and task `PAUSED`, and create a fixed backlog:

```bash
kubectl patch kafkaconnector "$connector" -n kafka --type=merge \
  -p '{"spec":{"state":"paused"}}'
paused=false
for attempt in $(seq 1 90); do
  if curl --fail --silent "http://127.0.0.1:8083/connectors/${connector}/status" \
    | jq -e '.connector.state == "PAUSED" and (.tasks | length > 0)
             and all(.tasks[]; .state == "PAUSED")' >/dev/null; then
    paused=true
    break
  fi
  sleep 2
done
if [ "$paused" != true ]; then
  echo "Isolated connector did not pause; stop this smoke and inspect task errors."
  exit 1
fi
for partition in 0 1; do
  for id in $(seq 1 50); do
    printf '{"id":%s,"partition":%s,"phase":"backlog"}\n' "$id" "$partition"
  done | kcat -b 127.0.0.1:9092 -P -t "$topic" -p "$partition"
done
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --offsets \
  > .local/sink-lag-smoke.offsets.before.txt
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --members --verbose \
  > .local/sink-lag-smoke.members.before.txt
```

Open `/monitor`, keep it refreshing for at least two 10-second collection rounds,
and record the isolated connector's resolved override group, both committed/end
offsets, partition lag **50 each** and complete total **100**. Run the same two
commands into `.after.txt` files and compare them with `diff -u`. With the
connector paused and no other producer/consumer, commits, member IDs and
partition assignments must remain unchanged; the observer must not add a member
or trigger a rebalance. Save this evidence separately from UI screenshots.
Group descriptions are observations, not proof if another client is changing
the group concurrently.

```bash
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --offsets \
  > .local/sink-lag-smoke.offsets.after.txt
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --members --verbose \
  > .local/sink-lag-smoke.members.after.txt
diff -u .local/sink-lag-smoke.offsets.before.txt .local/sink-lag-smoke.offsets.after.txt
diff -u .local/sink-lag-smoke.members.before.txt .local/sink-lag-smoke.members.after.txt
```

```bash
kubectl patch kafkaconnector "$connector" -n kafka --type=merge \
  -p '{"spec":{"state":"running"}}'
curl --fail --silent "http://127.0.0.1:8083/connectors/${connector}/status" | jq .
# Repeat read-only group/offset checks until both commits reach their end offsets.
kubectl exec -n kafka "$broker" -- /opt/kafka/bin/kafka-consumer-groups.sh \
  --bootstrap-server "$bootstrap" --group "$group" --describe --offsets
```

Verify recovery in both UI and CLI: `RUNNING` task, advancing commits and finally
valid per-partition **0** / complete total **0**. Pause/resume itself can legitimately
change membership; the unchanged-membership comparison belongs to the stable
paused observation window, not across resume. Allow commit cycles and confirm
task errors rather than interpreting missing offsets as recovery.
Stopping the Kafka forward with the cluster otherwise healthy should produce an
explicit collection error/stale cached result, never a synthetic zero; restore
the forward and confirm recovery. Do not reset offsets to manufacture lag.
Retain isolated smoke resources/evidence for diagnosis, or deliberately remove
only that smoke connector after the run; existing resources and PVCs must stay.

#### Issue #22 Live Verification

An isolated Iceberg sink with an explicit input-group override was exercised
against the local Kafka listener. The same Monitor document updated through
HTMX with JMX targets deliberately unconfigured:

| Observed phase | Partition 0 lag | Partition 1 lag | Complete connector lag |
| --- | --- | --- | --- |
| Added empty partition without a commit | 0 | unavailable | unavailable |
| Seed records committed on both partitions | 0 | 0 | 0 |
| Paused sink, produced a dedicated backlog | 20 | 20 | 40 |
| Isolated discovery-forward outage | 20, historical/stale | 20, historical/stale | 40, historical/stale |
| Resumed sink and observed committed recovery | 0 | 0 | 0 |

The paused window's Kafka CLI offset and member/assignment output was byte-for-byte
unchanged across observer collection and repeated browser requests. Recovery
advanced the commits to the corresponding partition end offsets. A subsequent
paused mapping change to a nonexistent input group showed missing commits on
both partitions, did not inherit the former group's totals, and did not register
that group in Kafka. Known end offsets became explicitly historical on discovery
failure even with no complete lag total.

A separate UI instance pointed at an unreachable Kafka bootstrap address showed
collection errors with no numeric lag. Restoring the isolated discovery forward
recovered fresh readings in the same document. Regression tests cover offset
boundaries, incomplete sums, mapping/partition changes, partial-data expiry and
independent freshness when another group is stale. `just rust-check`,
`just rust-test` and core tests without Kafka support passed.

The listener advertises broker 0 as loopback and was verified with `kcat -L`;
rollout preserved Kafka state but caused the single local Connect worker to
restart, requiring its stale REST forward to be repaired. Smoke resources and
temporary UI/forward processes were removed; shared service forwards remain.


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

### Processing Error and Dead-Letter Measurements

The Dashboard's **Processing errors, retries and dead-letter activity** panel
reads the same shared per-worker collection (no per-browser scraping). It
derives one row per exported `(worker, connector, task)` identity. These
measurements cover Connect's error-handling framework, not every exception
inside a connector's own implementation; REST task-state failures remain
separate. Record errors/failures count failed operations, not unique records,
and must not be added together.

Current and previous failure samples are indexed once per report by worker,
metric name and the complete label set; lookups do not rescan the scrape for
each task. Dashboard output is limited to **25 task rows per worker and 200
total**, with an omitted-row count; the shared metrics cache stays complete.
Absent `metrics_targets_file` is shown as **not configured**, distinct from
configured but unavailable measurements. A synthetic HTTPS browser smoke with
10,225 task identities (10,000 on one worker) displayed 200 rows, no more than
25 per worker, and reported all 10,025 omitted identities.

|Panel column|Exporter metric|Meaning|
|---|---|---|
|Record errors|`kafka_connect_task_error_total_record_errors_total`|Operations that could not execute successfully|
|Record failures|`kafka_connect_task_error_total_record_failures_total`|Failed operations, including retriable attempts|
|Retries|`kafka_connect_task_error_total_retries_total`|Operation retries|
|Skipped|`kafka_connect_task_error_total_records_skipped_total`|Records skipped by error handling|
|Logged|`kafka_connect_task_error_total_errors_logged_total`|Errors logged by Connect's error reporter|
|DLQ write attempts|`kafka_connect_task_error_deadletterqueue_produce_requests_total`|Attempted DLQ writes, not successful deliveries|
|DLQ failures|`kafka_connect_task_error_deadletterqueue_produce_failures_total`|DLQ write failures|
|Last error|`kafka_connect_task_error_last_error_timestamp`|Unix epoch **milliseconds**; zero means no recorded error|

Semantics:

- **Counts are cumulative** for the exporter's current metric lifecycle.
  They can reset when Connect recreates the task's metrics. **Rates** compare
  two consecutive available collections using actual elapsed time (the
  collector waits 10 seconds after each round). Only errors, failures and
  retries currently display rates.
- **Counter decreases** suppress that counter's rate for the interval and
  flag the row `counter reset`. New workers/tasks, missing baselines and
  collection failures suppress rates too. Workers, complete label sets,
  connectors and tasks are never merged. A reset followed by enough activity
  to exceed the old counter between scrapes is not observable from counters
  alone; these are sampled rates, not exact event accounting.
- **Availability is explicit**: missing or collection-error series show
  `unavailable`, never a healthy zero. A collection failure is reported as a
  collection failure, never as connector processing activity.
- **DLQ status** uses the already-expanded connector REST config
  (`errors.deadletterqueue.topic.name`); the topic name is never displayed.
  Unset topics and source connectors show `not configured / unsupported`
  instead of numeric DLQ counts, even if Connect exports zeros. Missing DLQ
  measurements or config remain explicitly unavailable. A configured DLQ
  with an exported zero has a real zero, not a fabricated reading.
- **No payloads**: the panel shows only counters and timestamps; record
  payloads, stack traces, and error messages are not newly exposed. Existing
  REST task-state failure details remain in the separate failure list above
  the panel.

Local verification exercised malformed JSON conversion on an isolated Iceberg
sink with `errors.tolerance=all`, strict JSON schema conversion, a pre-created
DLQ topic, `errors.log.include.messages=false`, and
`errors.deadletterqueue.context.headers.enable=false`. Record errors, failures,
skipped/logged counts and DLQ attempts increased only for that connector/task;
the DLQ received the rejected records. Consecutive collections produced
nonzero derived rates. Disabled DLQs remained explicitly not configured.
Counter decreases, worker changes, multiple task identities and stale/error
baselines are covered by deterministic Rust regression tests.
The final browser smoke also verified three malformed records updating only
the correct task row, three records received by its DLQ, disabled DLQ count
cells remaining nonnumeric, initial collection-error recovery, and explicitly
stale historical counts with no rates after a scrape failure. Smoke resources
and the temporary UI server were removed afterwards.

For this local stack, create smoke connectors as **KafkaConnector resources**,
not only through REST: `strimzi.io/use-connector-resources=true` causes the
operator to delete unmanaged REST connectors. Delete only the smoke connector
and its dedicated topics afterwards. Reload Dashboard for the current cached
panel; the existing REST failure list is unchanged.

Metric definitions: [Kafka 4.3.1 ErrorHandlingMetrics](https://github.com/apache/kafka/blob/4.3.1/connect/runtime/src/main/java/org/apache/kafka/connect/runtime/errors/ErrorHandlingMetrics.java).

### Debezium Identity Tags

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
Permanent view regressions additionally cover fresh/disconnected readings,
worker staleness, cache expiry, unavailable/error fields, and streaming row-limit
boundaries (including zero and the 200-row display cap).

The snapshot notification form was retained. A live notification session on this
host reached the existing error surface because Kafka advertises internal broker
DNS names that the host cannot resolve through the localhost bootstrap forward.
Snapshot tracker/session regression tests passed; successful live notification
consumption was not established by this smoke check.


### Debezium Snapshot Measurements and Notifications

Both the Monitor landing page and timed SSE sessions project supported snapshot
measurements from the **same shared worker cache**. No extra scraper or browser
consumer is created for metrics. Availability is independent for each attribute:

| UI measurement | Native attribute / exported suffix | Meaning |
| --- | --- | --- |
| Running / completed / aborted | `SnapshotRunning`, `SnapshotCompleted`, `SnapshotAborted` / `_snapshot_running`, `_snapshot_completed`, `_snapshot_aborted` | Explicit Boolean flags; only 0 or 1 is accepted. |
| Paused / skipped | `SnapshotPaused`, `SnapshotSkipped` / `_snapshot_paused`, `_snapshot_skipped` | Native flags, not inferred from notification silence. |
| Duration (s) | `SnapshotDurationInSeconds` / `_snapshot_duration_in_seconds` | Exporter's retained lifecycle duration; not a browser stopwatch or proof of the newest notification run. |
| Total / remaining tables | `TotalTableCount`, `RemainingTableCount` / `_total_table_count`, `_remaining_table_count` | Exported table inventory; not a row-total estimate or row percentage. |
| Reported table rows scanned | `RowsScanned` / `_rows_scanned` plus `table` label | Historical reported map entries; absent entries are unavailable, not zero. |
| Snapshot events (cumulative) | `TotalNumberOfEventsSeen` / `_total_number_of_events_seen_total` | Snapshot-context event total since exporter lifecycle/reset, not current-run progress or durable Kafka delivery. |

Actual PostgreSQL metric names use the `debezium_postgres` prefix. Other domains
are projected only when the matching attributes are actually exported. Every
scalar requires nonempty `connector`, `task`, `server` and `context` labels;
per-table rows additionally require `table`. Worker and Debezium domain remain
separate. No counts are summed across tables/tasks/workers/contexts. Unsupported
extra dimensions, duplicate series/ownership, nonfinite/negative numbers and
nonboolean flags are unavailable rather than an arbitrary or fabricated value.
Display is bounded independently to **200 identities and 200 table rows across
displayed identities**, with omitted counts; projection uses the complete cache.

The `RowsScanned`-specific exporter rule precedes the generic attribute rule.
The MXBean map becomes `TabularData(key,value)`; JMX Exporter 1.6.0 appends
`key=<table>` to bean properties while retaining the `RowsScanned` attribute.
The rule captures that key as `table` instead of collapsing different tables
into indistinguishable series. Apply the updated exporter ConfigMap, allow
Strimzi reconciliation and mounted-config propagation, and use the existing
state-preserving rolling-update procedure above if the worker needs a restart.
Verify the resulting HTTPS scrape has the table label; an empty native map
legitimately exports no row samples.

#### Context, Run Identity and Source Disagreement

- The installed **Debezium 3.7.0.Final PostgreSQL** pipeline supplies the same
  native `context="snapshot"` meter to initial **and incremental** snapshots.
  This context is shown literally and marked shared; it is **not initial-only**.
  An explicit `incremental-snapshot` context remains separate when an exporter
  actually provides it. Streaming measurements never supply snapshot values.
- Native `SnapshotMeter.snapshotStarted()` resets lifecycle flags/timing but
  **does not clear its row map or captured-table inventory**; only `reset()`
  clears those values. Event counters can also outlive a run. Table rows and
  cumulative events are always labeled retained/historical, never attributed
  to a new notification or converted into a percentage.
- Notifications use native `aggregate_type="Initial Snapshot"` or
  `"Incremental Snapshot"` and retain separate `initial-snapshot` /
  `incremental-snapshot` cards. `additional_data.connector_name` is Debezium's
  **logical source name**, which can differ from the KafkaConnector/REST name.
  For example, the supplied source reports `lakehouse`; its tagged metric
  connector remains `postgres-source-connector`. These names are not conflated.
- For an explicitly separate incremental metric context, comparison matches the
  notification logical source to the exported `server`. Multiple connector/task/
  worker/domain identities claiming that source/context make association
  ambiguous. A complete fresh flag set can show consistent or different
  observations; **even agreement does not establish the same run**. Initial
  notification IDs identify individual events, and incremental IDs have native
  concurrency limitations. Shared `snapshot` context always reports run
  association unavailable rather than overwriting either source.
- Metrics are sampled, so a terminal notification can arrive before the next
  completed metric scrape. Both observations remain visible. Missing metrics do
  not hide notifications; missing/failed notifications do not hide metrics.
  A failed scrape retains last-good measurements as **stale**; a cache older
  than 30 seconds also expires readings. Every stale flag/value is last observed,
  not live. A first collection failure contains no numeric measurements.
- Timed sessions retain the latest lifecycle plus the last 20 accepted wire
  events per logical source/context. Collection/table/chunk events do not
  replace lifecycle status. Terminal events are visible even without `STARTED`;
  newer starts clear old run timing/progress, and older timestamped events do
  not rewind lifecycle state. No history is persisted across sessions.
- Stop, deadline, navigation and SSE disconnect end the viewer's subscription;
  the application-wide metric collector remains independent. Notification poll
  errors stop only that consumer while cached metrics continue to the deadline.
  SSE transport failures do not automatically reconnect.

#### Real Snapshot Browser Smoke

Issue #21 used a dedicated KafkaConnector, replication slot/publication, a
1,500-row PostgreSQL table and a source signaling table; no existing offsets or
data were reset. A slow initial snapshot updated one open timed Monitor document:
running became true, duration advanced **3 → 13 → 61 seconds**, total/remaining
tables went **1/1 → 1/0**, completed became true, and the actual table-labeled
rows/event readings reached **1,500**. No percentage/progress bar was displayed.

Real incremental signals subsequently produced independent incremental
notification lifecycle cards, alongside the retained initial `SKIPPED` card.
The shared native metric context remained `snapshot`; an intermediate cached
metric observation still showed running/1,200 scanned rows after the completed
notification arrived, without treating either source as the authoritative
replacement. Two incremental snapshots left **3,000 cumulative events** versus
**1,500 reported table rows**, explicitly not a new-run percentage.

A separate UI with an unreachable HTTPS target displayed real incremental
`COMPLETED` notifications while snapshot collection remained **collection error**
with no fabricated numbers. On the healthy UI, an isolated target failure showed
last-observed/stale flags and counts without hiding notifications; restoring the
target recovered in the same document. Conversely, the host's unresolved
advertised Kafka DNS produced a visible notification error while the real initial
snapshot continued updating via metrics.

Successful notification smoke used a **temporary QA-only loopback metadata
relay** to make the existing forwarded broker's advertised addresses reachable;
no broker/listener/DNS configuration or application transport behavior was
changed. A separate temporary HTTP relay exercised a genuine SSE transport
disconnect. Stop and 10-second deadline retained final observations; disconnect
disabled Stop, retained metric rows and did not reconnect. Broker group
inspection showed zero members after these sessions. Long raw notification
payloads were browser-checked without horizontal page overflow.
Smoke-only connectors/tables/topics/slots/publications, relays, UI processes and
temporary configs were removed after verification. Existing private forwards
and the updated exporter configuration were retained.

Snapshot definitions:
- [Debezium 3.7 native snapshot attribute contract](https://github.com/debezium/debezium/blob/v3.7.0.Final/debezium-connector-common/src/main/java/io/debezium/pipeline/metrics/traits/SnapshotMetricsMXBean.java).
- [Native snapshot-meter lifetime/reset semantics](https://github.com/debezium/debezium/blob/v3.7.0.Final/debezium-connector-common/src/main/java/io/debezium/pipeline/meters/SnapshotMeter.java).
- [Shared incremental snapshot listeners](https://github.com/debezium/debezium/blob/v3.7.0.Final/debezium-connector-common/src/main/java/io/debezium/pipeline/ChangeEventSourceCoordinator.java#L374-L375).
- [Native notification format and lifecycle](https://debezium.io/documentation/reference/3.7/configuration/notification.html).
- [Initial notification logical-name identity](https://github.com/debezium/debezium/blob/v3.7.0.Final/debezium-connector-common/src/main/java/io/debezium/pipeline/notification/InitialSnapshotNotificationService.java).
- [JMX Exporter 1.6.0 map flattening](https://github.com/prometheus/jmx_exporter/blob/1.6.0/collector/src/main/java/io/prometheus/jmx/JmxScraper.java#L520-L601).

Primary configuration references:
- [Strimzi 1.2.0 Kafka Connect exporter example](https://github.com/strimzi/strimzi-kafka-operator/blob/1.2.0/examples/metrics/kafka-connect-metrics.yaml).
- [JMX Exporter 1.6.0 rule input, ordering, labels and snake-case semantics](https://prometheus.github.io/jmx_exporter/1.6.0/configuration/rules/).
- [JMX Exporter 1.6.0 HTTP TLS configuration](https://prometheus.github.io/jmx_exporter/1.6.0/configuration/ssl/).
- [1.6.0 keystore password variable-resolution call](https://github.com/prometheus/jmx_exporter/blob/1.6.0/jmx_prometheus_common/src/main/java/io/prometheus/jmx/common/HTTPServerFactory.java#L1391-L1400) and [exact `${ENV_NAME}` resolver syntax](https://github.com/prometheus/jmx_exporter/blob/1.6.0/collector/src/main/java/io/prometheus/jmx/variable/VariableResolver.java).
- [Debezium PostgreSQL custom metric tags and monitoring](https://debezium.io/documentation/reference/3.7/connectors/postgresql.html#postgresql-monitoring).
- [Strimzi 1.2.0 generated Connect NetworkPolicy](https://github.com/strimzi/strimzi-kafka-operator/blob/1.2.0/cluster-operator/src/main/java/io/strimzi/operator/cluster/model/KafkaConnectCluster.java).

### Queue and Backpressure Measurements

Monitor projects the Debezium queue and common Connect active-record metrics
below. Utilization is derived here, never exported:

| UI measurement | Native metric | Exported measurement | Meaning |
| --- | --- | --- | --- |
| Queue total capacity | Debezium `QueueTotalCapacity` (snapshot and streaming contexts) | `debezium_<domain>_queue_total_capacity` | Records the connector queue is designed to hold; record unit. |
| Remaining capacity | Debezium `QueueRemainingCapacity` | `debezium_<domain>_queue_remaining_capacity` | Free records; the queue is shared between snapshot/streaming threads and the Connect poll loop. |
| Records in queue / utilization | Derived | — | `(total − remaining) / total`, computed per connector/task/server/context row. |
| Max queue bytes | Debezium `MaxQueueSizeInBytes` | `debezium_<domain>_max_queue_size_in_bytes` | Shown exactly as the connector exports it. Debezium documents it as available when `max.queue.size.in.bytes` is configured positively; a real exported zero (common when unset) or a missing family stays raw, while the supplied exporter emits a gauge like any Debezium attribute. |
| Current queue bytes | Debezium `CurrentQueueSizeInBytes` | `debezium_<domain>_current_queue_size_in_bytes` | Measured queue volume in bytes; derived byte utilization divides it by the maximum and additionally requires a positive maximum. |
| Active records | Connect `SourceRecordActiveCount` / `SinkRecordActiveCount` | `kafka_connect_source_task_source_record_active_count`, `kafka_connect_sink_task_sink_record_active_count` | Records in the task pipeline not yet fully committed. This is **not** Kafka consumer lag. |
| Active peak / average (windowed) | Connect `SourceRecordActiveCountMax/Avg`, `SinkRecordActiveCountMax/Avg` | `..._active_count_max`, `..._active_count_avg` | Backed by Kafka's sampled `Max`/`Avg` stats: the peak can decay as metric samples expire, so neither is a lifetime value and neither is a current backlog. |
| Max Kafka consumer lag | Connect `SinkRecordLagMax` | `kafka_connect_sink_task_sink_record_lag_max` | Kafka consumer-group lag in records for the sink task's partitions. Shown only for sink rows, always labeled as consumer lag. |

Behaviors mirror the other panels: zero or inconsistent capacity (remaining
above total), negative/nonfinite, extra-dimension (custom metric tags) or
duplicate worker/series ownership mark the affected readings unavailable while
keeping row identities and genuine values visible; collection errors never
expose values; stale workers retain last good values marked stale. Snapshot and
streaming contexts each keep their own labeled row instead of silently choosing
a winner, and nothing is summed across queues or tasks. No connector behavior is
altered from these readings.

See [Kafka 4.3 Connect metric definitions](https://kafka.apache.org/43/generated/connect_metrics.html)
and the [Debezium PostgreSQL monitoring tables](https://debezium.io/documentation/reference/3.7/connectors/postgresql.html#postgresql-monitoring).

#### Induced Backlog Smoke Check

The changing signal on this local stack is the sink task's **active-record
count and windowed peak/average**, not consumer lag: the local Iceberg sink
does not perform the Kafka range lookups that export `sink-record-lag-max`, so
that column stays explicitly **unavailable** — never zero. On other sinks that
export lag, a paused consumer instead shows lag growing and recovering.

1. Run `just port-forward-all`, `just sync-secrets`, then `just ui`. Confirm the
   PostgreSQL source task and the Iceberg sink task are running, open
   `/monitor`, and record the queue, active-record and lag columns.
2. Pause the Iceberg sink through the Control page (`/control/<sink>/pause`) so
   it stops consuming CDC topics while the source keeps producing. A paused sink
   does not write data; unchanged Debezium queue utilization is expected, and
   the lag column is expected to stay **unavailable** on this sink.
3. Insert a uniquely tagged small batch into `source_db.public.users` with the
   collision-safe insert from the throughput smoke check. Within collection and
   browser refresh cycles watch the **sink active-record count** rise while
   throughput read/sent rates fall toward zero and source rates stay positive —
   accumulating work is visible before any connector fails.
4. Resume the sink (`/control/<sink>/resume`). Confirm the active-record count
   drains toward zero (the windowed peak keeps the observed maximum until the
   sample expires), sink read/sent rates recover, and no queue utilization,
   active record or connector state change was triggered by the panel itself.
5. Clean up the smoke rows with the matching collision-safe delete and recheck
   the panel again.

Issue #20 live smoke (local `my-connect-cluster:0.0.4` stack): the paused Iceberg
sink showed the panel changing instead of zero-filling — sink
**active records rose to 5 (available)** with max consumer lag explicitly
**unavailable** (the local Iceberg sink does not export
`sink-record-lag-max` while its flow is constrained), source queue
8192/8192 records stayed at utilization 0.00 with byte readings kept as
exported (`MaxQueueSizeInBytes` 0, byte utilization **unavailable**) because
the local connector configures no
`max.queue.size.in.bytes`. After resume, the same document showed sink active
records **drained to 0 (available)** while the windowed peak kept 5 and
the windowed average recovered; source rates stayed positive throughout, and no
panel reading altered connector behavior. This smoke check did not
demonstrate nonzero queue utilization on this idle single-task source; the
derived-fraction behavior (0.50 records, 0.25 bytes, zero/invalid capacity) is
covered by `crates/strimzi-ops-core/src/metrics/backpressure.rs` unit tests
and the TLS-route integration test in `crates/strimzi-ui/tests/http_routes.rs`.

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
