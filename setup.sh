#!/bin/bash
set -e

echo "🏗️  Lakehouse Ops Platform - Setup Script"
echo "=========================================="

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo "❌ Error: Docker is not running. Please start Docker and try again."
    exit 1
fi

echo "✅ Docker is running"

# Check if docker-compose is available
if ! command -v docker-compose &> /dev/null; then
    echo "❌ Error: docker-compose is not installed. Please install it and try again."
    exit 1
fi

echo "✅ docker-compose is available"

# Check Python version
python_version=$(python3 --version 2>&1 | awk '{print $2}')
required_version="3.10"

if [ "$(printf '%s\n' "$required_version" "$python_version" | sort -V | head -n1)" != "$required_version" ]; then
    echo "❌ Error: Python 3.10+ is required. Found: $python_version"
    exit 1
fi

echo "✅ Python version: $python_version"

# Start infrastructure
echo ""
echo "🚀 Starting infrastructure services..."
docker-compose up -d

echo ""
echo "⏳ Waiting for services to be ready..."
sleep 10

# Check service health
echo ""
echo "📊 Checking service health..."

# Check Redpanda
if docker-compose ps | grep -q "redpanda.*Up"; then
    echo "✅ Redpanda is running on port 9092"
else
    echo "⚠️  Warning: Redpanda may not be fully ready yet"
fi

# Check Garage
if docker-compose ps | grep -q "garage.*Up"; then
    echo "✅ Garage is running on port 3900"
else
    echo "⚠️  Warning: Garage may not be fully ready yet"
fi

# Check PostgreSQL
if docker-compose ps | grep -q "postgres-source.*Up"; then
    echo "✅ PostgreSQL is running on port 5432"
else
    echo "⚠️  Warning: PostgreSQL may not be fully ready yet"
fi

# Check Kafka Connect
if docker-compose ps | grep -q "connect.*Up"; then
    echo "✅ Kafka Connect is running on port 8083"
else
    echo "⚠️  Warning: Kafka Connect may not be fully ready yet"
fi

# Get Garage keys
echo ""
echo "🔑 Retrieving Garage access keys..."
echo "   Check the setup-garage logs for access credentials:"
echo ""
docker-compose logs setup-garage | grep -A 10 "Key" || echo "   Run: docker-compose logs setup-garage"

# Check if uv is installed
echo ""
if ! command -v uv &> /dev/null; then
    echo "⚠️  UV is not installed. Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="$HOME/.local/bin:$PATH"
    echo "✅ UV installed successfully"
else
    echo "✅ UV is already installed"
fi

# Install Python dependencies
echo ""
read -p "📦 Install Python dependencies with uv? (y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]; then
    echo "Installing Python dependencies with uv..."
    uv sync
    echo "✅ Python dependencies installed"
fi

echo ""
echo "✨ Setup complete!"
echo ""
echo "Next steps:"
echo "1. Update secrets.toml with your Garage access keys"
echo "2. Run 'uv run streamlit run app.py' to start the application"
echo "   Or: 'make run' for convenience"
echo "3. Open http://localhost:8501 in your browser"
echo ""
echo "For more information, see README.md"
