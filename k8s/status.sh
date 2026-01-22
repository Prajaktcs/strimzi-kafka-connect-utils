#!/bin/bash

NAMESPACE="kafka"

echo "📊 Strimzi Ops Development Environment Status"
echo "======================================================="
echo ""

# Check if namespace exists
if ! kubectl get namespace ${NAMESPACE} &> /dev/null; then
    echo "❌ Namespace '${NAMESPACE}' not found. Run ./deploy.sh to create it."
    exit 1
fi

echo "📦 Resources in namespace '${NAMESPACE}':"
echo ""

# Get all pods
echo "Pods:"
kubectl get pods -n ${NAMESPACE} -o wide

echo ""
echo "Services:"
kubectl get svc -n ${NAMESPACE}

echo ""
echo "Kafka Resources:"
kubectl get kafka,kafkaconnect,kafkaconnector -n ${NAMESPACE} 2>/dev/null || echo "  No Strimzi resources found"

echo ""
echo "Persistent Volume Claims:"
kubectl get pvc -n ${NAMESPACE}

echo ""
echo "======================================================="
echo ""
echo "📝 Quick Actions:"
echo ""
echo "Port Forward Kafka Connect:"
echo "  kubectl port-forward svc/my-connect-cluster-connect-api 8083:8083 -n ${NAMESPACE}"
echo ""
echo "Port Forward Kafka:"
echo "  kubectl port-forward svc/my-cluster-kafka-bootstrap 9092:9092 -n ${NAMESPACE}"
echo ""
echo "Port Forward PostgreSQL:"
echo "  kubectl port-forward svc/postgres 5432:5432 -n ${NAMESPACE}"
echo ""
echo "View logs:"
echo "  kubectl logs -f <pod-name> -n ${NAMESPACE}"
echo ""
