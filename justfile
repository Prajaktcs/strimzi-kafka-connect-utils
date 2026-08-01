# Strimzi Ops Platform — https://github.com/casey/just
# Run `just` to list recipes.
#
# One-command local stack:
#   just setup

set shell := ["bash", "-euo", "pipefail", "-c"]

namespace := "kafka"
connect_image := "my-connect-cluster:0.0.2"
helper := "scripts/local-dev.sh"

default:
    @just --list

# One-command local setup: cluster, deps, image, deploy, secrets, connector, port-forwards, UI
setup: install ensure-cluster build-connect deploy sync-secrets apply-connector port-forward-all
    @echo ""
    @echo "Local stack is ready."
    @echo "  Connect API : http://localhost:8083"
    @echo "  Kafka       : localhost:9092"
    @echo "  Postgres    : localhost:5432"
    @echo "  Garage S3   : http://localhost:3900"
    @echo "  Nessie      : http://localhost:19120"
    @echo ""
    @echo "Starting Streamlit UI..."
    uv run streamlit run app.py

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

# Build local Kafka Connect image (Debezium Postgres plugin)
build-connect:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Building Connect image {{ connect_image }}..."
    if docker buildx version >/dev/null 2>&1; then
      docker buildx build --load -t {{ connect_image }} -f k8s/Dockerfile.connect k8s/
    else
      # Colima / Docker without the buildx plugin — legacy builder
      DOCKER_BUILDKIT=0 docker build -t {{ connect_image }} -f k8s/Dockerfile.connect k8s/
    fi
    echo "Image {{ connect_image }} ready."

# Deploy local Kubernetes environment
deploy:
    cd k8s && ./deploy.sh

# Check deployment status
status:
    cd k8s && ./status.sh

# Destroy local environment (also stops port-forwards)
destroy: stop-forwards
    cd k8s && ./destroy.sh

# Write secrets.toml from Garage setup job credentials
sync-secrets:
    bash {{ helper }} sync-secrets

# Deploy the sample Debezium Postgres KafkaConnector
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
    @echo "Garage S3 → http://localhost:3900 (Ctrl+C to stop)"
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
