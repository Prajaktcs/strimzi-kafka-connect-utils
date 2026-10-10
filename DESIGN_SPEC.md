# Technical Specification: Strimzi Ops Platform

## 1. Project Overview

**Name:** `Strimzi-ops`
**Goal:** Validate, monitor, and control Debezium-based Kafka Connect pipelines.

- **Validator:** Static connector configuration analysis (`strimzi-ops-core` / `strimzi-lint`).
- **Monitor:** Snapshot tracking via Debezium notifications.
- **Control:** Restart, pause, resume, and trigger connector snapshots.
- **Interfaces:** Rust CLI and Axum/Askama/HTMX UI (`strimzi-ops` / `strimzi-ui`).

## 2. Current Local Kubernetes Architecture

The supported local deployment uses the manifests and scripts under `k8s/`, not Docker Compose. All runtime services live in namespace `kafka`.

| Component | Version / role |
| --- | --- |
| Strimzi | 1.1.0 operator |
| Kafka | 4.3.0, single-node KRaft |
| Kafka Connect | 4.3.0, custom local image with Debezium 3.6.0 and Iceberg sink |
| PostgreSQL | 18.4 Alpine, CDC source |
| RustFS | **1.0.1**, S3 storage and built-in Iceberg REST catalog |
| Application | Rust core library, CLI, and web UI |

Data flows from PostgreSQL through Debezium into Kafka, then through the Iceberg sink into RustFS. RustFS owns both object storage and the Iceberg REST catalog; there is no separate catalog service.

`k8s/04-rustfs.yaml` supplies the RustFS workload, persistent storage, and service. `k8s/05-rustfs-init.yaml` supplies the `rustfs-init` Job that initializes the `warehouse` bucket. The REST warehouse/prefix is also `warehouse`.

## 3. Endpoints and Authentication

| Interface | In-cluster endpoint | Port-forward endpoint |
| --- | --- | --- |
| Connect REST | `http://my-connect-cluster-connect-api:8083` | `http://localhost:8083` |
| RustFS S3 | `http://rustfs:9000` | `http://localhost:9000` |
| Iceberg REST | `http://rustfs:9000/iceberg` | `http://localhost:9000/iceberg` |
| RustFS console | `http://rustfs:9001` | `http://localhost:9001` |

Kafka and PostgreSQL use local forwarded ports `9092` and `5432`; the UI uses `8501`.

Fixed credentials `rustfsadmin` / `rustfsadmin` are **LOCAL DEV ONLY**. Both the REST catalog and S3FileIO require credentials. Connector catalog settings are:

```json
{
  "iceberg.catalog.type": "rest",
  "iceberg.catalog.uri": "http://rustfs:9000/iceberg",
  "iceberg.catalog.warehouse": "warehouse",
  "iceberg.catalog.prefix": "warehouse",
  "iceberg.catalog.rest.sigv4-enabled": true,
  "iceberg.catalog.rest.signing-name": "s3",
  "iceberg.catalog.rest.signing-region": "us-east-1",
  "iceberg.catalog.rest.access-key-id": "rustfsadmin",
  "iceberg.catalog.rest.secret-access-key": "rustfsadmin",
  "iceberg.catalog.io-impl": "org.apache.iceberg.aws.s3.S3FileIO",
  "iceberg.catalog.client.region": "us-east-1",
  "iceberg.catalog.s3.endpoint": "http://rustfs:9000",
  "iceberg.catalog.s3.access-key-id": "rustfsadmin",
  "iceberg.catalog.s3.secret-access-key": "rustfsadmin",
  "iceberg.catalog.s3.path-style-access": true
}
```

The Ops tool's host-side storage configuration is separate from the connector's REST authentication:

```toml
[storage]
type = "s3"
endpoint_url = "http://localhost:9000"
access_key = "rustfsadmin"
secret_key = "rustfsadmin"
bucket = "warehouse"
```

## 4. Local Setup and Limits

Run `just setup` from the project root to build and deploy the stack, write local secrets, apply sample connectors, start port-forwards, and launch the UI. See [the deployment guide](k8s/README.md) for individual commands.

RustFS is pinned to **1.0.1** rather than a floating tag. Its [S3 Tables / Iceberg REST catalog support](https://docs.rustfs.com/en/administration/data/s3-tables) is a preview feature. This single-node HTTP stack with shared administrator credentials is for local development, not production. Preview support does not imply complete Iceberg REST compatibility; runtime testing must establish compatibility for the repository's specific connector path.

The sample sink targets `public.users` from `lakehouse.public.users` and enables table auto-creation. The source requires `source_db.public.users`; connector deployment does not create or seed the source table. JSON converters disable schemas and preserve the Debezium CDC envelope.

## 5. Fresh Warehouse Cutover and Rollback

Replacing Garage and Nessie starts a **fresh RustFS warehouse**. It does not import legacy objects, catalog metadata, or existing Iceberg tables automatically.

Deployment retains the legacy Garage/Nessie PVCs. Preserve those volumes and save the old connector configurations before cutover. Namespace destruction or explicit PVC deletion can remove the rollback data.

To roll back, restore the previous git revision, redeploy its Garage/Nessie stack using the retained volumes, and restore the corresponding connector configurations. Do not point the old catalog at the new RustFS warehouse as a substitute for migration.

Connector offsets are not reset automatically on cutover or rollback. Already-consumed records will not be replayed simply by switching catalog endpoints. Any replay or offset reset requires a separate explicit decision, including consideration of duplicate writes and the desired source history.
