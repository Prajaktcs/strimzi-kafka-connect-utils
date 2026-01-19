### Updated Technical Specification: Strimzi Ops Platform (v3)

## 1. Project Overview

**Name:** `Strimzi-ops`
**Goal:** A unified platform to validate, monitor, and control a Debezium-based Data Strimzi.
**Core Features:**

- **Validator:** Static analysis of connector configs (Pydantic).
- **Monitor:** Real-time snapshot tracking via Debezium Notifications.
- **Control:** Restart/Pause/Resume connectors and trigger Snapshots.

## 2. Infrastructure Stack (Updated)

- **Broker:** **Redpanda** (Kafka API compatible, C++).
- **Object Storage:** **Garage** (S3 compatible, Rust).
- **Database:** Postgres 15 (Source & Sink).
- **Processing:** Kafka Connect (Debezium + Iceberg + JDBC).
- **App Logic:** Python 3.10+ (Streamlit + Confluent Kafka + Pydantic).

## 3. Local Development Setup (Docker)

Here is your updated `docker-compose.yaml`.

- **Change 1:** Replaced `minio` with `garage`.
- **Change 4:** Added a `configuration` step for Garage (buckets/keys) because it doesn't default to `admin/password` like MinIO.

```yaml
version: "3.8"

services:
  # --- 1. Infrastructure: Redpanda (Kafka Broker) ---
  redpanda:
    image: docker.redpanda.com/redpanda/redpanda:v23.2.14
    container_name: redpanda
    ports:
      - "9092:9092"
      - "9644:9644"
    command:
      - redpanda start
      - --smp 1
      - --memory 1G
      - --mode dev-container
      - --kafka-addr internal://0.0.0.0:29092,external://0.0.0.0:9092
      - --advertise-kafka-addr internal://redpanda:29092,external://localhost:9092
    healthcheck:
      test: ["CMD-SHELL", "rpk cluster health | grep -q 'Healthy: true'"]
      interval: 10s
      timeout: 5s
      retries: 5

  # --- 2. Storage: Garage (Rust S3) ---
  garage:
    image: dxflrs/garage:v0.9.1
    container_name: garage
    ports:
      - "3900:3900" # S3 API
      - "3902:3902" # Web Admin
    environment:
      # Garage requires a config file or env vars. We use a simple env setup for dev.
      RUST_LOG: info
    volumes:
      - ./config/garage.toml:/etc/garage.toml
      - garage_data:/var/lib/garage
    command: ["/garage", "server"]

  # --- 3. Setup Helper (Create Bucket/Keys in Garage) ---
  # Garage requires CLI interaction to create keys/buckets.
  # We run this one-off container to provision the dev environment.
  setup-garage:
    image: dxflrs/garage:v0.9.1
    depends_on:
      - garage
    entrypoint: >
      /bin/sh -c "
      sleep 5;
      # 1. Initialize layout (single node)
      /garage layout assign -z dc1 -c 1g localhost || true;
      /garage layout apply --version 1 || true;
      # 2. Create Key (Access/Secret) - We grep specifically for the key output
      # Note: In a real script, parsing is cleaner. Here we hardcode for simplicity or print to logs.
      /garage key create Strimzi-key || true;
      /garage bucket create warehouse || true;
      /garage bucket allow warehouse --read --write --key Strimzi-key;
      echo 'Garage Setup Complete. Check logs for Keys if dynamic.';
      "
    environment:
      RPC_HOST: garage:3901

  # --- 4. Database: Source (CDC Enabled) ---
  postgres-source:
    image: debezium/postgres:15
    container_name: postgres-source
    ports:
      - "5432:5432"
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: password
      POSTGRES_DB: source_db

  # --- 5. Processing: Kafka Connect ---
  connect:
    build: ./connect-image
    container_name: connect
    depends_on:
      redpanda:
        condition: service_healthy
    ports:
      - "8083:8083"
    environment:
      CONNECT_BOOTSTRAP_SERVERS: "redpanda:29092"
      CONNECT_REST_PORT: 8083
      CONNECT_GROUP_ID: "connect-cluster"
      CONNECT_CONFIG_STORAGE_TOPIC: "connect-configs"
      CONNECT_OFFSET_STORAGE_TOPIC: "connect-offsets"
      CONNECT_STATUS_STORAGE_TOPIC: "connect-status"
      CONNECT_KEY_CONVERTER: "org.apache.kafka.connect.json.JsonConverter"
      CONNECT_VALUE_CONVERTER: "org.apache.kafka.connect.json.JsonConverter"
      CONNECT_REST_ADVERTISED_HOST_NAME: "connect"
      CONNECT_KEY_CONVERTER_SCHEMAS_ENABLE: "false"
      CONNECT_VALUE_CONVERTER_SCHEMAS_ENABLE: "false"
      # S3/Garage Configs (Passed to Connectors)
      AWS_ACCESS_KEY_ID: "REPLACE_WITH_GENERATED_KEY_FROM_SETUP"
      AWS_SECRET_ACCESS_KEY: "REPLACE_WITH_GENERATED_SECRET_FROM_SETUP"

volumes:
  garage_data:
```

### 4. Configuration for Garage (`config/garage.toml`)

You need to mount this file to configure Garage to open the S3 port. Create `config/garage.toml`:

```toml
metadata_dir = "/var/lib/garage/meta"
data_dir = "/var/lib/garage/data"

[replication_mode]
mode = "none" # Single node for local dev

[s3_api]
s3_region = "us-east-1"
api_bind_addr = "[::]:3900" # Bind S3 to port 3900
root_domain = ".s3.local"

[s3_web]
bind_addr = "[::]:3902"
root_domain = ".web.local"
enabled = true

[rpc]
bind_addr = "[::]:3901"

```

### 5. Updated `secrets.toml` for the Python Tool

Since Garage runs on port `3900` (not `9000`), update your secrets:

```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"

[storage]
type = "s3"
endpoint_url = "http://localhost:3900" # Pointing to Garage
access_key = "YOUR_GARAGE_KEY"
secret_key = "YOUR_GARAGE_SECRET"
bucket = "warehouse"

```

This setup gives you a purely **Rust + C++** infrastructure layer (Redpanda + Garage), which is incredibly efficient and modern compared to the old Java/Go stack (Kafka/MinIO).

For a quick demonstration of setting up a local development environment with Docker and Kafka (Redpanda), check out this video: [Kafka & Docker for Local Development](https://www.google.com/search?q=https://www.youtube.com/watch%3Fv%3DF07gB3FqNDQ)

This video walks through the practical aspects of running Kafka (via Redpanda) in Docker Compose, reinforcing the setup steps above.
