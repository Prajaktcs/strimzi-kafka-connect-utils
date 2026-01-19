#!/bin/bash
set -e

echo "🔧 Setting up Garage S3 Storage..."
echo "=================================="

# Wait for Garage to be ready
echo "Waiting for Garage to start..."
sleep 5

# Initialize layout (single node)
echo "Initializing Garage layout..."
docker exec garage /garage layout assign -z dc1 -c 1g 127.0.0.1 || true
docker exec garage /garage layout apply --version 1 || true

# Create key and bucket
echo "Creating access key and bucket..."
docker exec garage /garage key create lakehouse-key || true
docker exec garage /garage bucket create warehouse || true
docker exec garage /garage bucket allow warehouse --read --write --key lakehouse-key

echo ""
echo "✅ Garage setup complete!"
echo ""
echo "📋 To get your access credentials, run:"
echo "   docker exec garage /garage key info lakehouse-key"
echo ""
echo "Update your secrets.toml with the displayed access key and secret key."
