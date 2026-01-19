.PHONY: help setup start stop restart logs clean install sync run test format lint check

help:
	@echo "Strimzi Ops Platform - Available Commands"
	@echo "==========================================="
	@echo "setup      - Run initial setup (start infrastructure + install deps)"
	@echo "start      - Start all infrastructure services"
	@echo "stop       - Stop all services"
	@echo "restart    - Restart all services"
	@echo "logs       - Show logs from all services"
	@echo "clean      - Stop and remove all containers and volumes"
	@echo "install    - Install Python dependencies with uv"
	@echo "sync       - Sync dependencies with uv (alias for install)"
	@echo "run        - Start the Streamlit application with uv"
	@echo "test       - Run tests with pytest"
	@echo "format     - Format Python code with black"
	@echo "lint       - Lint Python code with ruff"
	@echo "check      - Run format + lint checks"

setup:
	@./setup.sh

start:
	@echo "Starting infrastructure services..."
	@docker-compose up -d
	@echo "Services started. Run 'make logs' to view logs."

stop:
	@echo "Stopping services..."
	@docker-compose down

restart:
	@echo "Restarting services..."
	@docker-compose restart

logs:
	@docker-compose logs -f

clean:
	@echo "Cleaning up..."
	@docker-compose down -v
	@echo "All containers and volumes removed."

install:
	@echo "Installing Python dependencies with uv..."
	@uv sync
	@echo "Dependencies installed."

sync:
	@echo "Syncing Python dependencies with uv..."
	@uv sync
	@echo "Dependencies synced."

run:
	@echo "Starting Lakehouse Ops Platform..."
	@uv run streamlit run app.py

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
