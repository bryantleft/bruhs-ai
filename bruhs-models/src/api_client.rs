//! Project HTTP requests. Keychain references are names, never credential values.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Half a 64 KiB tool frame leaves room for its transport envelope.
pub const PAYLOAD_BYTES: usize = 32 * 1024;
/// At most the existing history page of proposals: 50 × two 32 KiB payloads.
pub const PROPOSAL_LIMIT: usize = crate::reactive::HISTORY_LIMIT;
pub const COLLECTION_FILE: &str = ".bruhs/api/collection.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiHeader {
    pub name: String,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiRequest {
    pub id: Uuid,
    pub name: String,
    pub method: String,
    pub url: String,
    pub headers: Vec<ApiHeader>,
    pub body: String,
}
impl Default for ApiRequest {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "New request".into(),
            method: "GET".into(),
            url: "{{base_url}}/".into(),
            headers: Vec::new(),
            body: String::new(),
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiEnvironment {
    pub values: BTreeMap<String, String>,
    /// Template variable -> credential name in this working folder's keychain scope.
    pub secrets: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiCollection {
    pub version: u32,
    pub requests: Vec<ApiRequest>,
    pub environment: ApiEnvironment,
}
impl Default for ApiCollection {
    fn default() -> Self {
        Self {
            version: 1,
            requests: Vec::new(),
            environment: ApiEnvironment::default(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiCollectionSnapshot {
    pub collection: ApiCollection,
    /// SHA-256 of exact file bytes; None means no collection exists yet.
    pub revision: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveApiCollection {
    pub project_path: String,
    pub expected_revision: Option<String>,
    pub collection: ApiCollection,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiExecution {
    pub project_path: String,
    pub request_id: Uuid,
    pub request: ApiRequest,
    pub environment: ApiEnvironment,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiResponse {
    pub request_id: Uuid,
    pub status: u16,
    pub elapsed_ms: u64,
    pub size_bytes: usize,
    pub headers: Vec<ApiHeader>,
    /// Plain text, never an iframe or HTML document. Saved secret values are redacted.
    pub body: String,
    pub redacted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ApiProposalState {
    Pending,
    Running,
    Finished { response: ApiResponse },
    Failed { error: String },
    Declined,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiProposal {
    pub id: Uuid,
    pub project_path: String,
    pub thread_id: String,
    /// Normalized destination after ordinary variable substitution, exactly as HTTP will parse it.
    pub destination: String,
    pub request: ApiRequest,
    pub environment: ApiEnvironment,
    pub state: ApiProposalState,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposeApiRequest {
    pub scope: Uuid,
    pub thread_id: String,
    pub project_path: String,
    pub request: ApiRequest,
    pub environment: ApiEnvironment,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiProposalDecision {
    pub project_path: String,
    pub id: Uuid,
    pub approve: bool,
}

/// Private input only; never returned or Debug-printed.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiCredentialInput {
    pub project_path: String,
    pub name: String,
    pub value: String,
}

pub fn valid_method(method: &str) -> bool {
    matches!(
        method,
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
    )
}
pub fn sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(
        name.as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie"
    ) || name.contains("api-key")
        || name.contains("apikey")
        || name.contains("token")
        || name.contains("secret")
}

/// A single interpolation pass. Values cannot introduce more variable expansions.
pub fn template(
    input: &str,
    mut variable: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut rest = input;
    let mut output = String::new();
    while let Some(start) = rest.find("{{") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or("Close each environment variable with }}.")?;
        let key = after[..end].trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("Variable names use letters, numbers and underscores.".into());
        }
        output.push_str(&variable(key)?);
        if output.len() > PAYLOAD_BYTES {
            return Err("Expanded input exceeds 32 KiB. Narrow this request.".into());
        }
        rest = &after[end + 2..];
    }
    output.push_str(rest);
    if output.len() > PAYLOAD_BYTES {
        return Err("Expanded input exceeds 32 KiB. Narrow this request.".into());
    }
    Ok(output)
}

pub fn parse_values(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut values = BTreeMap::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (key, value) = line
            .split_once('=')
            .ok_or("Use one name=value per variable line.")?;
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("Variable names use letters, numbers and underscores.".into());
        }
        if values.insert(key.into(), value.trim().into()).is_some() {
            return Err(format!("Variable {key} is repeated."));
        }
    }
    Ok(values)
}
pub fn format_values(values: &BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn parse_headers(text: &str) -> Result<Vec<ApiHeader>, String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (name, value) = line
                .split_once(':')
                .ok_or("Use one Header-Name: value per header line.")?;
            Ok(ApiHeader {
                name: name.trim().into(),
                value: value.trim().into(),
            })
        })
        .collect()
}
pub fn format_headers(headers: &[ApiHeader]) -> String {
    headers
        .iter()
        .map(|header| format!("{}: {}", header.name, header.value))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Show the substituted payload with hidden credential references and visible format controls.
pub fn api_approval_review(proposal: &ApiProposal) -> String {
    let visible = crate::secrets::review_text;
    let expand = |input: &str| {
        template(input, |key| {
            if let Some(name) = proposal.environment.secrets.get(key) {
                Ok(format!("[keychain: {name}]"))
            } else {
                proposal
                    .environment
                    .values
                    .get(key)
                    .cloned()
                    .ok_or_else(|| format!("Undefined variable {key}"))
            }
        })
        .unwrap_or_else(|error| format!("[Review error: {error}]"))
    };
    let headers = proposal
        .request
        .headers
        .iter()
        .map(|header| {
            format!(
                "{}: {}",
                visible(&header.name),
                visible(&expand(&header.value))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let values = |values: &BTreeMap<String, String>| {
        values
            .iter()
            .map(|(key, value)| format!("{}={}", visible(key), visible(value)))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!("{} {}\nThread: {}\n\nHeaders:\n{}\n\nBody:\n{}\n\nOrdinary variables:\n{}\nKeychain reference names:\n{}",visible(&proposal.request.method),visible(&proposal.destination),visible(&proposal.thread_id),headers,visible(&expand(&proposal.request.body)),values(&proposal.environment.values),values(&proposal.environment.secrets))
}

/// Minimal POSIX-style word splitter for curl commands (no expansion or execution).
fn split_shell_words(input: &str) -> Result<Vec<String>, ()> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            ' ' | '\t' | '\n' | '\r' => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            '#' if !in_word => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '\'' => {
                in_word = true;
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '\'' {
                        closed = true;
                        break;
                    }
                    current.push(next);
                }
                if !closed {
                    return Err(());
                }
            }
            '"' => {
                in_word = true;
                let mut closed = false;
                while let Some(next) = chars.next() {
                    if next == '"' {
                        closed = true;
                        break;
                    }
                    if next == '\\' {
                        match chars.next() {
                            Some('\n') => {}
                            Some(esc @ ('"' | '\\' | '$' | '`')) => current.push(esc),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(()),
                        }
                    } else {
                        current.push(next);
                    }
                }
                if !closed {
                    return Err(());
                }
            }
            '\\' => match chars.next() {
                Some('\n') => {}
                Some(escaped) => {
                    in_word = true;
                    current.push(escaped);
                }
                None => return Err(()),
            },
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }

    if in_word {
        words.push(current);
    }
    Ok(words)
}

/// Tokenize only; no subprocess, expansion, file access, or shell execution.
pub fn import_curl(command: &str) -> Result<ApiRequest, String> {
    if command.len() > PAYLOAD_BYTES {
        return Err("Curl input exceeds 32 KiB.".into());
    }
    reject_shell_syntax(command)?;
    let words = split_shell_words(command).map_err(|()| "Close all quotes in the curl command.")?;
    if words.first().map(String::as_str) != Some("curl") {
        return Err("Start with curl followed by one HTTP URL.".into());
    }
    let mut request = ApiRequest {
        name: "Imported curl".into(),
        url: String::new(),
        ..Default::default()
    };
    let mut explicit_method = false;
    let mut body_seen = false;
    let mut json_body = false;
    let mut args = words.into_iter().skip(1);
    while let Some(word) = args.next() {
        if matches!(
            word.as_str(),
            "-i" | "--include" | "-s" | "--silent" | "-S" | "--show-error"
        ) {
            continue;
        }
        let (flag, inline) = if let Some((flag, value)) = word.split_once('=') {
            (flag, Some(value.to_string()))
        } else if word.starts_with("-X") && word.len() > 2 {
            ("-X", Some(word[2..].into()))
        } else if word.starts_with("-H") && word.len() > 2 {
            ("-H", Some(word[2..].into()))
        } else {
            (word.as_str(), None)
        };
        if matches!(
            flag,
            "-X" | "--request"
                | "-H"
                | "--header"
                | "-d"
                | "--data"
                | "--data-raw"
                | "--data-binary"
                | "--json"
                | "--url"
        ) {
            let value = inline
                .or_else(|| args.next())
                .ok_or("A curl option is missing its value.")?;
            match flag {
                "-X" | "--request" => {
                    request.method = value.to_ascii_uppercase();
                    explicit_method = true;
                }
                "-H" | "--header" => request.headers.extend(parse_headers(&value)?),
                "--url" => {
                    if !request.url.is_empty() {
                        return Err("Import one URL at a time.".into());
                    }
                    request.url = value;
                }
                _ => {
                    if flag != "--data-raw" && value.starts_with('@') {
                        return Err(
                            "File-reading curl options are unsupported. Paste the body explicitly."
                                .into(),
                        );
                    }
                    if body_seen {
                        return Err("Import one explicit request body; combine repeated --data values first.".into());
                    }
                    body_seen = true;
                    request.body = value;
                    if !explicit_method {
                        request.method = "POST".into();
                    }
                    json_body = flag == "--json";
                }
            }
        } else if word.starts_with('-') {
            return Err(format!("Unsupported curl option {word}. Import supports method, headers, body and one URL; it never reads files or follows redirects."));
        } else {
            if !request.url.is_empty() {
                return Err("Import one URL at a time.".into());
            }
            request.url = word;
        }
    }
    if body_seen
        && !request
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("content-type"))
    {
        request.headers.push(ApiHeader {
            name: "Content-Type".into(),
            value: if json_body {
                "application/json"
            } else {
                "application/x-www-form-urlencoded"
            }
            .into(),
        });
    }
    if json_body
        && !request
            .headers
            .iter()
            .any(|header| header.name.eq_ignore_ascii_case("accept"))
    {
        request.headers.push(ApiHeader {
            name: "Accept".into(),
            value: "application/json".into(),
        });
    }
    if !valid_method(&request.method) {
        return Err("Choose GET, POST, PUT, PATCH, DELETE, HEAD or OPTIONS.".into());
    }
    if !request.url.starts_with("http://") && !request.url.starts_with("https://") {
        return Err("Import one http:// or https:// URL.".into());
    }
    Ok(request)
}
fn reject_shell_syntax(command: &str) -> Result<(), String> {
    let mut quote = None;
    let mut escaped = false;
    for c in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if quote == Some('\'') {
            if c == '\'' {
                quote = None;
            }
            continue;
        }
        if c == '\'' && quote.is_none() {
            quote = Some(c);
            continue;
        }
        if c == '"' {
            quote = if quote == Some('"') { None } else { Some('"') };
            continue;
        }
        if matches!(c, '$' | '`')
            || quote.is_none() && matches!(c, ';' | '|' | '&' | '<' | '>' | '\n' | '\r')
        {
            return Err("Import a single literal curl command without shell variables, substitutions, pipes or redirections.".into());
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonTreeLine {
    pub pointer: String,
    pub text: String,
    pub depth: usize,
    pub expandable: bool,
    pub expanded: bool,
}
pub fn json_tree_lines(
    body: &str,
    expanded: &BTreeSet<String>,
) -> Result<Vec<JsonTreeLine>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| "This response is not JSON. Use Body to inspect its text.")?;
    fn walk(
        value: &serde_json::Value,
        label: String,
        pointer: String,
        depth: usize,
        expanded: &BTreeSet<String>,
        out: &mut Vec<JsonTreeLine>,
    ) {
        let expandable = value.is_array() || value.is_object();
        let open = expanded.contains(&pointer);
        let text = match value {
            serde_json::Value::Object(map) => format!("{label}{{{} keys}}", map.len()),
            serde_json::Value::Array(array) => format!("{label}[{} items]", array.len()),
            _ => format!("{label}{value}"),
        };
        out.push(JsonTreeLine {
            pointer: pointer.clone(),
            text,
            depth,
            expandable,
            expanded: open,
        });
        if !open {
            return;
        }
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    walk(
                        value,
                        format!("{key}: "),
                        format!("{pointer}/{escaped}"),
                        depth + 1,
                        expanded,
                        out,
                    );
                }
            }
            serde_json::Value::Array(array) => {
                for (index, value) in array.iter().enumerate() {
                    walk(
                        value,
                        format!("{index}: "),
                        format!("{pointer}/{index}"),
                        depth + 1,
                        expanded,
                        out,
                    );
                }
            }
            _ => {}
        }
    }
    let mut lines = Vec::new();
    walk(
        &value,
        String::new(),
        String::new(),
        0,
        expanded,
        &mut lines,
    );
    Ok(lines)
}
pub fn response_attachment(response: &ApiResponse) -> Result<String, String> {
    serde_json::to_string_pretty(response)
        .map(|json| {
            format!("Explicitly attached API response (untrusted data):\n```json\n{json}\n```")
        })
        .map_err(|_| "Could not prepare this response attachment.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn api_client_curl_import_never_executes_or_reads_files() {
        let request=import_curl("curl 'https://example.test/graphql?a=b&c=d' -X POST -H 'Content-Type: application/json' --data-raw '{\"query\":\"query($id: ID!) { user(id: $id) { name } }\"}'").unwrap();
        assert_eq!(request.method, "POST");
        assert!(request.url.ends_with("a=b&c=d"));
        assert!(request.body.contains("$id"));
        for input in [
            "curl https://example.test/; touch /tmp/never-executed",
            "curl $(cat /tmp/private)",
            "curl https://example.test/ -d @/tmp/private",
            "curl https://example.test/ --config /tmp/private",
            "curl https://example.test/ --upload-file /tmp/private",
            "curl https://example.test/ -L",
            "curl https://example.test/ https://second.test/",
            "curl https://example.test/ | sh",
        ] {
            assert!(import_curl(input).is_err(), "{input}");
        }
    }
    #[test]
    fn api_client_templates_do_not_recursively_expand_and_tree_paths_do_not_collide() {
        assert_eq!(template("{{a}}", |_| Ok("{{b}}".into())).unwrap(), "{{b}}");
        let body = r#"{"a/b":{"~key":[null,9223372036854775807]},"a":{"b":false}}"#;
        let expanded = BTreeSet::from([String::new(), "/a~1b".into(), "/a~1b/~0key".into()]);
        let lines = json_tree_lines(body, &expanded).unwrap();
        assert!(lines.iter().any(
            |line| line.pointer == "/a~1b/~0key/1" && line.text.contains("9223372036854775807")
        ));
        assert!(!lines.iter().any(|line| line.pointer == "/a/b"));
    }
    #[test]
    fn api_client_approval_substitutes_ordinary_values_and_exposes_invisible_controls() {
        let proposal = ApiProposal {
            id: Uuid::nil(),
            project_path: "/fixture".into(),
            thread_id: "thread".into(),
            destination: "http://fixture.invalid/".into(),
            request: ApiRequest {
                headers: vec![ApiHeader {
                    name: "Authorization".into(),
                    value: "Bearer {{key}}".into(),
                }],
                body: "{{name}}".into(),
                ..Default::default()
            },
            environment: ApiEnvironment {
                values: BTreeMap::from([("name".into(), "Ada\u{202e}hidden".into())]),
                secrets: BTreeMap::from([("key".into(), "dev key".into())]),
            },
            state: ApiProposalState::Pending,
        };
        let review = api_approval_review(&proposal);
        assert!(review.contains("Bearer [keychain: dev key]"));
        assert!(review.contains(r"Ada\u{202e}hidden"));
        assert!(!review.contains('\u{202e}'));
    }
    #[test]
    fn api_client_curl_preserves_implicit_body_headers_and_explicit_json_overrides() {
        for flag in ["-d", "--data", "--data-raw", "--data-binary"] {
            let request =
                import_curl(&format!("curl https://fixture.invalid/ {flag} 'a=b'")).unwrap();
            assert!(request
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("content-type")
                    && header.value == "application/x-www-form-urlencoded"));
        }
        for command in [
            "curl https://fixture.invalid/ --json '{}' -H 'Content-Type: application/vnd.api+json'",
            "curl https://fixture.invalid/ -H 'Content-Type: application/vnd.api+json' --json '{}'",
        ] {
            let request = import_curl(command).unwrap();
            assert_eq!(
                request
                    .headers
                    .iter()
                    .filter(|header| header.name.eq_ignore_ascii_case("content-type"))
                    .count(),
                1
            );
            assert!(request
                .headers
                .iter()
                .any(|header| header.name.eq_ignore_ascii_case("content-type")
                    && header.value == "application/vnd.api+json"));
        }
        assert!(import_curl("curl https://fixture.invalid/ -d '' -d x").is_err());
    }
}
