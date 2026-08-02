# Strimzi Ops Platform — https://github.com/casey/just
#
# One command:
#   just setup
#
# That will:
#   1. start Colima+k8s if needed
#   2. ensure docker buildx (Homebrew plugin; avoids legacy builder warning)
#   3. build Connect image with Debezium + Iceberg sink
#   4. deploy Strimzi 1.1.0 + Kafka 4.3.0 + Postgres + Garage 2.3 + Nessie
#      (wipes legacy v1beta2 Strimzi CRDs if present; installs operator into kafka ns)
#   5. write secrets.toml with local-dev Garage keys
#   6. apply sample Postgres source + Iceberg sink connectors
#   7. start background port-forwards (nohup under .local/port-forwards/; IPv4)
#   8. launch Rust web UI (strimzi-ui)
#
# Day-to-day:
#   just port-forward-all   # (re)start + health-check Connect :8083
#   just status-forwards    # show tracked forward PIDs
#   just ui                 # ensures forwards, then starts strimzi-ui
#   just doctor             # pods + forwards + HTTP checks
#   just rust-check         # fmt + clippy -D warnings
#   just lint-config <file> # Rust strimzi-lint
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
garage_endpoint := "http://127.0.0.1:3900"

default:
    @just --list

# One-command local setup → infra + secrets + connector + port-forwards + UI
setup: ensure-cluster build-connect deploy sync-secrets apply-connector port-forward-all
    @echo ""
    @echo "Local stack is ready."
    @echo "  Connect API : http://127.0.0.1:8083"
    @echo "  Kafka       : 127.0.0.1:9092"
    @echo "  Postgres    : 127.0.0.1:5432  (postgres / password / source_db)"
    @echo "  Garage S3   : {{ garage_endpoint }}"
    @echo "  Nessie      : http://127.0.0.1:19120"
    @echo "  UI          : http://127.0.0.1:8501"
    @echo ""
    @echo "Stack: Strimzi {{ strimzi_version }} / Kafka {{ kafka_version }} / Debezium {{ debezium_version }}"
    @echo "Starting strimzi-ui..."
    just ui

# Alias for setup
up: setup

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

# Quick health: pods + Connect/UI HTTP checks + port-forward PIDs
doctor:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "Pods in {{ namespace }}:"
    kubectl get pods -n {{ namespace }} || true
    echo ""
    bash {{ helper }} status-forwards || true
    echo ""
    echo -n "Connect :8083 → "; curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8083/ || echo "down"
    echo -n "UI      :8501 → "; curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:8501/ || echo "down"
    echo -n "Garage  :3900 → "; curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:3900/ || echo "down"
    echo -n "Nessie  :19120 → "; curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:19120/ || echo "down"
    echo ""
    echo "If Connect is down: just port-forward-all"

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

# Start (or repair) all local port-forwards in the background; verifies Connect :8083
port-forward-all:
    bash {{ helper }} port-forward-all

# Show tracked background port-forward status
status-forwards:
    bash {{ helper }} status-forwards

# Stop background port-forwards started by setup / port-forward-all
stop-forwards:
    bash {{ helper }} stop-forwards

# Forward Kafka Connect REST API (8083) — foreground
port-forward:
    @echo "Kafka Connect API → http://127.0.0.1:8083 (Ctrl+C to stop)"
    kubectl port-forward --address 127.0.0.1 svc/my-connect-cluster-connect-api 8083:8083 -n {{ namespace }}

# Forward Kafka bootstrap (9092) — foreground
port-forward-kafka:
    @echo "Kafka bootstrap → 127.0.0.1:9092 (Ctrl+C to stop)"
    kubectl port-forward --address 127.0.0.1 svc/my-cluster-kafka-bootstrap 9092:9092 -n {{ namespace }}

# Forward PostgreSQL (5432) — foreground
port-forward-postgres:
    @echo "PostgreSQL → 127.0.0.1:5432 (user: postgres / password / db: source_db)"
    kubectl port-forward --address 127.0.0.1 svc/postgres 5432:5432 -n {{ namespace }}

# Forward Garage S3 API (3900) — foreground
port-forward-garage:
    @echo "Garage S3 → {{ garage_endpoint }} (Ctrl+C to stop)"
    kubectl port-forward --address 127.0.0.1 svc/garage 3900:3900 -n {{ namespace }}

# Forward Nessie catalog API (19120) — foreground
port-forward-nessie:
    @echo "Nessie → http://127.0.0.1:19120 (Ctrl+C to stop)"
    kubectl port-forward --address 127.0.0.1 svc/nessie 19120:19120 -n {{ namespace }}

# Start Rust web UI (ensures port-forwards first)
run: ui

# Ensure Connect is reachable, then start strimzi-ui on http://127.0.0.1:<port>
ui port="8501": port-forward-all
    @echo "Starting strimzi-ui on http://127.0.0.1:{{ port }} ..."
    cargo run -q -p strimzi-ui -- --port {{ port }} --connect-url http://127.0.0.1:8083 --bootstrap-servers 127.0.0.1:9092

# Lint a connector config file (Rust CLI)
lint-config file:
    cargo run -q -p strimzi-ops --bin strimzi-lint -- lint {{ file }}

# Alias for lint-config
lint-config-rust file:
    just lint-config {{ file }}

# Example: list connectors via Rust CLI (needs Connect URL)
connectors-list connect_url="http://127.0.0.1:8083":
    cargo run -q -p strimzi-ops -- --connect-url {{ connect_url }} connectors list

# Run workspace tests (alias for rust-test)
test: rust-test

# Run Rust workspace tests
rust-test:
    cargo test --workspace --all-features

# Format Rust with rustfmt (alias)
format: rust-fmt

# Format Rust with rustfmt
rust-fmt:
    cargo fmt --all

# Clippy + rustfmt check (Canonical preconditions); also used as `just lint` / `just check`
lint: rust-check
check: rust-check

rust-check:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
