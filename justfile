# Strimzi Ops Platform — https://github.com/casey/just
#
# One command:
#   just setup
#
# That will:
#   1. uv sync
#   2. start Colima+k8s if needed
#   3. ensure docker buildx (Homebrew plugin; avoids legacy builder warning)
#   4. build Connect image with Debezium + Iceberg sink
#   5. deploy Strimzi 1.1.0 + Kafka 4.3.0 + Postgres + Garage 2.3 + Nessie
#      (wipes legacy v1beta2 Strimzi CRDs if present; installs operator into kafka ns)
#   6. write secrets.toml with local-dev Garage keys
#   7. apply sample Postgres source + Iceberg sink connectors
#   8. start background port-forwards
#   9. launch Streamlit UI
#
# Tear down:
#   just destroy        # app namespace (+ stop port-forwards)
#   just destroy-hard   # also delete Strimzi CRDs (clean major upgrades)

set shell := ["bash", "-euo", "pipefail", "-c"]

# --- Local stack pins (keep in sync with k8s/ manifests) ---
namespace := "kafka"
strimzi_version := "1.1.0"
kafka_version := "4.3.0"
debezium_version := "3.6.0.Final"
iceberg_connect_version := "1.9.2"
connect_image := "my-connect-cluster:0.0.3"
helper := "scripts/local-dev.sh"

# Garage 2.3 --single-node --default-bucket credentials (local-dev only).
# Must match env in k8s/04-garage.yaml.
garage_access_key := "GKaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
garage_secret_key := "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
garage_bucket := "warehouse"
garage_endpoint := "http://localhost:3900"

default:
    @just --list

# One-command local setup → infra + secrets + connector + port-forwards + UI
setup: install ensure-cluster build-connect deploy sync-secrets apply-connector port-forward-all
    @echo ""
    @echo "Local stack is ready."
    @echo "  Connect API : http://localhost:8083"
    @echo "  Kafka       : localhost:9092"
    @echo "  Postgres    : localhost:5432  (postgres / password / source_db)"
    @echo "  Garage S3   : {{ garage_endpoint }}"
    @echo "  Nessie      : http://localhost:19120"
    @echo "  UI          : http://localhost:8501"
    @echo ""
    @echo "Stack: Strimzi {{ strimzi_version }} / Kafka {{ kafka_version }} / Debezium {{ debezium_version }}"
    @echo "Starting Streamlit UI..."
    uv run streamlit run app.py

# Alias for setup
up: setup

# Install Python dependencies with uv
install:
    @echo "Installing Python dependencies with uv..."
    uv sync
    @echo "Dependencies installed."

# Alias for install
sync: install

# Start Colima+k8s if needed, then verify kubectl
ensure-cluster:
    bash {{ helper }} ensure-cluster

# Ensure docker buildx plugin works (fixes Docker Desktop→Colima broken symlink)
ensure-buildx:
    bash {{ helper }} ensure-buildx

# Build local Kafka Connect image (Debezium Postgres + Iceberg sink) via BuildKit
build-connect: ensure-buildx
    @echo "Building Connect image {{ connect_image }} (Debezium {{ debezium_version }}, Iceberg {{ iceberg_connect_version }})..."
    docker buildx build --load -t {{ connect_image }} -f k8s/Dockerfile.connect k8s/
    @echo "Image {{ connect_image }} ready."

# Deploy local Kubernetes environment (Strimzi/Kafka/Postgres/Garage/Nessie/Connect)
deploy:
    cd k8s && ./deploy.sh

# Check deployment status
status:
    cd k8s && ./status.sh

# Quick health: pods + Connect/UI HTTP checks
doctor:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Pods in {{ namespace }}:"
    kubectl get pods -n {{ namespace }} || true
    echo ""
    echo -n "Connect :8083 → "; curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8083/ || echo "down"
    echo -n "UI      :8501 → "; curl -s -o /dev/null -w "%{http_code}\n" http://localhost:8501/ || echo "down"
    echo -n "Garage  :3900 → "; curl -s -o /dev/null -w "%{http_code}\n" http://localhost:3900/ || echo "down"
    echo -n "Nessie  :19120 → "; curl -s -o /dev/null -w "%{http_code}\n" http://localhost:19120/ || echo "down"

# Destroy local environment (also stops port-forwards)
destroy: stop-forwards
    cd k8s && ./destroy.sh

# Alias for destroy
down: destroy

# Destroy including Strimzi CRDs (needed after failed 0.x→1.x upgrades)
destroy-hard: stop-forwards
    cd k8s && DESTROY_CRDS=1 ./destroy.sh

# Write secrets.toml with local-dev Garage credentials from this justfile
sync-secrets:
    bash {{ helper }} sync-secrets "{{ garage_access_key }}" "{{ garage_secret_key }}" "{{ garage_bucket }}" "{{ garage_endpoint }}"

# Deploy the sample Debezium Postgres source + Iceberg sink connectors
apply-connector:
    bash {{ helper }} apply-connector

# Start all local port-forwards in the background
port-forward-all:
    bash {{ helper }} port-forward-all

# Stop background port-forwards started by setup / port-forward-all
stop-forwards:
    bash {{ helper }} stop-forwards

# Forward Kafka Connect REST API (8083) — foreground
port-forward:
    @echo "Kafka Connect API → http://localhost:8083 (Ctrl+C to stop)"
    kubectl port-forward svc/my-connect-cluster-connect-api 8083:8083 -n {{ namespace }}

# Forward Kafka bootstrap (9092) — foreground
port-forward-kafka:
    @echo "Kafka bootstrap → localhost:9092 (Ctrl+C to stop)"
    kubectl port-forward svc/my-cluster-kafka-bootstrap 9092:9092 -n {{ namespace }}

# Forward PostgreSQL (5432) — foreground
port-forward-postgres:
    @echo "PostgreSQL → localhost:5432 (user: postgres / password / db: source_db)"
    kubectl port-forward svc/postgres 5432:5432 -n {{ namespace }}

# Forward Garage S3 API (3900) — foreground
port-forward-garage:
    @echo "Garage S3 → {{ garage_endpoint }} (Ctrl+C to stop)"
    kubectl port-forward svc/garage 3900:3900 -n {{ namespace }}

# Forward Nessie catalog API (19120) — foreground
port-forward-nessie:
    @echo "Nessie → http://localhost:19120 (Ctrl+C to stop)"
    kubectl port-forward svc/nessie 19120:19120 -n {{ namespace }}

# Start Streamlit UI (assumes setup / port-forward-all already ran)
run:
    @echo "Starting Strimzi Ops Platform..."
    uv run streamlit run app.py

# Lint a connector config file
lint-config file:
    uv run strimzi-lint lint {{ file }}

# Run tests
test:
    @echo "Running tests..."
    uv run pytest tests/ -v

# Format Python code with black
format:
    @echo "Formatting Python code..."
    uv run black strimzi_ops/ app.py

# Lint Python code with ruff
lint:
    @echo "Linting Python code..."
    uv run ruff check strimzi_ops/ app.py

# Run format check + ruff
check:
    @echo "Running code quality checks..."
    uv run black --check strimzi_ops/ app.py
    uv run ruff check strimzi_ops/ app.py
    @echo "All checks passed!"
