use std::collections::BTreeSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use reqwest::header::HeaderValue;
use serde::Deserialize;
use url::Url;

/// A hard limit bounds threads, response memory, and total work per collection.
pub(super) const MAX_WORKERS: usize = 32;
const MAX_TARGET_FILE_BYTES: u64 = 128 * 1024;
const MAX_URL_BYTES: usize = 2048;
const MAX_TOKEN_BYTES: usize = 4096;
const MAX_CA_FILE_BYTES: u64 = 64 * 1024;

// Intentionally no Debug: URLs and credentials must never reach public output.
pub(super) struct Target {
    pub id: String,
    pub url: Url,
    pub authorization: Option<HeaderValue>,
    pub ca_file: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetsFile {
    #[serde(default)]
    workers: Vec<TargetConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetConfig {
    id: String,
    url: String,
    bearer_token: Option<String>,
    ca_file: Option<String>,
}

pub(super) enum TargetsError {
    Missing,
    Invalid(&'static str),
}

pub(super) fn read_targets(path: &Path) -> Result<Vec<Target>, TargetsError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            TargetsError::Missing
        } else {
            TargetsError::Invalid("cannot read metrics targets")
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_TARGET_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TargetsError::Invalid("cannot read metrics targets"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_TARGET_FILE_BYTES {
        return Err(TargetsError::Invalid("metrics targets exceed size limit"));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| TargetsError::Invalid("cannot parse metrics targets"))?;
    parse_targets(text, path.parent().unwrap_or_else(|| Path::new(".")))
}

fn parse_targets(text: &str, directory: &Path) -> Result<Vec<Target>, TargetsError> {
    let config: TargetsFile =
        toml::from_str(text).map_err(|_| TargetsError::Invalid("cannot parse metrics targets"))?;
    if config.workers.len() > MAX_WORKERS {
        return Err(TargetsError::Invalid(
            "too many metrics workers (maximum 32)",
        ));
    }
    let mut targets = Vec::with_capacity(config.workers.len());
    for worker in config.workers {
        if worker.id.is_empty()
            || worker.id.len() > 253
            || !worker.id.as_bytes()[0].is_ascii_alphanumeric()
            || !worker
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(TargetsError::Invalid("invalid metrics worker id"));
        }
        if worker.url.len() > MAX_URL_BYTES {
            return Err(TargetsError::Invalid("invalid metrics endpoint"));
        }
        let url = Url::parse(&worker.url)
            .map_err(|_| TargetsError::Invalid("invalid metrics endpoint"))?;
        if url.scheme() != "https" {
            return Err(TargetsError::Invalid("metrics endpoints require HTTPS"));
        }
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(TargetsError::Invalid("invalid metrics endpoint"));
        }
        let authorization = worker
            .bearer_token
            .as_deref()
            .map(|token| {
                if token.is_empty()
                    || token.len() > MAX_TOKEN_BYTES
                    || !token.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
                    })
                {
                    return Err(TargetsError::Invalid("invalid metrics bearer token"));
                }
                let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| TargetsError::Invalid("invalid metrics bearer token"))?;
                value.set_sensitive(true);
                Ok(value)
            })
            .transpose()?;
        let ca_file = worker.ca_file.as_deref().map(|path| directory.join(path));
        targets.push(Target {
            id: worker.id,
            url,
            authorization,
            ca_file,
        });
    }
    {
        let mut ids = BTreeSet::new();
        let mut urls = BTreeSet::new();
        for target in &targets {
            if !ids.insert(&target.id) {
                return Err(TargetsError::Invalid("duplicate metrics worker id"));
            }
            // Port zero is the supervisor's unreachable placeholder, not a
            // shared live exporter. Every identity must fail independently.
            if target.url.port() != Some(0) && !urls.insert(target.url.as_str()) {
                return Err(TargetsError::Invalid("duplicate metrics endpoint"));
            }
        }
    }
    Ok(targets)
}

pub(super) fn read_roots(path: &Path) -> Result<Vec<reqwest::Certificate>, &'static str> {
    let file = File::open(path).map_err(|_| "cannot read metrics CA certificate")?;
    let mut bytes = Vec::new();
    file.take(MAX_CA_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read metrics CA certificate")?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CA_FILE_BYTES {
        return Err("metrics CA certificate exceeds size limit");
    }
    let roots = reqwest::Certificate::from_pem_bundle(&bytes)
        .map_err(|_| "cannot parse metrics CA certificate")?;
    if roots.is_empty() {
        return Err("cannot parse metrics CA certificate");
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;

    use super::*;

    fn failure(text: &str) -> &'static str {
        match parse_targets(text, Path::new(".")) {
            Err(TargetsError::Invalid(reason)) => reason,
            _ => panic!("expected invalid targets"),
        }
    }

    #[test]
    fn rejects_duplicate_workers_and_endpoints_instead_of_double_counting() {
        assert_eq!(
            failure("[[workers]]\nid='pod-a'\nurl='https://localhost:1/metrics'\n[[workers]]\nid='pod-a'\nurl='https://localhost:2/metrics'"),
            "duplicate metrics worker id"
        );
        assert_eq!(
            failure("[[workers]]\nid='pod-a'\nurl='https://localhost:1/metrics'\n[[workers]]\nid='pod-b'\nurl='https://localhost:1/metrics'"),
            "duplicate metrics endpoint"
        );
    }

    #[test]
    fn rejects_url_credentials_queries_and_injected_headers_without_echoing() {
        for url in [
            "https://user:private-token@localhost/metrics",
            "https://localhost/metrics?token=private-token",
            "https://localhost/metrics#private-token",
            "https://localhost:invalid/private-token",
        ] {
            assert_eq!(
                failure(&format!("[[workers]]\nid='pod-a'\nurl='{url}'")),
                "invalid metrics endpoint"
            );
        }
        assert_eq!(
            failure("[[workers]]\nid='pod-a'\nurl='https://localhost/metrics'\nbearer_token=\"private-token\\r\\nInjected: secret\""),
            "invalid metrics bearer token"
        );
    }

    #[test]
    fn rejects_every_plaintext_endpoint_with_or_without_authentication() {
        for endpoint in [
            "http://metrics.example.internal/metrics",
            "http://localhost/metrics",
            "http://127.0.0.1/metrics",
            "http://127.0.0.2/metrics",
            "http://[::1]/metrics",
            "http://127.0.0.1.example.internal/metrics",
            "http://192.0.2.1/metrics",
            "http://[::ffff:192.0.2.1]/metrics",
            "file:///private-token",
        ] {
            for auth in ["", "\nbearer_token='private-token'"] {
                assert_eq!(
                    failure(&format!("[[workers]]\nid='worker'\nurl='{endpoint}'{auth}")),
                    "metrics endpoints require HTTPS"
                );
            }
        }
        for endpoint in [
            "https://metrics.example.internal/metrics",
            "https://localhost/metrics",
            "https://127.0.0.1/metrics",
            "https://[::1]/metrics",
            "https://127.0.0.1:0/metrics",
        ] {
            assert!(parse_targets(
                &format!("[[workers]]\nid='worker'\nurl='{endpoint}'"),
                Path::new("."),
            )
            .is_ok());
        }
    }

    #[test]
    fn rejects_unbounded_membership_before_starting_any_requests() {
        let mut text = String::new();
        for index in 0..=MAX_WORKERS {
            writeln!(
                text,
                "[[workers]]\nid='pod-{index}'\nurl='https://localhost:{}/metrics'",
                index + 1
            )
            .unwrap();
        }
        assert_eq!(failure(&text), "too many metrics workers (maximum 32)");
    }
}
