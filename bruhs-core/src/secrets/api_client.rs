//! Credentials stay in this module through request execution and response redaction.
use super::SecretStore;
use crate::api_client::{ApiClientError, ApiResult};
use base64::Engine;
use bruhs_models::api_client::*;
use std::{collections::BTreeMap, time::Instant};
use zeroize::Zeroizing;

impl SecretStore {
    pub(crate) async fn execute_api(
        &self,
        client: &reqwest::Client,
        execution: &ApiExecution,
    ) -> ApiResult<ApiResponse> {
        let started = Instant::now();
        let environment = &execution.environment;
        let mut secrets = BTreeMap::new();
        // Resolve only references actually used by headers/body, never unrelated saved credentials.
        let mut needed = std::collections::BTreeSet::new();
        for input in execution
            .request
            .headers
            .iter()
            .map(|header| header.value.as_str())
            .chain(std::iter::once(execution.request.body.as_str()))
        {
            template(input, |key| {
                if environment.secrets.contains_key(key) {
                    needed.insert(key.to_string());
                }
                Ok(String::new())
            })
            .map_err(ApiClientError::Invalid)?;
        }
        // Catalog writes and OS unlock prompts share one credential-store owner.
        for key in needed {
            let name = &environment.secrets[&key];
            let value=self.find(&execution.project_path,name).await.map_err(|_|ApiClientError::Storage("Could not unlock this request's saved credential. Unlock the OS keychain and retry with a new approval.".into()))?
                .filter(|value|!value.is_empty()).ok_or_else(||ApiClientError::Storage(format!("Save credential {name} for this working folder in Settings, Vault, then retry with a new approval.")))?;
            secrets.insert(key, value);
        }
        let interpolate = |text: &str| -> ApiResult<Zeroizing<String>> {
            template(text, |key| {
                secrets
                    .get(key)
                    .map(|value| value.to_string())
                    .or_else(|| environment.values.get(key).cloned())
                    .ok_or_else(|| format!("Define environment variable {key} before sending."))
            })
            .map(Zeroizing::new)
            .map_err(ApiClientError::Invalid)
        };
        let url =
            crate::api_client::validation::plain_template(&execution.request.url, environment)?;
        let method = reqwest::Method::from_bytes(execution.request.method.as_bytes())
            .map_err(|_| ApiClientError::Invalid("Invalid HTTP method.".into()))?;
        let mut request = client.request(method, url);
        let mut expanded_bytes = 0usize;
        for header in &execution.request.headers {
            let value = interpolate(&header.value)?;
            expanded_bytes = expanded_bytes.saturating_add(value.len() + header.name.len());
            let mut parsed = reqwest::header::HeaderValue::from_str(&value).map_err(|_| {
                ApiClientError::Invalid(
                    "A resolved header contains invalid HTTP characters. Check its keychain value."
                        .into(),
                )
            })?;
            parsed.set_sensitive(sensitive_header(&header.name) || header.value.contains("{{"));
            request = request.header(&header.name, parsed);
        }
        let body = interpolate(&execution.request.body)?;
        if expanded_bytes.saturating_add(body.len()) > PAYLOAD_BYTES {
            return Err(ApiClientError::Invalid(
                "The expanded request exceeds 32 KiB. Narrow its body and headers.".into(),
            ));
        }
        // reqwest owns this transient copy until the network operation completes. Never log it.
        request = request.body(body.to_string());
        let mut response = request.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        let mut redactor = Redactor::new(secrets.values().map(|value| value.as_str()));
        let mut headers = Vec::new();
        let mut bytes_count = 0usize;
        for (name, value) in response.headers() {
            bytes_count = bytes_count.saturating_add(name.as_str().len() + value.as_bytes().len());
            if bytes_count > PAYLOAD_BYTES {
                return Err(oversized());
            }
            let value = if sensitive_header(name.as_str()) {
                redactor.changed = true;
                "[redacted]".into()
            } else {
                redactor.redact(value.to_str().unwrap_or("[non-text header]"))
            };
            headers.push(ApiHeader {
                name: redactor.redact(name.as_str()),
                value,
            });
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if bytes_count
                .saturating_add(bytes.len())
                .saturating_add(chunk.len())
                > PAYLOAD_BYTES
            {
                return Err(oversized());
            }
            bytes.extend_from_slice(&chunk);
        }
        let size_bytes = bytes.len();
        let text=std::str::from_utf8(&bytes).map_err(|_|ApiClientError::Network("The response is not UTF-8 text. Request JSON/text or inspect the binary endpoint with an external client.".into()))?;
        let body = redactor.redact(text);
        let result = ApiResponse {
            request_id: execution.request_id,
            status,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            size_bytes,
            headers,
            body,
            redacted: redactor.changed,
        };
        if serde_json::to_vec(&result).map_err(|_| oversized())?.len() > PAYLOAD_BYTES {
            return Err(oversized());
        }
        Ok(result)
    }
}
fn oversized() -> ApiClientError {
    ApiClientError::Network("The response exceeds the 32 KiB transport budget. Narrow it at the endpoint; no partial body was returned.".into())
}
fn network_error(error: reqwest::Error) -> ApiClientError {
    if error.is_timeout() {
        ApiClientError::Cancelled
    } else {
        ApiClientError::Network("The HTTP request failed. Check the destination, TLS certificate and server connection before retrying; a remote write may already have been applied.".into())
    }
}
struct Redactor {
    patterns: Vec<Zeroizing<String>>,
    changed: bool,
}
impl Redactor {
    fn new<'a>(values: impl Iterator<Item = &'a str>) -> Self {
        let mut patterns = Vec::new();
        for value in values {
            if value.is_empty() {
                continue;
            }
            patterns.push(Zeroizing::new(value.to_string()));
            if let Ok(escaped) = serde_json::to_string(value) {
                patterns.push(Zeroizing::new(escaped[1..escaped.len() - 1].into()));
            }
            patterns.push(Zeroizing::new(
                percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)
                    .to_string(),
            ));
            patterns.push(Zeroizing::new(
                url::form_urlencoded::byte_serialize(value.as_bytes()).collect(),
            ));
            patterns.push(Zeroizing::new(
                base64::engine::general_purpose::STANDARD.encode(value),
            ));
            patterns.push(Zeroizing::new(
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value),
            ));
        }
        patterns.sort_by_key(|value| std::cmp::Reverse(value.len()));
        patterns.dedup();
        Self {
            patterns,
            changed: false,
        }
    }
    fn redact(&mut self, text: &str) -> String {
        let mut output = text.to_string();
        for pattern in &self.patterns {
            if output.contains(pattern.as_str()) {
                self.changed = true;
                output = output.replace(pattern.as_str(), "[redacted]");
            }
        }
        output
    }
}
