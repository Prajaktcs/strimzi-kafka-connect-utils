#!/bin/bash
set -e

NAMESPACE="kafka"
STRIMZI_VERSION="1.1.0"

echo "Deploying Strimzi Ops local development environment"
echo "======================================================="
echo ""

# Check if kubectl is available
if ! command -v kubectl &> /dev/null; then
    echo "kubectl not found. Please install kubectl first."
    exit 1
fi

# Check if cluster is reachable
if ! kubectl cluster-info &> /dev/null; then
    echo "Cannot connect to Kubernetes cluster."
    echo "Make sure your cluster is running (e.g., colima start)"
    exit 1
fi

echo "Connected to Kubernetes cluster"
echo ""

# Create app namespace first (operator will be installed here for local single-ns watch)
kubectl apply -f 00-namespace.yaml

# Strimzi 1.x dropped the v1beta2 API. Patching CRDs that still list v1beta2 in
# status.storedVersions fails. For local dev, wipe legacy CRDs/operators first.
if kubectl get crd kafkas.kafka.strimzi.io >/dev/null 2>&1; then
  STORED_VERSIONS="$(kubectl get crd kafkas.kafka.strimzi.io -o jsonpath='{.status.storedVersions[*]}' 2>/dev/null || true)"
  if [[ "${STORED_VERSIONS}" == *v1beta2* ]]; then
    echo "Detected legacy Strimzi CRDs (storedVersions includes v1beta2)."
    echo "Resetting Strimzi CRDs for a clean ${STRIMZI_VERSION} install..."
    # Drop CRs first so CRD deletion is not blocked.
    kubectl delete kafka,kafkanodepool,kafkaconnect,kafkaconnector --all -A --wait=false 2>/dev/null || true
    kubectl get crd -o name | grep -E '\.(kafka|core)\.strimzi\.io$' | xargs kubectl delete --wait=true
    kubectl delete namespace strimzi-system --wait=false 2>/dev/null || true
    echo "Legacy Strimzi CRDs removed."
  fi
fi

# Install or upgrade Strimzi operator to the pinned version (watches the kafka namespace)
echo "Installing/upgrading Strimzi operator ${STRIMZI_VERSION} into ${NAMESPACE}..."
OPERATOR_YAML="$(mktemp)"
curl -fsSL \
  "https://github.com/strimzi/strimzi-kafka-operator/releases/download/${STRIMZI_VERSION}/strimzi-cluster-operator-${STRIMZI_VERSION}.yaml" \
  -o "${OPERATOR_YAML}"
# Official bundle defaults to namespace "myproject"; point it at our local namespace.
sed "s/namespace: myproject/namespace: ${NAMESPACE}/g" "${OPERATOR_YAML}" | kubectl apply -f -
rm -f "${OPERATOR_YAML}"

echo "Waiting for Strimzi operator to be ready..."
kubectl wait --for=condition=ready pod -l name=strimzi-cluster-operator -n ${NAMESPACE} --timeout=300s
echo "Strimzi operator ${STRIMZI_VERSION} is ready"
echo ""

# Apply manifests
echo "Applying Kubernetes manifests..."

# Namespace already applied above

# Deploy PostgreSQL
echo "  - PostgreSQL database"
kubectl apply -f 01-postgres.yaml

# Wait for PostgreSQL to be ready
echo "Waiting for PostgreSQL to be ready..."
kubectl wait --for=condition=ready pod -l app=postgres -n ${NAMESPACE} --timeout=300s
echo "PostgreSQL is ready"

# Deploy Garage S3
echo "  - Garage S3"
kubectl apply -f 04-garage.yaml

# Wait for Garage to be ready
echo "Waiting for Garage to be ready..."
kubectl wait --for=condition=ready pod -l app=garage -n ${NAMESPACE} --timeout=300s
echo "Garage is ready"

# Run Garage Setup
echo "  - Garage Setup Job"
kubectl delete job garage-setup -n ${NAMESPACE} 2>/dev/null || true
kubectl apply -f 04-garage-setup.yaml
echo "Waiting for Garage Setup to complete..."
kubectl wait --for=condition=complete job/garage-setup -n ${NAMESPACE} --timeout=120s
echo "Garage Setup complete"

# Retrieve S3 Keys from Job logs
echo "Retrieving S3 Credentials..."
POD_NAME=$(kubectl get pods -n ${NAMESPACE} -l job-name=garage-setup -o jsonpath='{.items[0].metadata.name}')
echo "---------------------------------------------------"
kubectl logs $POD_NAME -n ${NAMESPACE} | grep "Key ID" || true
kubectl logs $POD_NAME -n ${NAMESPACE} | grep "Secret Key" || true
echo "---------------------------------------------------"

# Deploy Nessie Catalog
echo "  - Nessie Iceberg Catalog"
kubectl apply -f 05-iceberg-catalog.yaml

# Wait for Nessie to be ready
echo "Waiting for Nessie to be ready..."
kubectl wait --for=condition=ready pod -l app=nessie -n ${NAMESPACE} --timeout=300s
echo "Nessie is ready"

# Deploy Kafka
echo "  - Kafka cluster"
kubectl apply -f 02-kafka.yaml

# Wait for Kafka to be ready
echo "Waiting for Kafka cluster to be ready (this may take a few minutes)..."
kubectl wait kafka/my-cluster --for=condition=Ready --timeout=600s -n ${NAMESPACE}
echo "Kafka cluster is ready"

# Deploy Kafka Connect
echo "  - Kafka Connect"
kubectl apply -f 03-kafka-connect.yaml

# Wait for Kafka Connect to be ready
echo "Waiting for Kafka Connect to be ready (this may take a few minutes for first build)..."
kubectl wait kafkaconnect/my-connect-cluster --for=condition=Ready --timeout=900s -n ${NAMESPACE}
echo "Kafka Connect is ready"

echo ""
echo "Deployment complete!"
echo ""
echo "Connection Information:"
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
echo "4. Garage S3 API:"
echo "   kubectl port-forward svc/garage 3900:3900 -n ${NAMESPACE}"
echo ""
echo "5. Nessie Catalog API:"
echo "   kubectl port-forward svc/nessie 19120:19120 -n ${NAMESPACE}"
echo ""
echo "Then configure secrets.toml with the credentials retrieved above."
echo ""
echo "Monitor deployment status:"
echo "kubectl get pods -n ${NAMESPACE}"
echo ""
echo "To remove everything:"
echo "./destroy.sh"
echo ""
