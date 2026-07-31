"""Kubernetes utilities for Strimzi Ops."""

import logging
import subprocess

logger = logging.getLogger(__name__)


def _connect_label_selector(cluster_name: str) -> str:
    """
    Build the Strimzi label selector for Kafka Connect pods.

    Strimzi labels Connect pods as ``{cluster_name}-connect``.
    """
    name = cluster_name if cluster_name.endswith("-connect") else f"{cluster_name}-connect"
    return f"strimzi.io/name={name}"


def get_connect_pod_name(cluster_name: str) -> str | None:
    """Get the name of one of the Kafka Connect pods for a given cluster."""
    try:
        cmd = [
            "kubectl",
            "get",
            "pods",
            "-l",
            _connect_label_selector(cluster_name),
            "-o",
            "jsonpath={.items[0].metadata.name}",
        ]
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return result.stdout.strip() or None
    except Exception as e:
        logger.error(f"Failed to find connect pod for cluster {cluster_name}: {e}")
        return None


def fetch_logs(cluster_name: str, lines: int = 100, filter_text: str | None = None) -> str:
    """
    Fetch recent logs from the Kafka Connect cluster (non-blocking).

    Args:
        cluster_name: Strimzi KafkaConnect cluster name
        lines: Number of log lines to fetch per pod
        filter_text: Optional substring filter (e.g. connector name)

    Returns:
        Combined log text
    """
    cmd = [
        "kubectl",
        "logs",
        "-l",
        _connect_label_selector(cluster_name),
        "--tail",
        str(lines),
        "--prefix=true",
    ]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=True, timeout=30)
        output = result.stdout
    except subprocess.TimeoutExpired:
        logger.error(f"Timed out fetching logs for cluster {cluster_name}")
        return f"Timed out fetching logs for cluster '{cluster_name}'"
    except subprocess.CalledProcessError as e:
        err = (e.stderr or e.stdout or str(e)).strip()
        logger.error(f"Failed to fetch logs for cluster {cluster_name}: {err}")
        return f"Failed to fetch logs: {err}"

    if filter_text:
        filtered = [line for line in output.splitlines() if filter_text in line]
        return "\n".join(filtered) if filtered else f"No log lines matched filter '{filter_text}'"

    return output or "No logs returned"
