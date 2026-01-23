#!/bin/bash
set -e

NAMESPACE="kafka"
STRIMZI_VERSION="0.50.0"

echo "🚀 Deploying Strimzi Ops local development environment"
echo "======================================================="
echo ""

# Check if kubectl is available
if ! command -v kubectl &> /dev/null; then
    echo "❌ kubectl not found. Please install kubectl first."
    exit 1
fi

# Check if cluster is reachable
if ! kubectl cluster-info &> /dev/null; then
    echo "❌ Cannot connect to Kubernetes cluster."
    echo "   Make sure your cluster is running (e.g., colima start)"
    exit 1
fi

echo "✅ Connected to Kubernetes cluster"
echo ""

# Check if Strimzi operator is installed
if ! kubectl get crd kafkas.kafka.strimzi.io &> /dev/null; then
    echo "📦 Strimzi operator not found. Installing Strimzi ${STRIMZI_VERSION}..."
    kubectl create namespace strimzi-system || true
    kubectl create -f "https://github.com/strimzi/strimzi-kafka-operator/releases/download/${STRIMZI_VERSION}/strimzi-cluster-operator-${STRIMZI_VERSION}.yaml" -n strimzi-system

    echo "⏳ Waiting for Strimzi operator to be ready..."
    kubectl wait --for=condition=ready pod -l name=strimzi-cluster-operator -n strimzi-system --timeout=300s
    echo "✅ Strimzi operator installed"
else
    echo "✅ Strimzi operator already installed"
fi
echo ""

# Apply manifests
echo "📝 Applying Kubernetes manifests..."

# Create namespace
kubectl apply -f 00-namespace.yaml

# Deploy PostgreSQL
echo "  - PostgreSQL database"
kubectl apply -f 01-postgres.yaml

# Wait for PostgreSQL to be ready
echo "⏳ Waiting for PostgreSQL to be ready..."
kubectl wait --for=condition=ready pod -l app=postgres -n ${NAMESPACE} --timeout=300s
echo "✅ PostgreSQL is ready"

# Deploy Kafka
echo "  - Kafka cluster"
kubectl apply -f 02-kafka.yaml

# Wait for Kafka to be ready
echo "⏳ Waiting for Kafka cluster to be ready (this may take a few minutes)..."
kubectl wait kafka/my-cluster --for=condition=Ready --timeout=600s -n ${NAMESPACE}
echo "✅ Kafka cluster is ready"

# Deploy Kafka Connect
echo "  - Kafka Connect"
kubectl apply -f 03-kafka-connect.yaml

# Wait for Kafka Connect to be ready
echo "⏳ Waiting for Kafka Connect to be ready (this may take a few minutes for first build)..."
kubectl wait kafkaconnect/my-connect-cluster --for=condition=Ready --timeout=900s -n ${NAMESPACE}
echo "✅ Kafka Connect is ready"

echo ""
echo "🎉 Deployment complete!"
echo ""
echo "📋 Connection Information:"
echo "======================================================="
echo ""
echo "To access services from your local machine, run these commands in separate terminals:"
echo ""
echo "1. Kafka Connect REST API:"
echo "   kubectl port-forward svc/my-connect-cluster-connect-api 8083:8083 -n ${NAMESPACE}"
echo ""
echo "2. Kafka Bootstrap Servers:"
echo "   kubectl port-forward svc/my-cluster-kafka-bootstrap 9092:9092 -n ${NAMESPACE}"
echo ""
echo "3. PostgreSQL Database:"
echo "   kubectl port-forward svc/postgres 5432:5432 -n ${NAMESPACE}"
echo ""
echo "Then configure secrets.toml with:"
echo ""
echo "[kafka]"
echo "bootstrap_servers = \"localhost:9092\""
echo "connect_url = \"http://localhost:8083\""
echo ""
echo "📊 Monitor deployment status:"
echo "kubectl get pods -n ${NAMESPACE}"
echo ""
echo "🗑️  To remove everything:"
echo "./destroy.sh"
echo ""
