#!/usr/bin/env bash
# Helpers for one-command local setup (invoked from the justfile).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAMESPACE="${NAMESPACE:-kafka}"
LOCAL_DIR="${ROOT}/.local"
PID_DIR="${LOCAL_DIR}/port-forwards"
METRICS_DIR="${PID_DIR}/metrics"
METRICS_TARGETS="${LOCAL_DIR}/connect-metrics.toml"
CONNECT_CLUSTER="${CONNECT_CLUSTER:-my-connect-cluster}"

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
    echo "Start a cluster first, e.g.: colima start --kubernetes --cpu 4 --memory 8"
    exit 1
  fi

  echo "No Kubernetes cluster reachable. Starting Colima with Kubernetes..."
  colima start --kubernetes --cpu 4 --memory 8

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
  local access_key="${1:-rustfsadmin}"
  local secret_key="${2:-rustfsadmin}"
  local bucket="${3:-warehouse}"
  local endpoint="${4:-http://127.0.0.1:9000}"
  write_secrets "${access_key}" "${secret_key}" "${bucket}" "${endpoint}"
  echo "Wrote LOCAL DEV ONLY RustFS credentials to secrets.toml"
  echo "  access_key=${access_key}"
  echo "  endpoint=${endpoint}"
  echo "  bucket=${bucket}"
}

write_secrets() {
  local access_key="$1"
  local secret_key="$2"
  local bucket="${3:-warehouse}"
  local endpoint="${4:-http://127.0.0.1:9000}"
  # Prefer 127.0.0.1 over localhost so clients (reqwest, curl) do not hit ::1 first.
  cat >"${ROOT}/secrets.toml" <<EOF
[kafka]
bootstrap_servers = "127.0.0.1:9092"
connect_url = "http://127.0.0.1:8083"
metrics_targets_file = ".local/connect-metrics.toml"

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
  echo "Applying iceberg sink connector..."
  kubectl apply -f "${ROOT}/k8s/test-iceberg-sink.yaml"
}

port_listening() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"${port}" -sTCP:LISTEN >/dev/null 2>&1
    return $?
  fi
  # Fallback: try connecting to the port.
  (echo >/dev/tcp/127.0.0.1/"${port}") >/dev/null 2>&1
}

kill_port_listeners() {
  local port="$1"
  if ! command -v lsof >/dev/null 2>&1; then
    return 0
  fi
  local stale
  stale="$(lsof -tiTCP:"${port}" -sTCP:LISTEN 2>/dev/null || true)"
  if [[ -n "${stale}" ]]; then
    echo "Stopping process(es) on port ${port}: ${stale}"
    # shellcheck disable=SC2086
    kill ${stale} 2>/dev/null || true
    sleep 1
  fi
}

start_port_forward() {
  local name="$1"
  local svc="$2"
  local local_port="$3"
  local remote_port="$4"
  local pid_file log_file old_pid

  mkdir -p "${PID_DIR}"
  pid_file="${PID_DIR}/${name}.pid"
  log_file="${PID_DIR}/${name}.log"

  if [[ -f "${pid_file}" ]]; then
    old_pid="$(cat "${pid_file}")"
    if kill -0 "${old_pid}" 2>/dev/null && port_listening "${local_port}"; then
      echo "Port-forward ${name} already running (pid ${old_pid} → 127.0.0.1:${local_port})"
      return 0
    fi
    if kill -0 "${old_pid}" 2>/dev/null; then
      echo "Port-forward ${name} pid ${old_pid} is stale; restarting"
      kill "${old_pid}" 2>/dev/null || true
      sleep 1
    fi
    rm -f "${pid_file}"
  fi

  kill_port_listeners "${local_port}"

  # Bind IPv4 only and detach so forwards survive just/script exit.
  : >"${log_file}"
  nohup kubectl port-forward \
    --address 127.0.0.1 \
    -n "${NAMESPACE}" \
    "${svc}" \
    "${local_port}:${remote_port}" \
    >"${log_file}" 2>&1 &
  local pf_pid=$!
  echo "${pf_pid}" >"${pid_file}"
  disown "${pf_pid}" 2>/dev/null || true

  # Wait briefly for the listener to come up.
  local i
  for i in 1 2 3 4 5 6 7 8 9 10; do
    if kill -0 "$(cat "${pid_file}")" 2>/dev/null && port_listening "${local_port}"; then
      echo "Port-forward ${name} → 127.0.0.1:${local_port}"
      return 0
    fi
    sleep 0.3
  done

  echo "Failed to start port-forward ${name}. See ${log_file}"
  if [[ -f "${log_file}" ]]; then
    tail -n 20 "${log_file}" || true
  fi
  return 1
}
# Metrics forwards are owned by one supervisor, never by a load-balanced Service.
# State files use Kubernetes pod names (DNS labels), so TOML quoting is unambiguous.
metrics_process_running() {
  local pid="$1" expected="$2" command
  [[ "${pid}" =~ ^[0-9]+$ ]] || return 1
  kill -0 "${pid}" 2>/dev/null || return 1
  command="$(ps -p "${pid}" -o command= 2>/dev/null)" || return 1
  [[ "${command}" == *"${expected}"* ]]
}

stop_metrics_child() {
  local state="$1" pid
  if [[ -f "${state}/pid" ]]; then
    pid="$(cat "${state}/pid")"
    if metrics_process_running "${pid}" "pod/$(basename "${state}")"; then
      kill "${pid}" 2>/dev/null || true
      wait "${pid}" 2>/dev/null || true
    fi
    rm -f "${state}/pid"
  fi
}

publish_metrics_targets() {
  local reachable="$1" state port tmp
  tmp="$(mktemp "${LOCAL_DIR}/connect-metrics.toml.XXXXXX")"
  {
    echo "# Generated by the local Connect pod forwarding supervisor."
    # Explicit empty array keeps the file valid when no Connect pods exist.
    local any=false
    for state in "${METRICS_DIR}"/workers/*; do
      [[ -d "${state}" ]] || continue
      any=true
      port=0
      if [[ "${reachable}" == true && -f "${state}/pid" ]]; then
        port="$(cat "${state}/port")"
      fi
      printf '[[workers]]\nid = "%s"\nurl = "http://127.0.0.1:%s/metrics"\n\n' \
        "$(basename "${state}")" "${port}"
    done
    if [[ "${any}" == false ]]; then echo 'workers = []'; fi
  } >"${tmp}"
  chmod 600 "${tmp}"
  mv -f "${tmp}" "${METRICS_TARGETS}"
}

metrics_supervisor_cleanup() {
  local state
  for state in "${METRICS_DIR}"/workers/*; do
    [[ -d "${state}" ]] || continue
    stop_metrics_child "${state}"
  done
  if [[ -n "${sleeper:-}" ]]; then kill "${sleeper}" 2>/dev/null || true; fi
  # Keep known identities but make stopped endpoints fail, not falsely healthy.
  publish_metrics_targets false
  rm -f "${PID_DIR}/metrics-supervisor.pid"
  rmdir "${METRICS_DIR}/lock" 2>/dev/null || true
}

metrics_supervisor() {
  umask 077
  mkdir -p "${METRICS_DIR}/workers"
  trap metrics_supervisor_cleanup EXIT
  trap 'exit 0' TERM INT
  local pods pod uid phase ready deleting state port pid old_uid
  local next_port=9404 sleeper=""
  # Reclaim only children recorded by a previous supervisor, never listeners.
  for state in "${METRICS_DIR}"/workers/*; do
    [[ -d "${state}" ]] || continue
    stop_metrics_child "${state}"
    port="$(cat "${state}/port")"
    if (( port >= next_port )); then next_port=$((port + 1)); fi
  done
  publish_metrics_targets false
  while true; do
    # Do not filter by readiness: pending, failed and terminating pods still
    # appear as worker identities with deliberately unreachable endpoints.
    if pods="$(kubectl --request-timeout=5s get pods -n "${NAMESPACE}" \
      -l "strimzi.io/name=${CONNECT_CLUSTER}-connect" \
      --sort-by=.metadata.name \
      -o go-template='{{range .items}}{{.metadata.name}} {{.metadata.uid}} {{.status.phase}} {{range .status.conditions}}{{if eq .type "Ready"}}{{.status}}{{end}}{{end}} {{if .metadata.deletionTimestamp}}deleting{{else}}active{{end}}{{"\n"}}{{end}}' \
      2>/dev/null)"; then
      for state in "${METRICS_DIR}"/workers/*; do
        [[ -d "${state}" ]] || continue
        rm -f "${state}/seen"
      done
      while read -r pod uid phase ready deleting; do
        [[ -n "${pod}" ]] || continue
        state="${METRICS_DIR}/workers/${pod}"
        mkdir -p "${state}"
        : >"${state}/seen"
        old_uid=""
        if [[ -f "${state}/uid" ]]; then old_uid="$(cat "${state}/uid")"; fi
        if [[ "${uid}" != "${old_uid}" ]]; then stop_metrics_child "${state}"; fi
        printf '%s\n' "${uid}" >"${state}/uid"
        if [[ ! -f "${state}/port" ]]; then
          while port_listening "${next_port}"; do next_port=$((next_port + 1)); done
          printf '%s\n' "${next_port}" >"${state}/port"
          next_port=$((next_port + 1))
        fi
        if [[ "${phase}" != Running || "${ready}" != True || "${deleting}" != active ]]; then
          stop_metrics_child "${state}"
          continue
        fi
        if [[ -f "${state}/pid" ]]; then
          pid="$(cat "${state}/pid")"
          if metrics_process_running "${pid}" "pod/${pod}"; then continue; fi
          rm -f "${state}/pid"
        fi
        port="$(cat "${state}/port")"
        if port_listening "${port}"; then
          # Someone else owns the port. Choose another; never kill or scrape them.
          while port_listening "${next_port}"; do next_port=$((next_port + 1)); done
          port="${next_port}"
          printf '%s\n' "${port}" >"${state}/port"
          next_port=$((next_port + 1))
        fi
        kubectl --request-timeout=5s port-forward --pod-running-timeout=1s \
          --address 127.0.0.1 -n "${NAMESPACE}" "pod/${pod}" "${port}:9404" \
          >"${state}/forward.log" 2>&1 &
        printf '%s\n' "$!" >"${state}/pid"
      done <<<"${pods}"
      for state in "${METRICS_DIR}"/workers/*; do
        [[ -d "${state}" ]] || continue
        if [[ ! -f "${state}/seen" ]]; then
          stop_metrics_child "${state}"
          rm -rf "${state}"
        fi
      done
      publish_metrics_targets true
    else
      echo "Connect pod discovery failed; retaining known worker identities."
      for state in "${METRICS_DIR}"/workers/*; do
        [[ -d "${state}" ]] || continue
        stop_metrics_child "${state}"
      done
      publish_metrics_targets false
    fi
    # Sleep as a tracked child so TERM interrupts the interval immediately.
    sleep 15 &
    sleeper=$!
    wait "${sleeper}" || true
  done
}

start_metrics_supervisor() {
  local pid_file="${PID_DIR}/metrics-supervisor.pid" pid
  mkdir -p "${METRICS_DIR}"
  if [[ -f "${pid_file}" ]]; then
    pid="$(cat "${pid_file}")"
    if metrics_process_running "${pid}" "${ROOT}/scripts/local-dev.sh metrics-supervisor"; then
      echo "Metrics supervisor already running (pid ${pid})"
      return 0
    fi
    rm -f "${pid_file}"
  fi
  # Atomic lock prevents concurrent port-forward-all commands spawning owners.
  if ! mkdir "${METRICS_DIR}/lock" 2>/dev/null; then
    echo "Metrics supervisor lock exists: run just stop-forwards before restarting."
    return 1
  fi
  nohup bash "${ROOT}/scripts/local-dev.sh" metrics-supervisor \
    >"${METRICS_DIR}/supervisor.log" 2>&1 &
  pid=$!
  printf '%s\n' "${pid}" >"${pid_file}"
  disown "${pid}" 2>/dev/null || true
  echo "Metrics supervisor started (pid ${pid}; targets ${METRICS_TARGETS})"
}

stop_metrics_supervisor() {
  local pid_file="${PID_DIR}/metrics-supervisor.pid" pid state
  if [[ -f "${pid_file}" ]]; then
    pid="$(cat "${pid_file}")"
    if metrics_process_running "${pid}" "${ROOT}/scripts/local-dev.sh metrics-supervisor"; then
      kill "${pid}" 2>/dev/null || true
      # Allow a bounded API request and the EXIT trap to finish before cleanup.
      for _ in 1 2 3 4 5 6 7; do
        if ! metrics_process_running "${pid}" "${ROOT}/scripts/local-dev.sh metrics-supervisor"; then break; fi
        sleep 1
      done
      if metrics_process_running "${pid}" "${ROOT}/scripts/local-dev.sh metrics-supervisor"; then
        echo "Metrics supervisor is still shutting down; retry just stop-forwards."
        return 1
      fi
    fi
  fi
  mkdir -p "${METRICS_DIR}/workers"
  metrics_supervisor_cleanup
}

status_metrics_forwards() {
  local pid state port
  if [[ -f "${PID_DIR}/metrics-supervisor.pid" ]]; then
    pid="$(cat "${PID_DIR}/metrics-supervisor.pid")"
    if metrics_process_running "${pid}" "${ROOT}/scripts/local-dev.sh metrics-supervisor"; then
      echo "  metrics-supervisor: up (pid ${pid}; refresh every 15s)"
    else
      echo "  metrics-supervisor: down (stale pid file)"
    fi
  else
    echo "  metrics-supervisor: down"
  fi
  for state in "${METRICS_DIR}"/workers/*; do
    [[ -d "${state}" ]] || continue
    port="$(cat "${state}/port")"
    if [[ -f "${state}/pid" ]] && metrics_process_running "$(cat "${state}/pid")" "pod/$(basename "${state}")" && port_listening "${port}"; then
      echo "    $(basename "${state}"): up → 127.0.0.1:${port}/metrics"
    else
      echo "    $(basename "${state}"): collection error (no healthy tracked forward)"
    fi
  done
}


port_forward_all() {
  start_port_forward connect svc/my-connect-cluster-connect-api 8083 8083
  start_port_forward kafka svc/my-cluster-kafka-bootstrap 9092 9092
  start_port_forward postgres svc/postgres 5432 5432
  start_port_forward rustfs svc/rustfs 9000 9000
  start_port_forward rustfs-console svc/rustfs 9001 9001
  start_metrics_supervisor

  if curl -sf --connect-timeout 3 "http://127.0.0.1:8083/" >/dev/null; then
    echo "Connect API healthy at http://127.0.0.1:8083/"
  else
    echo "Warning: Connect API did not respond on http://127.0.0.1:8083/ yet."
    echo "  Check: kubectl get pods -n ${NAMESPACE} -l strimzi.io/name=my-connect-cluster-connect"
    echo "  Logs:  ${PID_DIR}/connect.log"
    return 1
  fi
  echo "All port-forwards running in background (just stop-forwards to stop)."
}

status_port_forwards() {
  local name port pid_file pid
  local -a pairs=(
    "connect:8083"
    "kafka:9092"
    "postgres:5432"
    "rustfs:9000"
    "rustfs-console:9001"
  )
  echo "Background port-forwards (.local/port-forwards):"
  for entry in "${pairs[@]}"; do
    name="${entry%%:*}"
    port="${entry##*:}"
    pid_file="${PID_DIR}/${name}.pid"
    if [[ -f "${pid_file}" ]]; then
      pid="$(cat "${pid_file}")"
      if kill -0 "${pid}" 2>/dev/null && port_listening "${port}"; then
        echo "  ${name}: up (pid ${pid} → 127.0.0.1:${port})"
      else
        echo "  ${name}: down (stale pid file)"
      fi
    elif port_listening "${port}"; then
      echo "  ${name}: listening on ${port} (not tracked by just)"
    else
      echo "  ${name}: down"
    fi
  done
  status_metrics_forwards
}

stop_port_forwards() {
  stop_metrics_supervisor
  if [[ ! -d "${PID_DIR}" ]]; then
    echo "No port-forwards to stop."
    # Still clear any stray listeners on known ports.
    kill_port_listeners 8083
    kill_port_listeners 9092
    kill_port_listeners 5432
    kill_port_listeners 9000
    kill_port_listeners 9001
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
  kill_port_listeners 8083
  kill_port_listeners 9092
  kill_port_listeners 5432
  kill_port_listeners 9000
  kill_port_listeners 9001
  echo "Port-forwards stopped."
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
  status-forwards
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
    status-forwards) status_port_forwards ;;
    stop-forwards) stop_port_forwards ;;
    metrics-supervisor) metrics_supervisor ;;
    *) usage; exit 1 ;;
  esac
}

main "$@"
