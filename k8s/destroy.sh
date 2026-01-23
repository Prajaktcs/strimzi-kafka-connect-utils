#!/bin/bash
set -e

NAMESPACE="kafka"

echo "🗑️  Destroying Strimzi Ops local development environment"
echo "======================================================="
echo ""

read -p "This will delete all resources in the '${NAMESPACE}' namespace. Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Cancelled."
    exit 0
fi

echo "🔥 Deleting resources..."

# Delete in reverse order
if kubectl get kafkaconnect my-connect-cluster -n ${NAMESPACE} &> /dev/null; then
    echo "  - Kafka Connect"
    kubectl delete -f 03-kafka-connect.yaml
fi

if kubectl get kafka my-cluster -n ${NAMESPACE} &> /dev/null; then
    echo "  - Kafka cluster"
    kubectl delete -f 02-kafka.yaml
fi

if kubectl get statefulset postgres -n ${NAMESPACE} &> /dev/null; then
    echo "  - PostgreSQL"
    kubectl delete -f 01-postgres.yaml
fi

# Delete PVCs
echo "  - Persistent Volume Claims"
kubectl delete pvc --all -n ${NAMESPACE} 2>/dev/null || true

# Delete namespace
echo "  - Namespace"
kubectl delete -f 00-namespace.yaml

echo ""
echo "✅ All resources deleted"
echo ""
echo "Note: Strimzi operator is still installed in strimzi-system namespace."
echo "To remove it:"
echo "  kubectl delete namespace strimzi-system"
echo ""
