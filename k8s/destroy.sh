#!/bin/bash
set -e

NAMESPACE="kafka"

echo "Destroying Strimzi Ops local development environment"
echo "======================================================="
echo ""

read -p "This will delete all resources in the '${NAMESPACE}' namespace. Continue? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Cancelled."
    exit 0
fi

echo "Deleting resources..."

# Delete in reverse order
if kubectl get kafkaconnect my-connect-cluster -n ${NAMESPACE} &> /dev/null; then
    echo "  - Kafka Connect"
    kubectl delete -f 03-kafka-connect.yaml
fi

if kubectl get kafka my-cluster -n ${NAMESPACE} &> /dev/null; then
    echo "  - Kafka cluster"
    kubectl delete -f 02-kafka.yaml
fi

if kubectl get deployment nessie -n ${NAMESPACE} &> /dev/null; then
    echo "  - Nessie"
    kubectl delete -f 05-iceberg-catalog.yaml
fi

if kubectl get job garage-setup -n ${NAMESPACE} &> /dev/null; then
    echo "  - Garage Setup (legacy)"
    kubectl delete job garage-setup -n ${NAMESPACE} --ignore-not-found=true || true
fi

if kubectl get statefulset garage -n ${NAMESPACE} &> /dev/null; then
    echo "  - Garage"
    kubectl delete -f 04-garage.yaml
fi

if kubectl get statefulset postgres -n ${NAMESPACE} &> /dev/null; then
    echo "  - PostgreSQL"
    kubectl delete -f 01-postgres.yaml
fi

# Delete PVCs
echo "  - Persistent Volume Claims"
kubectl delete pvc --all -n ${NAMESPACE} 2>/dev/null || true

# Delete namespace (includes operator when installed into kafka)
echo "  - Namespace ${NAMESPACE}"
kubectl delete namespace ${NAMESPACE} --wait=true || true

# Also remove a leftover operator namespace from older setups
if kubectl get namespace strimzi-system >/dev/null 2>&1; then
  echo "  - Namespace strimzi-system"
  kubectl delete namespace strimzi-system --wait=true || true
fi

# Optional full wipe of Strimzi CRDs (needed when jumping across major API versions)
if [[ "${DESTROY_CRDS:-}" == "1" || "${DESTROY_CRDS:-}" == "true" ]]; then
  echo "  - Strimzi CRDs"
  kubectl get crd -o name | grep -E '\.(kafka|core)\.strimzi\.io$' | xargs kubectl delete --wait=true 2>/dev/null || true
fi

echo ""
echo "All resources deleted."
if [[ "${DESTROY_CRDS:-}" != "1" && "${DESTROY_CRDS:-}" != "true" ]]; then
  echo "Tip: for a clean Strimzi major upgrade, re-run with DESTROY_CRDS=1"
fi
echo ""
