#!/usr/bin/env bash
# Helpers for one-command local setup (invoked from the justfile).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-kafka}"
LOCAL_DIR="${ROOT}/.local"
PID_DIR="${LOCAL_DIR}/port-forwards"

ensure_buildx() {
  if docker buildx version >/dev/null 2>&1; then
    echo "docker buildx $(docker buildx version | awk '{print $2}')"
    return 0
  fi

  echo "docker buildx plugin missing or broken."

  if ! command -v brew >/dev/null 2>&1; then
    echo "Install buildx: https://docs.docker.com/go/buildx/"
    echo "Or on macOS with Homebrew: brew install docker-buildx"
    exit 1
  fi

  echo "Installing docker-buildx via Homebrew..."
  brew install docker-buildx

  local plugin_src plugin_dir
  plugin_src="$(brew --prefix)/opt/docker-buildx/bin/docker-buildx"
  plugin_dir="${HOME}/.docker/cli-plugins"
  mkdir -p "${plugin_dir}"
  # Replace broken Docker Desktop symlinks with the Homebrew plugin.
  ln -sfn "${plugin_src}" "${plugin_dir}/docker-buildx"

  if ! docker buildx version >/dev/null 2>&1; then
    echo "buildx still unavailable after install. Try:"
    echo "  mkdir -p ~/.docker/cli-plugins"
    echo "  ln -sfn \"$(brew --prefix)/opt/docker-buildx/bin/docker-buildx\" ~/.docker/cli-plugins/docker-buildx"
    exit 1
  fi

  echo "docker buildx ready: $(docker buildx version)"
}

ensure_cluster() {
  if ! command -v kubectl >/dev/null 2>&1; then
    echo "kubectl not found. Install kubectl first."
    exit 1
  fi

  if kubectl cluster-info >/dev/null 2>&1; then
    echo "Connected to Kubernetes cluster"
    return 0
  fi

  if ! command -v colima >/dev/null 2>&1; then
    echo "Cannot connect to Kubernetes and colima is not installed."
    echo "Start a cluster first, e.g.: colima start --kubernetes --cpu 4 --memory 4"
    exit 1
  fi

  echo "No Kubernetes cluster reachable. Starting Colima with Kubernetes..."
  colima start --kubernetes --cpu 4 --memory 4

  echo "Waiting for Kubernetes API..."
  for _ in $(seq 1 60); do
    if kubectl cluster-info >/dev/null 2>&1; then
      echo "Connected to Kubernetes cluster"
      return 0
    fi
    sleep 2
  done

  echo "Colima started but Kubernetes is still unreachable."
  exit 1
}

sync_secrets() {
  # Args from justfile pins (access secret bucket endpoint); fall back to local defaults.
  local access_key="${1:-GKaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}"
  local secret_key="${2:-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb}"
  local bucket="${3:-warehouse}"
  local endpoint="${4:-http://localhost:3900}"
  write_secrets "${access_key}" "${secret_key}" "${bucket}" "${endpoint}"
  echo "Wrote local-dev Garage credentials to secrets.toml"
  echo "  access_key=${access_key}"
  echo "  endpoint=${endpoint}"
  echo "  bucket=${bucket}"
}

write_secrets() {
  local access_key="$1"
  local secret_key="$2"
  local bucket="${3:-warehouse}"
  local endpoint="${4:-http://localhost:3900}"
  cat >"${ROOT}/secrets.toml" <<EOF
[kafka]
bootstrap_servers = "localhost:9092"
connect_url = "http://localhost:8083"

[storage]
type = "s3"
endpoint_url = "${endpoint}"
access_key = "${access_key}"
secret_key = "${secret_key}"
bucket = "${bucket}"
EOF
}

apply_connector() {
  echo "Applying postgres source connector..."
  kubectl apply -f "${ROOT}/k8s/test-connector.yaml"
}

start_port_forward() {
  local name="$1"
  local svc="$2"
  local local_port="$3"
  local remote_port="$4"
  local pid_file log_file

  mkdir -p "${PID_DIR}"
  pid_file="${PID_DIR}/${name}.pid"
  log_file="${PID_DIR}/${name}.log"

  if [[ -f "${pid_file}" ]]; then
    local old_pid
    old_pid="$(cat "${pid_file}")"
    if kill -0 "${old_pid}" 2>/dev/null; then
      echo "Port-forward ${name} already running (pid ${old_pid})"
      return 0
    fi
    rm -f "${pid_file}"
  fi

  # Free a stale listener on the local port if present
  if command -v lsof >/dev/null 2>&1; then
    local stale
    stale="$(lsof -tiTCP:"${local_port}" -sTCP:LISTEN 2>/dev/null || true)"
    if [[ -n "${stale}" ]]; then
      echo "Stopping process(es) on port ${local_port}: ${stale}"
      # shellcheck disable=SC2086
      kill ${stale} 2>/dev/null || true
      sleep 1
    fi
  fi

  kubectl port-forward -n "${NAMESPACE}" "${svc}" "${local_port}:${remote_port}" \
    >"${log_file}" 2>&1 &
  echo $! >"${pid_file}"
  sleep 1

  if ! kill -0 "$(cat "${pid_file}")" 2>/dev/null; then
    echo "Failed to start port-forward ${name}. See ${log_file}"
    return 1
  fi
  echo "Port-forward ${name} → localhost:${local_port}"
}

port_forward_all() {
  start_port_forward connect svc/my-connect-cluster-connect-api 8083 8083
  start_port_forward kafka svc/my-cluster-kafka-bootstrap 9092 9092
  start_port_forward postgres svc/postgres 5432 5432
  start_port_forward garage svc/garage 3900 3900
  start_port_forward nessie svc/nessie 19120 19120
  echo "All port-forwards running in background (just stop-forwards to stop)."
}

stop_port_forwards() {
  if [[ ! -d "${PID_DIR}" ]]; then
    echo "No port-forwards to stop."
    return 0
  fi
  local pid_file pid
  for pid_file in "${PID_DIR}"/*.pid; do
    [[ -e "${pid_file}" ]] || continue
    pid="$(cat "${pid_file}")"
    if kill -0 "${pid}" 2>/dev/null; then
      echo "Stopping $(basename "${pid_file}" .pid) (pid ${pid})"
      kill "${pid}" 2>/dev/null || true
    fi
    rm -f "${pid_file}"
  done
}

usage() {
  cat <<EOF
Usage: $0 <command>

Commands:
  ensure-cluster
  ensure-buildx
  sync-secrets
  apply-connector
  port-forward-all
  stop-forwards
EOF
}

main() {
  local cmd="${1:-}"
  shift || true
  case "${cmd}" in
    ensure-cluster) ensure_cluster ;;
    ensure-buildx) ensure_buildx ;;
    sync-secrets) sync_secrets "$@" ;;
    apply-connector) apply_connector ;;
    port-forward-all) port_forward_all ;;
    stop-forwards) stop_port_forwards ;;
    *) usage; exit 1 ;;
  esac
}

main "$@"
