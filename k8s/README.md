# Kubernetes Deployment for Local Development

This directory contains Kubernetes manifests and scripts to deploy a complete Kafka Connect environment locally using Strimzi.

## What Gets Deployed

1. **Strimzi Operator** - v1.1.0 - Kubernetes operator for managing Kafka
2. **Kafka Cluster** - v4.3.0 - Single-node cluster using **KRaft mode** (ZooKeeper-less)
3. **Kafka Connect** - v4.3.0 - With Debezium 3.6.0 (PostgreSQL connector)
4. **PostgreSQL** - v18.4-alpine - Database configured for CDC (Change Data Capture)
5. **RustFS** - **1.0.1** - S3-compatible storage with built-in Iceberg REST catalog (preview)

## Prerequisites

- Kubernetes cluster (Colima, Minikube, kind, or Docker Desktop)
- kubectl installed and configured
- [just](https://github.com/casey/just) (`brew install just`)
- Minimum 4GB RAM allocated to your cluster

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
colima start --kubernetes --cpu 4 --memory 4

# Verify
kubectl cluster-info
```

#### Minikube

```bash
minikube start --cpus 4 --memory 4096

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
- Deploy Kafka Connect with custom image (built locally)
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

Port forward to access from localhost:

```bash
just port-forward
just port-forward-kafka
just port-forward-postgres
just port-forward-rustfs
```

Then configure `secrets.toml`:
```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"

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

Verified locally with RustFS **1.0.1**, Iceberg Connect **1.9.2**, and DuckDB
**1.5.6**: two exact CDC rows were read through the catalog and S3, survived a
RustFS pod restart and initializer rerun, and a third insert committed after
restart. Legacy Garage/Nessie PVCs remained bound; their services were stopped.

## Manifest Details

### 00-namespace.yaml
Creates dedicated `kafka` namespace for all resources.

### 01-postgres.yaml
Deploys PostgreSQL 18.4 Alpine with logical replication enabled.

### 02-kafka.yaml
Deploys single-node Kafka 4.3.0 cluster via Strimzi using **KRaft** and **KafkaNodePool**.

### 03-kafka-connect.yaml
Deploys Kafka Connect with Debezium PostgreSQL + Iceberg sink connectors. Uses a custom local image `my-connect-cluster:0.0.3`.

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
