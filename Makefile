.PHONY: help setup deploy status destroy port-forward port-forward-kafka port-forward-postgres install sync run test format lint check lint-config

help:
	@echo "Strimzi Ops Platform - Available Commands"
	@echo "==========================================="
	@echo ""
	@echo "Setup & Deployment:"
	@echo "  setup              - Complete setup (install deps + deploy K8s)"
	@echo "  deploy             - Deploy local Kubernetes environment"
	@echo "  status             - Check deployment status"
	@echo "  destroy            - Destroy local environment"
	@echo ""
	@echo "Port Forwarding:"
	@echo "  port-forward       - Forward Kafka Connect API (8083)"
	@echo "  port-forward-kafka - Forward Kafka bootstrap (9092)"
	@echo "  port-forward-postgres - Forward PostgreSQL (5432)"
	@echo ""
	@echo "Python:"
	@echo "  install            - Install Python dependencies"
	@echo "  sync               - Sync dependencies (alias for install)"
	@echo ""
	@echo "Application:"
	@echo "  run                - Start Streamlit UI"
	@echo "  lint-config        - Lint connector config (FILE=path)"
	@echo ""
	@echo "Development:"
	@echo "  test               - Run tests"
	@echo "  format             - Format code with black"
	@echo "  lint               - Lint code with ruff"
	@echo "  check              - Run all code quality checks"
	@echo ""
	@echo "Examples:"
	@echo "  make setup"
	@echo "  make lint-config FILE=examples/debezium-postgres-connector.yaml"
	@echo ""

setup:
	@echo "Running complete setup..."
	@$(MAKE) install
	@$(MAKE) deploy
	@echo ""
	@echo "Setup complete!"
	@echo ""
	@echo "Next steps:"
	@echo "1. Open a new terminal and run: make port-forward"
	@echo "2. Configure secrets.toml (copy from secrets.toml.example)"
	@echo "3. Run the UI: make run"
	@echo ""

deploy:
	@cd k8s && ./deploy.sh

status:
	@cd k8s && ./status.sh

destroy:
	@cd k8s && ./destroy.sh

port-forward:
	@echo "Starting port-forward for Kafka Connect API..."
	@echo "  http://localhost:8083"
	@echo "  Press Ctrl+C to stop"
	@kubectl port-forward svc/my-connect-cluster-connect-api 8083:8083 -n kafka

port-forward-kafka:
	@echo "Starting port-forward for Kafka Bootstrap..."
	@echo "  localhost:9092"
	@echo "  Press Ctrl+C to stop"
	@kubectl port-forward svc/my-cluster-kafka-bootstrap 9092:9092 -n kafka

port-forward-postgres:
	@echo "Starting port-forward for PostgreSQL..."
	@echo "  localhost:5432"
	@echo "  User: postgres, Password: password, DB: source_db"
	@echo "  Press Ctrl+C to stop"
	@kubectl port-forward svc/postgres 5432:5432 -n kafka

install:
	@echo "Installing Python dependencies with uv..."
	@uv sync
	@echo "Dependencies installed."

sync:
	@echo "Syncing Python dependencies with uv..."
	@uv sync
	@echo "Dependencies synced."

run:
	@echo "Starting Strimzi Ops Platform..."
	@uv run streamlit run app.py

lint-config:
	@echo "Usage: make lint-config FILE=path/to/config.yaml"
	@if [ -z "$(FILE)" ]; then \
		echo "Error: FILE parameter required"; \
		echo "Example: make lint-config FILE=examples/debezium-postgres-connector.yaml"; \
		exit 1; \
	fi
	@uv run strimzi-lint lint $(FILE)

test:
	@echo "Running tests..."
	@uv run pytest tests/ -v

format:
	@echo "Formatting Python code..."
	@uv run black strimzi_ops/ app.py

lint:
	@echo "Linting Python code..."
	@uv run ruff check strimzi_ops/ app.py

check:
	@echo "Running code quality checks..."
	@uv run black --check strimzi_ops/ app.py
	@uv run ruff check strimzi_ops/ app.py
	@echo "All checks passed!"
