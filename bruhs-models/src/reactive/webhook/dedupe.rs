//! Request templates used to identify repeated deliveries and fixed-time batches.
use super::{sensitive_webhook_field, WebhookMetadata};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WebhookDedupe {
    pub replay_key: String,
    pub coalesce_key: String,
    pub coalesce_window_seconds: Option<u32>,
}

impl WebhookDedupe {
    pub fn uses_request_keys(&self) -> bool {
        !self.replay_key.is_empty() || !self.coalesce_key.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.uses_request_keys() && self.coalesce_window_seconds.is_none()
    }

    pub fn validate(&self) -> Result<(), String> {
        for (label, template) in [
            ("Duplicate request key", &self.replay_key),
            ("Coalesce key", &self.coalesce_key),
        ] {
            if !template.is_empty()
                && (template.len() > 4096 || !visit_template(template, |_, _| true))
            {
                return Err(format!(
                    "{label} must contain a valid request placeholder, such as {{{{body.id}}}}."
                ));
            }
        }
        if self
            .coalesce_window_seconds
            .is_some_and(|s| !(10..=3600).contains(&s))
        {
            return Err("Coalesce window must be whole seconds from 10 to 3600.".into());
        }
        Ok(())
    }
}

/// Stream template pieces so repeating a large body placeholder never builds a giant string.
fn visit_template(template: &str, mut visit: impl FnMut(&str, bool) -> bool) -> bool {
    let mut rest = template;
    let mut found = false;
    while let Some((literal, tail)) = rest.split_once("{{") {
        let Some((expr, next)) = tail.split_once("}}") else {
            return false;
        };
        let expr = expr.trim();
        let mut path = expr.split('.');
        let root = path.next().unwrap_or_default();
        if literal.contains("}}")
            || !matches!(root, "request" | "headers" | "query" | "body")
            || (root == "request" && expr != root)
            || path.clone().count() > 32
            || path.any(|p| {
                p.is_empty() || p.contains(['{', '}']) || p.chars().any(char::is_whitespace)
            })
            || !visit(literal, false)
            || !visit(expr, true)
        {
            return false;
        }
        found = true;
        rest = next;
    }
    found && !rest.contains("}}") && visit(rest, false)
}

pub(super) fn body_field(value: &Value, path: &[&str]) -> Option<Value> {
    let Some((part, tail)) = path.split_first() else {
        return Some(value.clone());
    };
    let key = part.strip_suffix("[]").unwrap_or(part);
    let child = if *part == "[]" {
        value // Older bruhs filters also accept a separate wildcard segment.
    } else if let Value::Array(items) = value {
        if *part == "length" {
            return tail.is_empty().then(|| Value::from(items.len()));
        }
        let index = key.parse::<usize>().ok()?;
        if index.to_string() != key {
            return None;
        }
        items.get(index)?
    } else {
        value.get(key)?
    };
    if !part.ends_with("[]") {
        return body_field(child, tail);
    }
    let mut values = Vec::new();
    for item in child.as_array()? {
        if let Some(v) = body_field(item, tail) {
            match v {
                Value::Array(array) if tail.iter().any(|p| p.ends_with("[]")) => {
                    values.extend(array)
                }
                v => values.push(v),
            }
        }
    }
    Some(Value::Array(values))
}

pub(super) fn request_value(
    expression: &str,
    payload: &Value,
    request: &WebhookMetadata,
) -> Option<String> {
    let (root, path) = expression.split_once('.').unwrap_or((expression, ""));
    let raw = || {
        request
            .body_text()
            .map_or_else(|| payload.to_string(), |text| text.into_owned())
    };
    let json = match root {
        "body" if expression == root => return Some(raw()),
        "body" => body_field(payload, &path.split('.').collect::<Vec<_>>())?,
        "headers" if expression != root => {
            return Some(request.headers.get(&path.to_ascii_lowercase())?.join(", "))
        }
        "query" if expression != root => return request.query.get(path)?.first().cloned(),
        "headers" | "query" => {
            let headers = root == "headers";
            let fields = if headers {
                &request.headers
            } else {
                &request.query
            };
            Value::Object(
                fields
                    .iter()
                    .filter_map(|(k, v)| {
                        let val = if sensitive_webhook_field(k) {
                            "[redacted]".into()
                        } else if headers {
                            v.join(", ")
                        } else {
                            v.last()?.clone()
                        };
                        Some((k.clone(), Value::String(val)))
                    })
                    .collect(),
            )
        }
        "request" if expression == "request" => {
            let uri = request.uri.as_deref().unwrap_or("/");
            let mut text = String::from("POST ");
            if let Some((path, query)) = uri.split_once('?') {
                text.push_str(path);
                text.push('?');
                for (i, pair) in query.split('&').enumerate() {
                    if i != 0 {
                        text.push('&');
                    }
                    if let Some((name, _)) = pair.split_once('=') {
                        let name_to_decode = name.replace('+', " ");
                        let decoded = percent_encoding::percent_decode_str(&name_to_decode)
                            .decode_utf8_lossy();
                        if sensitive_webhook_field(&decoded) {
                            text.push_str(name);
                            text.push_str("=[redacted]");
                            continue;
                        }
                    }
                    text.push_str(pair);
                }
            } else {
                text.push_str(uri);
            }
            text.push('\n');
            for (name, values) in &request.headers {
                let value = if sensitive_webhook_field(name) {
                    "[redacted]".into()
                } else {
                    values.join(", ")
                };
                text.push_str(&format!("{name}: {value}\n"));
            }
            text.push('\n');
            text.push_str(&raw());
            return Some(text);
        }
        _ => return None,
    };
    match json {
        Value::Null => None,
        Value::String(s) => Some(s),
        value => serde_json::to_string_pretty(&value).ok(),
    }
}

pub fn webhook_key_hash(
    template: &str,
    payload: &Value,
    request: &WebhookMetadata,
) -> Option<Vec<u8>> {
    let mut digest = Sha256::new();
    let valid = visit_template(template, |text, placeholder| {
        if placeholder {
            let Some(value) = request_value(text, payload, request).filter(|v| !v.is_empty())
            else {
                return false;
            };
            digest.update(value.as_bytes());
        } else {
            digest.update(text.as_bytes());
        }
        true
    });
    valid.then(|| digest.finalize().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_templates_resolve_each_placeholder_and_skip_incomplete_keys() {
        let request = WebhookMetadata {
            headers: [("delivery-id".into(), vec!["abc".into()])].into(),
            query: [("lane".into(), vec!["alpha".into(), "beta".into()])].into(),
            raw_body: Some("{ \"id\": 1 }".into()),
            ..Default::default()
        };
        let body = json!({"id":1,"items":[{"id":2},{},{"id":3}],"empty":""});
        let key = |t| webhook_key_hash(t, &body, &request);
        let hash = |s: &str| Some(Sha256::digest(s.as_bytes()).to_vec());
        assert_eq!(
            key("{{headers.Delivery-ID}}/{{ query.lane }}/{{body.id}}"),
            hash("abc/alpha/1")
        );
        assert_eq!(key("{{body.items[].id}}"), hash("[\n  2,\n  3\n]"));
        assert_eq!(key("{{body}}"), hash("{ \"id\": 1 }"));
        for template in [
            "fixed",
            "{{body.absent}}",
            "prefix{{body.empty}}",
            "{{request.part}}",
            "{{body.id",
            "{{unknown}}",
        ] {
            assert_eq!(key(template), None, "{template}");
        }
        assert!(WebhookDedupe {
            replay_key: "constant".into(),
            ..Default::default()
        }
        .validate()
        .is_err());
        for n in [9, 10, 120, 3600, 3601] {
            assert_eq!(
                WebhookDedupe {
                    coalesce_window_seconds: Some(n),
                    ..Default::default()
                }
                .validate()
                .is_ok(),
                (10..=3600).contains(&n)
            );
        }
    }
}
