//! Conditions for routing an authenticated webhook to individual tasks.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{borrow::Cow, collections::BTreeMap};

mod dedupe;
pub use dedupe::*;
mod security;
pub use security::*;
mod prompt;
pub use prompt::*;

pub const MAX_WEBHOOK_FILTERS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookFilter {
    pub path: String,
    #[serde(flatten)]
    pub test: WebhookTest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", content = "value", rename_all = "camelCase")]
pub enum WebhookTest {
    Equals(String),
    NotEquals(String),
    In(Vec<String>),
    StartsWith(String),
    Exists,
}

/// Request metadata available to routing and to explicit prompt placeholders.
#[derive(Default)]
pub struct WebhookMetadata {
    pub headers: BTreeMap<String, Vec<String>>,
    pub query: BTreeMap<String, Vec<String>>,
    /// Original ingress bytes, including any UTF-8 BOM. Signatures cover these bytes.
    pub raw_body: Option<Vec<u8>>,
    pub uri: Option<String>,
}

impl WebhookMetadata {
    /// Fetch's UTF-8 text decoding drops one leading BOM and replaces invalid bytes.
    pub fn body_text(&self) -> Option<Cow<'_, str>> {
        let bytes = self.raw_body.as_deref()?;
        Some(String::from_utf8_lossy(
            bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes),
        ))
    }

    pub fn is_form(&self) -> bool {
        self.headers
            .get("content-type")
            .and_then(|values| values.first())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
    }

    /// Structured fields for routing and persistence. The whole-body placeholder
    /// still uses the original decoded text, including whitespace and form escapes.
    pub fn payload(&self) -> Option<Value> {
        let text = self.body_text()?;
        Some(if self.is_form() {
            // URLSearchParams treats a leading '?' as a query prefix. Later keys win.
            let encoded = text.strip_prefix('?').unwrap_or(&text);
            Value::Object(
                form_urlencoded::parse(encoded.as_bytes())
                    .map(|(key, value)| (key.into_owned(), Value::String(value.into_owned())))
                    .collect(),
            )
        } else {
            serde_json::from_str(&text).unwrap_or_else(|_| Value::String(text.into_owned()))
        })
    }
}

pub fn sensitive_webhook_field(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase().replace(['-', '_', '.'], "");
    [
        "authorization",
        "cookie",
        "token",
        "apikey",
        "password",
        "passwd",
        "secret",
        "signature",
        "credential",
        "privatekey",
        "sessionid",
    ]
    .iter()
    .any(|key| normalized.contains(key))
        || matches!(normalized.as_str(), "key" | "auth" | "session")
}

impl WebhookFilter {
    pub fn validate(&self) -> Result<(), &'static str> {
        let mut path = self.path.split('.');
        if self.path.len() > 512
            || !matches!(path.next(), Some("body" | "headers" | "query"))
            || path.clone().count() > 32
            || path.any(|s| s.is_empty() || s.chars().any(char::is_whitespace))
        {
            return Err("Enter a path under body, headers or query, such as body.action.");
        }
        let valid = |s: &str| !s.is_empty() && s.len() <= 4096;
        match &self.test {
            WebhookTest::Exists => Ok(()),
            WebhookTest::In(items)
                if !items.is_empty()
                    && items.len() <= 256
                    && items
                        .iter()
                        .all(|v| valid(v) && v.trim() == v && !v.contains(',')) =>
            {
                Ok(())
            }
            WebhookTest::Equals(v) | WebhookTest::NotEquals(v) | WebhookTest::StartsWith(v)
                if valid(v) =>
            {
                Ok(())
            }
            _ => Err("Enter a value, or a comma-separated list for “is one of”."),
        }
    }

    pub fn matches(&self, body: &Value, request: &WebhookMetadata) -> bool {
        let mut parts = self.path.split('.');
        let root = parts.next();
        let mut values = Vec::new();
        let present = match root {
            Some("body") if self.path == "body" && request.raw_body.is_some() => {
                values.extend(request.body_text());
                true
            }
            Some("body") => {
                let path = parts.collect::<Vec<_>>();
                let Some(selected) = dedupe::body_field(body, &path) else {
                    return false;
                };
                let wildcard = path.iter().any(|part| part.ends_with("[]"));
                let nodes = match selected {
                    Value::Array(nodes) if wildcard => nodes,
                    single => vec![single],
                };
                let present = nodes.iter().any(|node| !node.is_null());
                for node in nodes {
                    match node {
                        Value::String(value) => values.push(Cow::Owned(value)),
                        Value::Number(_) | Value::Bool(_) => {
                            values.push(Cow::Owned(node.to_string()))
                        }
                        _ => {}
                    }
                }
                if wildcard && matches!(self.test, WebhookTest::NotEquals(_)) && values.is_empty() {
                    return true;
                }
                present
            }
            Some("headers" | "query") => {
                let headers = root == Some("headers");
                let map = if headers {
                    &request.headers
                } else {
                    &request.query
                };
                let Some(key) = parts.next() else {
                    return matches!(self.test, WebhookTest::Exists);
                };
                let key = key.strip_suffix("[]").unwrap_or(key);
                if parts.any(|p| p != "[]") {
                    return false;
                }
                let key = if headers {
                    Cow::Owned(key.to_ascii_lowercase())
                } else {
                    Cow::Borrowed(key)
                };
                let found = map.get(key.as_ref());
                if let Some(found) = found {
                    values.extend(found.iter().map(|v| Cow::Borrowed(v.as_str())));
                }
                found.is_some()
            }
            _ => false,
        };
        match &self.test {
            WebhookTest::Exists => present,
            WebhookTest::NotEquals(want) => {
                !values.is_empty() && values.iter().all(|v| v.as_ref() != want)
            }
            WebhookTest::Equals(want) => values.iter().any(|v| v.as_ref() == want),
            WebhookTest::StartsWith(prefix) => values.iter().any(|v| v.starts_with(prefix)),
            WebhookTest::In(choices) => values
                .iter()
                .any(|v| choices.iter().any(|choice| choice == v.as_ref())),
        }
    }
}

/// How one task handled an authenticated webhook delivery at receipt time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookOutcome {
    WindowOpened,
    Queued,
    AwaitingReview,
    Coalesced,
    Filtered,
    Paused,
    Throttled,
    Duplicate,
    Rejected,
    RejectedSignature,
    Expired,
}

impl WebhookOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::WindowOpened => "Opened window",
            Self::Queued => "Queued",
            Self::AwaitingReview => "Awaiting review",
            Self::Coalesced => "Coalesced",
            Self::Filtered => "Filtered",
            Self::Paused => "Paused",
            Self::Throttled => "Throttled",
            Self::Duplicate => "Duplicate",
            Self::Expired => "Expired",
            Self::RejectedSignature => "Invalid signature",
            Self::Rejected => "Rejected",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDeliverySummary {
    pub id: String,
    pub received_at: crate::EpochMs,
    pub outcome: WebhookOutcome,
    #[serde(default)]
    pub missing_field_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub summary: WebhookDeliverySummary,
    pub headers: String,
    pub query: String,
    pub body: String,
    pub body_truncated: bool,
    #[serde(default)]
    pub signature_verified: bool,
    pub error: Option<String>,
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
}

#[cfg(test)]
mod request_body_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn body_filters_and_templates_resolve_the_same_form_and_wildcard_paths() {
        let mut request = WebhookMetadata {
            raw_body: Some(b"?state=held&state=ready+now".to_vec()),
            ..Default::default()
        };
        request.headers.insert(
            "content-type".into(),
            vec!["application/x-www-form-urlencoded".into()],
        );
        let payload = request.payload().unwrap();
        let check = |path: &str, test, body: &Value, request: &WebhookMetadata| {
            WebhookFilter {
                path: path.into(),
                test,
            }
            .matches(body, request)
        };
        assert_eq!(payload, json!({"state":"ready now"}));
        assert!(check(
            "body",
            WebhookTest::StartsWith("?state=held".into()),
            &payload,
            &request
        ));
        assert!(check(
            "body.state",
            WebhookTest::Equals("ready now".into()),
            &payload,
            &request
        ));
        request.headers.clear();
        request.raw_body =
            Some(br#"{"rows":[{"label":"a"},null],"empty":[null],"absent":null}"#.to_vec());
        let payload = request.payload().unwrap();
        for path in ["body.rows[].label", "body.rows.[].label"] {
            assert!(check(
                path,
                WebhookTest::Equals("a".into()),
                &payload,
                &request
            ));
        }
        assert!(check(
            "body.rows.length",
            WebhookTest::Equals("2".into()),
            &payload,
            &request
        ));
        assert!(!check(
            "body.rows.00.label",
            WebhookTest::Exists,
            &payload,
            &request
        ));
        assert!(!check(
            "body.absent",
            WebhookTest::Exists,
            &payload,
            &request
        ));
        assert!(!check(
            "body.empty[]",
            WebhookTest::Exists,
            &payload,
            &request
        ));
        assert!(check(
            "body.empty[]",
            WebhookTest::NotEquals("skip".into()),
            &payload,
            &request
        ));
        request.raw_body = Some(b"{unfinished".to_vec());
        assert_eq!(request.payload(), Some(json!("{unfinished")));
    }
}
