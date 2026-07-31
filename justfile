# Strimzi Ops Platform — https://github.com/casey/just
# Run `just` to list recipes.

set shell := ["bash", "-euo", "pipefail", "-c"]

namespace := "kafka"
connect_image := "my-connect-cluster:0.0.1"

default:
    @just --list

# Complete setup: Python deps, Connect image, K8s deploy, secrets.toml
setup: install ensure-cluster build-connect deploy secrets
    @echo ""
    @echo "Setup complete!"
    @echo ""
    @echo "Next steps:"
    @echo "1. Port-forward (separate terminals):"
    @echo "   just port-forward"
    @echo "   just port-forward-kafka"
    @echo "   just port-forward-postgres"
    @echo "   just port-forward-garage"
    @echo "   just port-forward-nessie"
    @echo "2. Paste Garage Key ID / Secret Key from deploy output into secrets.toml"
    @echo "3. Start the UI: just run"
    @echo "4. Optional source connector: kubectl apply -f k8s/test-connector.yaml"
    @echo ""

# Install Python dependencies with uv
install:
    @echo "Installing Python dependencies with uv..."
    uv sync
    @echo "Dependencies installed."

# Alias for install
sync: install

# Fail fast if kubectl cannot reach a cluster
ensure-cluster:
    #!/usr/bin/env bash
    if ! command -v kubectl >/dev/null 2>&1; then
      echo "kubectl not found. Install kubectl first."
      exit 1
    fi
    if ! kubectl cluster-info >/dev/null 2>&1; then
      echo "Cannot connect to Kubernetes."
      echo "Start a cluster first, e.g.: colima start --kubernetes --cpu 4 --memory 4"
      exit 1
    fi
    echo "Connected to Kubernetes cluster"

# Build local Kafka Connect image (Debezium Postgres plugin)
build-connect:
    @echo "Building Connect image {{ connect_image }}..."
    docker build -t {{ connect_image }} -f k8s/Dockerfile.connect k8s/
    @echo "Image {{ connect_image }} ready."

# Deploy local Kubernetes environment
deploy:
    cd k8s && ./deploy.sh

# Check deployment status
status:
    cd k8s && ./status.sh

# Destroy local environment
destroy:
    cd k8s && ./destroy.sh

# Create secrets.toml from example if missing
secrets:
    #!/usr/bin/env bash
    if [[ -f secrets.toml ]]; then
      echo "secrets.toml already exists"
    else
      cp secrets.toml.example secrets.toml
      echo "Created secrets.toml from secrets.toml.example"
      echo "Update [storage] access_key/secret_key with Garage keys from deploy output."
    fi

# Forward Kafka Connect REST API (8083)
port-forward:
    @echo "Kafka Connect API → http://localhost:8083 (Ctrl+C to stop)"
    kubectl port-forward svc/my-connect-cluster-connect-api 8083:8083 -n {{ namespace }}

# Forward Kafka bootstrap (9092)
port-forward-kafka:
    @echo "Kafka bootstrap → localhost:9092 (Ctrl+C to stop)"
    kubectl port-forward svc/my-cluster-kafka-bootstrap 9092:9092 -n {{ namespace }}

# Forward PostgreSQL (5432)
port-forward-postgres:
    @echo "PostgreSQL → localhost:5432 (user: postgres / password / db: source_db)"
    kubectl port-forward svc/postgres 5432:5432 -n {{ namespace }}

# Forward Garage S3 API (3900)
port-forward-garage:
    @echo "Garage S3 → http://localhost:3900 (Ctrl+C to stop)"
    kubectl port-forward svc/garage 3900:3900 -n {{ namespace }}

# Forward Nessie catalog API (19120)
port-forward-nessie:
    @echo "Nessie → http://localhost:19120 (Ctrl+C to stop)"
    kubectl port-forward svc/nessie 19120:19120 -n {{ namespace }}

# Start Streamlit UI
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
