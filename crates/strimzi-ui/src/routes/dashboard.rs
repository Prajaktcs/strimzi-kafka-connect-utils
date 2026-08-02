use axum::extract::{Query, State};
use serde::Deserialize;
use serde_json::Value;

use crate::blocking::with_connect_client;
use crate::state::AppState;
use crate::views::{
    render, ConnectorSummary, DashboardMetrics, DashboardPage, FailedItem, HtmlResult,
    MissingConfigPage, StatusCount,
};
use std::collections::BTreeMap;

#[derive(Debug, Deserialize)]
pub struct FlashQuery {
    pub flash: Option<String>,
}

pub async fn dashboard(
    State(state): State<AppState>,
    Query(query): Query<FlashQuery>,
) -> HtmlResult {
    if !state.has_connect_url() {
        return render(MissingConfigPage {
            active: "dashboard",
        });
    }

    let url = state.require_connect_url()?;
    let flash = query.flash;
    let page = with_connect_client(url, move |client| build_dashboard(client, flash)).await?;
    render(page)
}

fn build_dashboard(
    client: &strimzi_ops_core::ConnectClient,
    flash: Option<String>,
) -> crate::result::Result<DashboardPage> {
    let cluster = client.get_cluster_info()?;
    let plugins = client.get_connector_plugins()?;
    let all = client.get_all_connectors_status()?;

    let mut running_connectors = 0usize;
    let mut failed_connectors = 0usize;
    let mut total_tasks = 0usize;
    let mut running_tasks = 0usize;
    let mut status_map: BTreeMap<String, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut connectors = Vec::new();

    for (name, info) in &all {
        let status = info.get("status").cloned().unwrap_or(Value::Null);
        let connector_state = status
            .pointer("/connector/state")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_owned();

        *status_map.entry(connector_state.clone()).or_default() += 1;
        if connector_state == "RUNNING" {
            running_connectors += 1;
        } else if connector_state == "FAILED" {
            failed_connectors += 1;
            failures.push(FailedItem {
                name: name.clone(),
                kind: "Connector".to_owned(),
                error: truncate_error(status.pointer("/connector/trace").and_then(Value::as_str)),
            });
        }

        let tasks = status
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        total_tasks += tasks.len();
        let mut tasks_running = 0usize;
        for task in &tasks {
            if task.get("state").and_then(Value::as_str) == Some("RUNNING") {
                running_tasks += 1;
                tasks_running += 1;
            } else if task.get("state").and_then(Value::as_str) == Some("FAILED") {
                let task_id = task
                    .get("id")
                    .map_or_else(|| "?".to_owned(), std::string::ToString::to_string);
                failures.push(FailedItem {
                    name: format!("{name} (Task {task_id})"),
                    kind: "Task".to_owned(),
                    error: truncate_error(task.get("trace").and_then(Value::as_str)),
                });
            }
        }

        connectors.push(ConnectorSummary {
            name: name.clone(),
            connector_type: info
                .pointer("/info/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            state: connector_state,
            tasks_running,
            tasks_total: tasks.len(),
        });
    }

    connectors.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(DashboardPage {
        active: "dashboard",
        version: cluster.version.unwrap_or_else(|| "Unknown".to_owned()),
        kafka_cluster_id: cluster
            .kafka_cluster_id
            .unwrap_or_else(|| "Unknown".to_owned()),
        plugins: plugins.len(),
        metrics: DashboardMetrics {
            total_connectors: all.len(),
            running_connectors,
            failed_connectors,
            running_tasks,
            total_tasks,
        },
        status_distribution: status_map
            .into_iter()
            .map(|(state, count)| StatusCount { state, count })
            .collect(),
        failures,
        connectors,
        flash,
    })
}

fn truncate_error(trace: Option<&str>) -> String {
    let text = trace.unwrap_or("Unknown error");
    if text.len() <= 200 {
        text.to_owned()
    } else {
        format!("{}...", &text[..200])
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn summarizes_running_connector() {
        let mut all = serde_json::Map::new();
        all.insert(
            "demo".to_owned(),
            json!({
                "info": { "type": "source" },
                "status": {
                    "connector": { "state": "RUNNING" },
                    "tasks": [{ "id": 0, "state": "RUNNING" }]
                }
            }),
        );

        let mut running_connectors = 0;
        let mut total_tasks = 0;
        let mut running_tasks = 0;
        for (_name, info) in &all {
            let status = info.get("status").cloned().unwrap_or(Value::Null);
            if status.pointer("/connector/state").and_then(Value::as_str) == Some("RUNNING") {
                running_connectors += 1;
            }
            let tasks = status
                .get("tasks")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            total_tasks += tasks.len();
            running_tasks += tasks
                .iter()
                .filter(|task| task.get("state").and_then(Value::as_str) == Some("RUNNING"))
                .count();
        }

        assert_eq!(running_connectors, 1);
        assert_eq!(running_tasks, 1);
        assert_eq!(total_tasks, 1);
        assert_eq!(truncate_error(Some(&"x".repeat(250))).len(), 203);
    }
}
