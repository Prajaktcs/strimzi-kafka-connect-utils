//! Kubernetes helpers for Kafka Connect pod logs via `kubectl`.

use std::process::Command;

use crate::{Error, Result};

/// Build the Strimzi label selector for Kafka Connect pods.
///
/// Strimzi labels Connect pods as `{cluster_name}-connect`.
#[must_use]
pub fn connect_label_selector(cluster_name: &str) -> String {
    let name = if cluster_name.ends_with("-connect") {
        cluster_name.to_owned()
    } else {
        format!("{cluster_name}-connect")
    };
    format!("strimzi.io/name={name}")
}

/// Filter log lines that contain `filter_text`.
#[must_use]
pub fn filter_log_lines(output: &str, filter_text: &str) -> String {
    let filtered: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(filter_text))
        .collect();
    if filtered.is_empty() {
        format!("No log lines matched filter '{filter_text}'")
    } else {
        filtered.join("\n")
    }
}

/// Fetch recent logs from the Kafka Connect cluster pods.
///
/// Runs `kubectl logs -l ... --tail N --prefix=true`. Failures and empty
/// output return user-facing messages (same behaviour as the Python helper).
pub fn fetch_logs(
    cluster_name: &str,
    namespace: Option<&str>,
    lines: u32,
    filter_text: Option<&str>,
) -> Result<String> {
    let selector = connect_label_selector(cluster_name);
    let mut command = Command::new("kubectl");
    command.args([
        "logs",
        "-l",
        &selector,
        "--tail",
        &lines.to_string(),
        "--prefix=true",
    ]);
    if let Some(namespace) = namespace {
        command.args(["-n", namespace]);
    }
    let output = command.output().map_err(|source| Error::Kubectl {
        reason: format!("failed to run kubectl: {source}"),
    })?;

    if !output.status.success() {
        return Ok(format!("Failed to fetch logs: {}", kubectl_detail(&output)));
    }

    let text = String::from_utf8_lossy(&output.stdout).into_owned();

    if let Some(filter) = filter_text {
        return Ok(filter_log_lines(&text, filter));
    }

    if text.is_empty() {
        Ok("No logs returned".to_owned())
    } else {
        Ok(text)
    }
}

/// Annotation on a `KafkaConnect` CR for a REST URL reachable from this process
/// (Ingress, `LoadBalancer`, or port-forward). Without it, the in-cluster Service
/// DNS name is used.
pub const CONNECT_URL_ANNOTATION: &str = "strimzi-ops.io/connect-url";

/// `kubectl get kafkaconnect` JSON.
///
/// When `namespace` is `None`, lists across all namespaces (`-A`).
pub fn fetch_kafkaconnect_list(namespace: Option<&str>) -> Result<serde_json::Value> {
    let mut command = Command::new("kubectl");
    command.args(["get", "kafkaconnect", "-o", "json"]);
    if let Some(namespace) = namespace {
        command.args(["-n", namespace]);
    } else {
        command.arg("-A");
    }
    let output = command.output().map_err(|source| Error::Kubectl {
        reason: format!("failed to run kubectl: {source}"),
    })?;
    if !output.status.success() {
        return Err(Error::KubernetesDiscover {
            reason: kubectl_detail(&output),
        });
    }
    serde_json::from_slice(&output.stdout).map_err(|err| Error::KubernetesDiscover {
        reason: format!("cannot parse kubectl JSON: {err}"),
    })
}

fn kubectl_detail(output: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&output.stderr);
    let out = String::from_utf8_lossy(&output.stdout);
    let detail = err.trim();
    let detail = if detail.is_empty() {
        out.trim()
    } else {
        detail
    };
    if detail.is_empty() {
        format!("kubectl exited with {}", output.status)
    } else {
        detail.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_selector_appends_connect_suffix() {
        assert_eq!(
            connect_label_selector("my-cluster"),
            "strimzi.io/name=my-cluster-connect"
        );
    }

    #[test]
    fn label_selector_keeps_existing_connect_suffix() {
        assert_eq!(
            connect_label_selector("my-cluster-connect"),
            "strimzi.io/name=my-cluster-connect"
        );
    }

    #[test]
    fn filters_matching_lines() {
        let output = "pod/a: hello connector-x\npod/b: unrelated\npod/c: connector-x done";
        assert_eq!(
            filter_log_lines(output, "connector-x"),
            "pod/a: hello connector-x\npod/c: connector-x done"
        );
    }

    #[test]
    fn filter_with_no_matches_returns_message() {
        assert_eq!(
            filter_log_lines("nope\nstill nope", "connector-x"),
            "No log lines matched filter 'connector-x'"
        );
    }
}
