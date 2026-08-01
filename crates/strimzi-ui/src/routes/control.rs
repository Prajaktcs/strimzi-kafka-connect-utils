use std::sync::Arc;

use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value};
use strimzi_ops_core::{
    to_strimzi_yaml, validate_config, ConnectClient, CreateConnectorRequest, SnapshotTrigger,
};

use crate::error::Error;
use crate::state::AppState;
use crate::views::{
    redirect, render, ControlPage, ControlRow, CreatePage, EditPage, HtmlResult, MissingConfigPage,
    SnapshotPage, YamlPage,
};

pub async fn control_list(
    State(state): State<AppState>,
    Query(query): Query<ControlQuery>,
) -> HtmlResult {
    let Some(client) = state.client.clone() else {
        return render(MissingConfigPage { active: "control" });
    };

    let page = tokio::task::spawn_blocking(move || build_control_page(&client, query))
        .await
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })??;
    render(page)
}

#[derive(Debug, Deserialize)]
pub struct ControlQuery {
    pub flash: Option<String>,
    pub focus: Option<String>,
}

fn build_control_page(
    client: &Arc<ConnectClient>,
    query: ControlQuery,
) -> crate::result::Result<ControlPage> {
    let all = client.get_all_connectors_status()?;
    let mut connectors = Vec::new();
    for (name, info) in all {
        let status = info.get("status").cloned().unwrap_or(Value::Null);
        let state = status
            .pointer("/connector/state")
            .and_then(Value::as_str)
            .unwrap_or("UNKNOWN")
            .to_owned();
        let tasks = status
            .get("tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let tasks_running = tasks
            .iter()
            .filter(|task| task.get("state").and_then(Value::as_str) == Some("RUNNING"))
            .count();
        let focused = query.focus.as_deref() == Some(name.as_str());
        connectors.push(ControlRow {
            name,
            state,
            tasks_running,
            tasks_total: tasks.len(),
            focused,
        });
    }
    connectors.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ControlPage {
        active: "control",
        connectors,
        flash: query.flash,
        focus: query.focus,
    })
}

pub async fn pause_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> HtmlResult {
    lifecycle_action(state, name, ConnectClient::pause_connector, "Paused").await
}

pub async fn resume_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> HtmlResult {
    lifecycle_action(state, name, ConnectClient::resume_connector, "Resumed").await
}

pub async fn restart_connector(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> HtmlResult {
    lifecycle_action(state, name, ConnectClient::restart_connector, "Restarted").await
}

async fn lifecycle_action<F>(state: AppState, name: String, action: F, verb: &str) -> HtmlResult
where
    F: FnOnce(&ConnectClient, &str) -> strimzi_ops_core::Result<()> + Send + 'static,
{
    let client = state.require_client()?;
    let name_for_msg = name.clone();
    tokio::task::spawn_blocking(move || action(&client, &name))
        .await
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })??;
    Ok(redirect(&format!(
        "/control?flash={verb}%20{name_for_msg}&focus={name_for_msg}"
    )))
}

pub async fn snapshot_form(Path(name): Path<String>) -> HtmlResult {
    render(SnapshotPage {
        active: "control",
        name,
        flash: None,
    })
}

#[derive(Debug, Deserialize)]
pub struct SnapshotForm {
    pub snapshot_type: String,
    pub tables: Option<String>,
}

pub async fn snapshot_submit(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Form(form): Form<SnapshotForm>,
) -> HtmlResult {
    let client = state.require_client()?;
    let bootstrap = state.settings.bootstrap_servers.clone();
    let tables: Option<Vec<String>> = form.tables.as_ref().map(|raw| {
        raw.split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    });
    let snapshot_type = form.snapshot_type.clone();
    let name_clone = name.clone();

    let result = tokio::task::spawn_blocking(move || {
        let trigger = SnapshotTrigger::new((*client).clone(), bootstrap);
        trigger.trigger(&name_clone, &snapshot_type, tables.as_deref())
    })
    .await
    .map_err(|err| Error::Internal {
        reason: err.to_string(),
    })??;

    let msg = urlencoding_encode(&format!("Snapshot {}: {}", result.status, result.message));
    Ok(redirect(&format!("/control?flash={msg}&focus={name}")))
}

pub async fn yaml_view(State(state): State<AppState>, Path(name): Path<String>) -> HtmlResult {
    let yaml = load_yaml(&state, &name).await?;
    render(YamlPage {
        active: "control",
        name,
        yaml,
    })
}

pub async fn yaml_download(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    match load_yaml(&state, &name).await {
        Ok(yaml) => {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/yaml; charset=utf-8"),
            );
            if let Ok(value) =
                HeaderValue::from_str(&format!("attachment; filename=\"{name}.yaml\""))
            {
                headers.insert(header::CONTENT_DISPOSITION, value);
            }
            (headers, yaml).into_response()
        }
        Err(err) => err.into_response(),
    }
}

async fn load_yaml(state: &AppState, name: &str) -> crate::result::Result<String> {
    let client = state.require_client()?;
    let cluster = state.cluster_name().to_owned();
    let name_clone = name.to_owned();
    tokio::task::spawn_blocking(move || {
        let config = client.get_connector_config(&name_clone)?;
        Ok::<_, strimzi_ops_core::Error>(to_strimzi_yaml(&name_clone, &config, &cluster))
    })
    .await
    .map_err(|err| Error::Internal {
        reason: err.to_string(),
    })?
    .map_err(Error::from)
}

pub async fn edit_form(State(state): State<AppState>, Path(name): Path<String>) -> HtmlResult {
    let client = state.require_client()?;
    let name_clone = name.clone();
    let config = tokio::task::spawn_blocking(move || client.get_connector_config(&name_clone))
        .await
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })??;
    let config_json = serde_json::to_string_pretty(&config).map_err(|err| Error::Json {
        reason: err.to_string(),
    })?;
    render(EditPage {
        active: "control",
        name,
        config_json,
        validation_error: None,
        flash: None,
    })
}

#[derive(Debug, Deserialize)]
pub struct EditForm {
    pub config_json: String,
    pub force: Option<String>,
}

pub async fn edit_submit(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Form(form): Form<EditForm>,
) -> HtmlResult {
    let client = state.require_client()?;
    let parsed: Map<String, Value> =
        serde_json::from_str(&form.config_json).map_err(|err| Error::Json {
            reason: err.to_string(),
        })?;

    let force = form.force.as_deref() == Some("1");
    let name_clone = name.clone();
    let config_json = form.config_json.clone();

    let outcome = tokio::task::spawn_blocking(move || {
        let report = validate_config(parsed.clone(), Some(&name_clone), None)?;
        if !report.valid && !force {
            return Ok::<_, Error>(EditOutcome::Invalid {
                formatted: report.formatted,
                config_json,
            });
        }
        client.update_connector(&name_clone, &parsed)?;
        Ok(EditOutcome::Updated)
    })
    .await
    .map_err(|err| Error::Internal {
        reason: err.to_string(),
    })??;

    match outcome {
        EditOutcome::Updated => Ok(redirect(&format!(
            "/control?flash=Updated%20{name}&focus={name}"
        ))),
        EditOutcome::Invalid {
            formatted,
            config_json,
        } => render(EditPage {
            active: "control",
            name,
            config_json,
            validation_error: Some(formatted),
            flash: None,
        }),
    }
}

enum EditOutcome {
    Updated,
    Invalid {
        formatted: String,
        config_json: String,
    },
}

pub async fn create_form() -> HtmlResult {
    render(CreatePage {
        active: "control",
        config_json: "{\n  \"name\": \"my-connector\",\n  \"config\": {\n  }\n}".to_owned(),
        flash: None,
        error: None,
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateForm {
    pub config_json: String,
}

pub async fn create_submit(
    State(state): State<AppState>,
    Form(form): Form<CreateForm>,
) -> HtmlResult {
    let client = state.require_client()?;
    let value: Value = serde_json::from_str(&form.config_json).map_err(|err| Error::Json {
        reason: err.to_string(),
    })?;

    let request = create_request_from_value(value).map_err(|reason| Error::Json { reason })?;
    let name = request.name.clone();

    tokio::task::spawn_blocking(move || client.create_connector(&request))
        .await
        .map_err(|err| Error::Internal {
            reason: err.to_string(),
        })??;

    Ok(redirect(&format!(
        "/control?flash=Created%20{name}&focus={name}"
    )))
}

fn create_request_from_value(value: Value) -> std::result::Result<CreateConnectorRequest, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "create payload must be a JSON object".to_owned())?;
    if let (Some(Value::String(name)), Some(Value::Object(config))) =
        (obj.get("name"), obj.get("config"))
    {
        return Ok(CreateConnectorRequest {
            name: name.clone(),
            config: config.clone(),
        });
    }
    if let Some(Value::String(name)) = obj.get("name") {
        let mut config = obj.clone();
        config.remove("name");
        return Ok(CreateConnectorRequest {
            name: name.clone(),
            config,
        });
    }
    Err("create payload must include name and config".to_owned())
}

fn urlencoding_encode(input: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}
