# Kubernetes Deployment for Local Development

This directory contains Kubernetes manifests and scripts to deploy a complete Kafka Connect environment locally using Strimzi.

## What Gets Deployed

1. **Strimzi Operator** - v0.50.0 - Kubernetes operator for managing Kafka
2. **Kafka Cluster** - v4.1.1 - Single-node cluster using **KRaft mode** (ZooKeeper-less)
3. **Kafka Connect** - v4.1.1 - With Debezium 3.4.0 (PostgreSQL connector)
4. **PostgreSQL** - v18.2-alpine - Database configured for CDC (Change Data Capture)
5. **Garage S3** - v2.1.0 - S3-compatible object storage for Iceberg

## Prerequisites

- Kubernetes cluster (Colima, Minikube, kind, or Docker Desktop)
- kubectl installed and configured
- [just](https://github.com/casey/just) (`brew install just`)
- Minimum 4GB RAM allocated to your cluster

## Quick Start

```bash
# From the project root:

# Full local setup (deps + Connect image + deploy + secrets.toml)
just setup

# Or deploy only
just deploy

# Check status
just status

# Port forward services (each in a separate terminal)
just port-forward          # Terminal 1: Kafka Connect API
just port-forward-kafka    # Terminal 2: Kafka Bootstrap
just port-forward-postgres # Terminal 3: PostgreSQL
just port-forward-garage   # Terminal 4: Garage S3
just port-forward-nessie   # Terminal 5: Nessie catalog

# Clean up when done
just destroy
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
just deploy

# Or from k8s/ directory
./deploy.sh
```

The script will:
- Check if Strimzi operator is installed (installs if needed)
- Create namespace `kafka`
- Deploy PostgreSQL with CDC configuration
- Deploy Garage S3 and run setup job
- Deploy Kafka cluster in **KRaft mode** (takes ~3 minutes)
- Deploy Kafka Connect with custom image (built locally)
- Wait for everything to be ready

### 3. Verify Deployment

```bash
# From project root
just status

# Or from k8s/ directory
./status.sh
```

Expected output:
```
Pods:
  my-cluster-dual-role-0              1/1     Running
  my-connect-cluster-connect-0        1/1     Running
  postgres-0                          1/1     Running
  garage-0                            1/1     Running
```

### 4. Access Services

Port forward to access from localhost:

```bash
just port-forward
just port-forward-kafka
just port-forward-postgres
just port-forward-garage
just port-forward-nessie
```

Then configure `secrets.toml`:
```toml
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"

[storage]
type = "s3"
endpoint_url = "http://localhost:3900"
access_key = "..."
secret_key = "..."
bucket = "warehouse"
```

## Manifest Details

### 00-namespace.yaml
Creates dedicated `kafka` namespace for all resources.

### 01-postgres.yaml
Deploys PostgreSQL 18.2 Alpine with logical replication enabled.

### 02-kafka.yaml
Deploys single-node Kafka 4.1.1 cluster via Strimzi using **KRaft** and **KafkaNodePool**.

### 03-kafka-connect.yaml
Deploys Kafka Connect with Debezium PostgreSQL connector. Uses a custom local image `my-connect-cluster:0.0.1`.

### 04-garage.yaml
Deploys Garage S3 v2.1.0 for object storage.

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
