use std::time::Duration;

use reqwest::blocking::{Client, Response};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use url::Url;

use crate::connect::types::{
    ClusterInfo, ConnectorConfig, ConnectorPlugin, CreateConnectorRequest,
};
use crate::{Error, Result};

/// Default HTTP timeout for Connect REST calls.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Blocking Kafka Connect REST API client.
#[derive(Debug, Clone)]
pub struct ConnectClient {
    base_url: Url,
    http: Client,
}

impl ConnectClient {
    /// Create a client for `connect_url` (for example `http://localhost:8083`).
    pub fn new(connect_url: &str) -> Result<Self> {
        Self::with_timeout(connect_url, DEFAULT_TIMEOUT)
    }

    /// Create a client with a custom request timeout.
    pub fn with_timeout(connect_url: &str, timeout: Duration) -> Result<Self> {
        let trimmed = connect_url.trim_end_matches('/');
        let base_url = Url::parse(trimmed).map_err(|source| Error::InvalidConnectUrl {
            url: trimmed.to_owned(),
            reason: source.to_string(),
        })?;
        let http = Client::builder()
            .timeout(timeout)
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::CONTENT_TYPE,
                    reqwest::header::HeaderValue::from_static("application/json"),
                );
                headers.insert(
                    reqwest::header::ACCEPT,
                    reqwest::header::HeaderValue::from_static("application/json"),
                );
                headers
            })
            .build()
            .map_err(|source| Error::ConnectHttp {
                method: "BUILD".to_owned(),
                path: String::new(),
                reason: source.to_string(),
            })?;
        Ok(Self { base_url, http })
    }

    /// Kafka Connect base URL.
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn get_cluster_info(&self) -> Result<ClusterInfo> {
        self.request_json(Method::GET, "/", None)
    }

    pub fn get_connector_plugins(&self) -> Result<Vec<ConnectorPlugin>> {
        self.request_json(Method::GET, "connector-plugins", None)
    }

    pub fn list_connectors(&self) -> Result<Vec<String>> {
        self.request_json(Method::GET, "connectors", None)
    }

    pub fn get_all_connectors_status(&self) -> Result<Map<String, Value>> {
        self.request_json(Method::GET, "connectors?expand=status&expand=info", None)
    }

    pub fn get_connector_info(&self, connector_name: &str) -> Result<Value> {
        self.request_json(Method::GET, &format!("connectors/{connector_name}"), None)
    }

    pub fn get_connector_status(&self, connector_name: &str) -> Result<Value> {
        self.request_json(
            Method::GET,
            &format!("connectors/{connector_name}/status"),
            None,
        )
    }

    pub fn get_connector_config(&self, connector_name: &str) -> Result<ConnectorConfig> {
        self.request_json(
            Method::GET,
            &format!("connectors/{connector_name}/config"),
            None,
        )
    }

    pub fn create_connector(&self, request: &CreateConnectorRequest) -> Result<Value> {
        let body = serde_json::to_value(request).map_err(|source| Error::ConnectHttp {
            method: "POST".to_owned(),
            path: "connectors".to_owned(),
            reason: source.to_string(),
        })?;
        self.request_json(Method::POST, "connectors", Some(&body))
    }

    pub fn update_connector(
        &self,
        connector_name: &str,
        config: &ConnectorConfig,
    ) -> Result<Value> {
        let body = Value::Object(config.clone());
        self.request_json(
            Method::PUT,
            &format!("connectors/{connector_name}/config"),
            Some(&body),
        )
    }

    pub fn delete_connector(&self, connector_name: &str) -> Result<()> {
        self.request_empty(Method::DELETE, &format!("connectors/{connector_name}"))
    }

    pub fn pause_connector(&self, connector_name: &str) -> Result<()> {
        self.request_empty(Method::PUT, &format!("connectors/{connector_name}/pause"))
    }

    pub fn resume_connector(&self, connector_name: &str) -> Result<()> {
        self.request_empty(Method::PUT, &format!("connectors/{connector_name}/resume"))
    }

    pub fn restart_connector(&self, connector_name: &str) -> Result<()> {
        self.request_empty(
            Method::POST,
            &format!("connectors/{connector_name}/restart"),
        )
    }

    pub fn restart_connector_task(&self, connector_name: &str, task_id: u32) -> Result<()> {
        self.request_empty(
            Method::POST,
            &format!("connectors/{connector_name}/tasks/{task_id}/restart"),
        )
    }

    fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<T> {
        let response = self.send(method.clone(), path, body)?;
        response.json().map_err(|source| Error::ConnectHttp {
            method: method.to_string(),
            path: path.to_owned(),
            reason: format!("cannot decode JSON response: {source}"),
        })
    }

    fn request_empty(&self, method: Method, path: &str) -> Result<()> {
        self.send(method, path, None)?;
        Ok(())
    }

    fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Response> {
        let url = self.join_path(path)?;
        let mut builder = self.http.request(method.clone(), url.clone());
        if let Some(value) = body {
            builder = builder.json(value);
        }

        let response = builder.send().map_err(|source| {
            let hint = if source.is_connect() || source.is_timeout() {
                format!(
                    "cannot reach {url} ({source}). Is Kafka Connect running and port-forwarded? Try `just port-forward-all` (or `just ui`, which starts forwards first)."
                )
            } else {
                format!("request to {url} failed: {source}")
            };
            Error::ConnectHttp {
                method: method.to_string(),
                path: if path.is_empty() {
                    "/".to_owned()
                } else {
                    path.to_owned()
                },
                reason: hint,
            }
        })?;

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let reason = response.text().unwrap_or_else(|_| status.to_string());
        Err(Error::ConnectHttp {
            method: method.to_string(),
            path: path.to_owned(),
            reason: format!("HTTP {status}: {reason}"),
        })
    }

    fn join_path(&self, path: &str) -> Result<Url> {
        if path.is_empty() || path == "/" {
            // Ensure a trailing slash so Connect root (`GET /`) resolves correctly.
            let mut url = self.base_url.clone();
            if !url.path().ends_with('/') {
                url.set_path(&format!("{}/", url.path().trim_end_matches('/')));
            }
            return Ok(url);
        }
        let relative = path.trim_start_matches('/');
        self.base_url
            .join(relative)
            .map_err(|source| Error::InvalidConnectUrl {
                url: format!("{}{path}", self.base_url),
                reason: source.to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use httpmock::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::connect::CreateConnectorRequest;

    #[test]
    fn list_connectors_parses_json_array() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(GET).path("/connectors");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(json!(["a", "b"]));
        });

        let client = ConnectClient::new(&server.base_url()).expect("client");
        let names = client.list_connectors().expect("list");
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
        mock.assert();
    }

    #[test]
    fn pause_connector_accepts_empty_body() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(PUT).path("/connectors/demo/pause");
            then.status(202);
        });

        let client = ConnectClient::new(&server.base_url()).expect("client");
        client.pause_connector("demo").expect("pause");
        mock.assert();
    }

    #[test]
    fn create_connector_posts_body() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(POST).path("/connectors").json_body(json!({
                "name": "demo",
                "config": { "connector.class": "x", "tasks.max": "1" }
            }));
            then.status(201)
                .header("content-type", "application/json")
                .json_body(json!({ "name": "demo" }));
        });

        let client = ConnectClient::new(&server.base_url()).expect("client");
        let mut config = Map::new();
        config.insert("connector.class".to_owned(), json!("x"));
        config.insert("tasks.max".to_owned(), json!("1"));
        let created = client
            .create_connector(&CreateConnectorRequest {
                name: "demo".to_owned(),
                config,
            })
            .expect("create");
        assert_eq!(created["name"], "demo");
        mock.assert();
    }

    #[test]
    fn http_error_surfaces_status() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/connectors/missing");
            then.status(404).body("not found");
        });

        let client = ConnectClient::new(&server.base_url()).expect("client");
        let err = client
            .get_connector_info("missing")
            .expect_err("should fail");
        let message = err.to_string();
        assert!(message.contains("404"), "{message}");
    }
}
