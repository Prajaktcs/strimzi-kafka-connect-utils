# Kubernetes Deployment for Local Development

This directory contains Kubernetes manifests and scripts to deploy a complete Kafka Connect environment locally using Strimzi.

## What Gets Deployed

1. **Strimzi Operator** - v0.50.0 - Kubernetes operator for managing Kafka
2. **Kafka Cluster** - v3.10.0 - Single-node cluster (suitable for local development)
3. **Kafka Connect** - v3.10.0 - With Debezium 3.1.2 (PostgreSQL & MySQL connectors)
4. **PostgreSQL** - v16-alpine - Database configured for CDC (Change Data Capture)

## Prerequisites

- Kubernetes cluster (Colima, Minikube, kind, or Docker Desktop)
- kubectl installed and configured
- Minimum 4GB RAM allocated to your cluster

## Quick Start

```bash
# From the project root:

# Deploy everything
make deploy

# Check status
make status

# Port forward services (each in a separate terminal)
make port-forward        # Terminal 1: Kafka Connect API
make port-forward-kafka  # Terminal 2: Kafka Bootstrap

# Clean up when done
make destroy
```

You can also run the scripts directly from this directory:

```bash
./deploy.sh
./status.sh
./destroy.sh
```

## Detailed Guide

### 1. Start Your Kubernetes Cluster

#### Colima (Recommended for macOS)

```bash
# Start with Kubernetes enabled
colima start --kubernetes --cpu 4 --memory 4

# Verify
kubectl cluster-info
```

#### Minikube

```bash
minikube start --cpus 4 --memory 4096

# Verify
kubectl cluster-info
```

#### kind (Kubernetes in Docker)

```bash
kind create cluster --name strimzi-dev

# Verify
kubectl cluster-info
```

### 2. Deploy

```bash
# From project root
make deploy

# Or from k8s/ directory
./deploy.sh
```

The script will:
- Check if Strimzi operator is installed (installs if needed)
- Create namespace `kafka`
- Deploy PostgreSQL with CDC configuration
- Deploy Kafka cluster (takes ~3 minutes)
- Deploy Kafka Connect with custom connectors (takes ~5-10 minutes on first run)

### 3. Verify Deployment

```bash
# From project root
make status

# Or from k8s/ directory
./status.sh
```

Expected output:
```
Pods:
  my-cluster-kafka-0                  1/1     Running
  my-cluster-zookeeper-0              1/1     Running
  my-connect-cluster-connect-xxx      1/1     Running
  postgres-0                          1/1     Running
```

### 4. Access Services

Port forward to access from localhost (each in a separate terminal):

```bash
# Kafka Connect REST API (Terminal 1)
make port-forward

# Kafka Bootstrap (Terminal 2)
make port-forward-kafka

# PostgreSQL (Terminal 3 - optional)
make port-forward-postgres
```

Or using kubectl directly:

```bash
kubectl port-forward svc/my-connect-cluster-connect-api 8083:8083 -n kafka
kubectl port-forward svc/my-cluster-kafka-bootstrap 9092:9092 -n kafka
kubectl port-forward svc/postgres 5432:5432 -n kafka
```

Then configure `secrets.toml`:
```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"
```

## Manifest Details

### 00-namespace.yaml
Creates dedicated `kafka` namespace for all resources.

### 01-postgres.yaml
Deploys PostgreSQL 16 Alpine with:
- CDC configuration (`wal_level=logical`)
- Persistent storage (1Gi)
- Database: `source_db`
- User: `postgres`
- Password: `password`

### 02-kafka.yaml
Deploys single-node Kafka cluster via Strimzi:
- Kafka version: 3.10.0
- 1 broker replica
- Internal listeners (plain and TLS)
- ZooKeeper included
- Persistent storage (5Gi for Kafka, 2Gi for ZooKeeper)
- Resource limits suitable for local development

### 03-kafka-connect.yaml
Deploys Kafka Connect with:
- Version: 3.10.0
- Pre-installed connectors:
  - Debezium PostgreSQL (v3.1.2.Final)
  - Debezium MySQL (v3.1.2.Final)
- Custom image build (automatic)
- Connector resources enabled

## Troubleshooting

### Kafka Connect build takes too long

First build can take 5-10 minutes as it downloads connector plugins and builds a custom image. Subsequent deployments use cached image.

Check build progress:
```bash
kubectl logs -f my-connect-cluster-connect-build -n kafka
```

### Pods not starting

Check resource availability:
```bash
kubectl top nodes
kubectl describe pod <pod-name> -n kafka
```

### Port forward connection refused

Ensure pod is running:
```bash
kubectl get pods -n kafka
```

If pod is ready but connection fails, the service might not be fully initialized. Wait 30 seconds and try again.

### Strimzi operator issues

Check operator logs:
```bash
kubectl logs -f deployment/strimzi-cluster-operator -n strimzi-system
```

## Customization

### Add More Connectors

Edit `03-kafka-connect.yaml` and add to the `plugins` section:

```yaml
- name: my-connector
  artifacts:
    - type: tgz
      url: https://example.com/connector.tar.gz
```

Then apply:
```bash
kubectl apply -f 03-kafka-connect.yaml
```

### Increase Resources

For larger deployments, edit the resource requests/limits in the YAML files.

### Use Different Kafka Version

Update `spec.kafka.version` in `02-kafka.yaml`.

## Cleanup

### Remove All Resources

```bash
./destroy.sh
```

This removes:
- All pods, services, and resources in `kafka` namespace
- Persistent Volume Claims
- The `kafka` namespace itself

**Note**: Strimzi operator remains installed for faster future deployments.

### Remove Strimzi Operator

```bash
kubectl delete namespace strimzi-system
```

## Production Considerations

**This setup is for local development only.** For production:

1. Use multiple replicas for high availability
2. Configure proper resource requests/limits
3. Enable TLS/authentication
4. Use proper storage classes with backups
5. Configure monitoring (Prometheus/Grafana)
6. Use dedicated namespaces per environment
7. Implement proper secrets management
8. Configure network policies

See [Strimzi Documentation](https://strimzi.io/docs/operators/latest/overview.html) for production setup.
