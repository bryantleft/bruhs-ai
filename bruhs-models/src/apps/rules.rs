//! The call a bruh writes and the rules it follows before it reaches an app:
//! method, path and query limits (D12), one normalized path for every check
//! (F4), refused credential endpoints (F6), and how a call is classified
//! (D5, D6, F5).

use super::{
    AppAction, AppConnector, APP_BODY_MAX_BYTES, APP_BODY_TOO_LARGE, APP_GET_BODY, APP_METHOD,
    APP_PATH, APP_PATH_ESCAPE, APP_QUERY_METHOD, APP_QUERY_TOO_LARGE, APP_QUERY_VALUES,
    MAX_PATH_BYTES, MAX_QUERY_BYTES, MAX_QUERY_ITEM_BYTES, MAX_QUERY_PARAMS,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The methods a call may use, upper case only.
pub const APP_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// One `app_request` call as the bruh wrote it. There is no host and no
/// header: the connector fixes the host and Bruhs adds the token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppCall {
    pub app: String,
    /// Picks the account by its label or entry name when the bruh may use
    /// more than one for this app.
    #[serde(default)]
    pub account: Option<String>,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: BTreeMap<String, Value>,
    #[serde(default)]
    pub body: Option<Value>,
}

/// Checks a call's method, path, query and body against D12's limits. It
/// doesn't look the app up; the gate does that next.
pub fn validate_call(call: &AppCall) -> Result<(), &'static str> {
    if !APP_METHODS.contains(&call.method.as_str()) {
        return Err(APP_METHOD);
    }
    normalize_path(&call.path)?;
    validate_query(&call.query)?;
    if let Some(body) = &call.body {
        if matches!(call.method.as_str(), "GET" | "DELETE") {
            return Err(APP_GET_BODY);
        }
        if compact_len(body) > APP_BODY_MAX_BYTES {
            return Err(APP_BODY_TOO_LARGE);
        }
    }
    Ok(())
}

fn validate_query(query: &BTreeMap<String, Value>) -> Result<(), &'static str> {
    if query.len() > MAX_QUERY_PARAMS {
        return Err(APP_QUERY_TOO_LARGE);
    }
    for (key, value) in query {
        let lower = key.to_ascii_lowercase();
        if lower == "_method" || lower.contains("method-override") {
            return Err(APP_QUERY_METHOD);
        }
        let value_fits = match value {
            Value::String(text) => text.len() <= MAX_QUERY_ITEM_BYTES,
            Value::Number(_) | Value::Bool(_) => true,
            Value::Null | Value::Array(_) | Value::Object(_) => false,
        };
        if key.len() > MAX_QUERY_ITEM_BYTES || !value_fits {
            return Err(APP_QUERY_VALUES);
        }
    }
    let total = serde_json::to_string(query).map_or(usize::MAX, |text| text.len());
    if total > MAX_QUERY_BYTES {
        return Err(APP_QUERY_TOO_LARGE);
    }
    Ok(())
}

fn compact_len(value: &Value) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |text| text.len())
}

/// Whether a path stays inside the app: see [`normalize_path`].
pub fn validate_path(path: &str) -> Result<(), &'static str> {
    normalize_path(path).map(drop)
}

/// The one path a call is classified, shown and sent on (F4). It starts with
/// `/`, is at most [`MAX_PATH_BYTES`] of printable ASCII with no space, `?`,
/// `#`, `\` or `@`, and has no empty, `.` or `..` segment. Escapes of
/// unreserved characters (letters, digits, `-`, `.`, `_`, `~`) are decoded,
/// so `%6Derge` is `merge` and `%2e%2e` is `..`; every other escape keeps its
/// place with upper-case hex, and one hiding a separator or a control
/// character is refused.
pub fn normalize_path(path: &str) -> Result<String, &'static str> {
    if path.len() > MAX_PATH_BYTES || !path.starts_with('/') {
        return Err(APP_PATH);
    }
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        at += 1;
        if byte == b'%' {
            let (Some(high), Some(low)) = (
                bytes.get(at).copied().and_then(hex_value),
                bytes.get(at + 1).copied().and_then(hex_value),
            ) else {
                return Err(APP_PATH);
            };
            at += 2;
            let decoded = high * 16 + low;
            if is_unreserved(decoded) {
                out.push(char::from(decoded));
            } else if matches!(decoded, b'/' | b'\\' | b'?' | b'#' | b'@')
                || decoded.is_ascii_control()
            {
                return Err(APP_PATH_ESCAPE);
            } else {
                out.push('%');
                out.push(char::from(HEX_UPPER[usize::from(high)]));
                out.push(char::from(HEX_UPPER[usize::from(low)]));
            }
        } else if byte.is_ascii_graphic() && !matches!(byte, b'?' | b'#' | b'\\' | b'@') {
            out.push(char::from(byte));
        } else {
            return Err(APP_PATH);
        }
    }
    let segments_ok = out
        .split('/')
        .skip(1)
        .all(|segment| !matches!(segment, "" | "." | ".."));
    if segments_ok {
        Ok(out)
    } else {
        Err(APP_PATH)
    }
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~')
}

/// Matches a rule's path pattern against a normalized path, segment by
/// segment: `*` matches any run of characters within one segment, a `**`
/// segment matches any number of segments, and literal characters compare
/// ASCII case-insensitively (F4).
pub fn path_glob(pattern: &str, path: &str) -> bool {
    let patterns: Vec<&str> = pattern.split('/').collect();
    let segments: Vec<&str> = path.split('/').collect();
    segments_match(&patterns, &segments)
}

fn segments_match(patterns: &[&str], segments: &[&str]) -> bool {
    match patterns.split_first() {
        None => segments.is_empty(),
        Some((&"**", rest)) => (0..=segments.len()).any(|skip| {
            segments
                .get(skip..)
                .is_some_and(|tail| segments_match(rest, tail))
        }),
        Some((first, rest)) => segments.split_first().is_some_and(|(segment, tail)| {
            segment_match(first.as_bytes(), segment.as_bytes()) && segments_match(rest, tail)
        }),
    }
}

/// `*` within one segment, iteratively: on a mismatch, the last `*` takes
/// one more character.
fn segment_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(literal) if text.get(t).is_some_and(|c| c.eq_ignore_ascii_case(literal)) => {
                p += 1;
                t += 1;
            }
            _ => match star {
                Some((star_p, star_t)) => {
                    p = star_p + 1;
                    t = star_t + 1;
                    star = Some((star_p, star_t + 1));
                }
                None => return false,
            },
        }
    }
    pattern
        .get(p..)
        .is_some_and(|rest| rest.iter().all(|c| *c == b'*'))
}

/// Whether the call reaches an endpoint that makes, reads or changes a
/// credential, key or webhook (F6). Such a call is refused at every level
/// with [`super::APP_CREDENTIAL_REFUSED`], before any other rule. A path
/// that doesn't normalize counts as refused.
pub fn refused_endpoint(connector: &AppConnector, method: &str, path: &str) -> bool {
    let Ok(path) = normalize_path(path) else {
        return true;
    };
    connector
        .refused
        .iter()
        .any(|rule| rule.methods.contains(&method) && path_glob(rule.path, &path))
}

/// What a call does (D5), on its normalized path: the connector's rules
/// (first match wins), then its GraphQL rule (a POST is a read for a query
/// and the rule's action otherwise; any other method is the rule's action,
/// F5), then a GET is a read when the connector says so, and anything else
/// is the connector's default write. The method must already be upper case
/// ([`validate_call`]). A path that doesn't normalize counts as delivery,
/// the strictest action, though [`validate_call`] refuses it first.
pub fn classify(
    connector: &AppConnector,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> AppAction {
    let Ok(path) = normalize_path(path) else {
        return AppAction::Delivery;
    };
    if let Some(rule) = connector
        .rules
        .iter()
        .find(|rule| rule.methods.contains(&method) && path_glob(rule.path, &path))
    {
        return rule.action;
    }
    if let Some(graphql) = connector
        .graphql
        .as_ref()
        .filter(|graphql| path_glob(graphql.path, &path))
    {
        return match (method, graphql_operation(body)) {
            ("POST", GraphqlOperation::Query) => AppAction::Read,
            _ => graphql.mutation,
        };
    }
    if method == "GET" && connector.get_is_read {
        AppAction::Read
    } else {
        connector.default_write
    }
}

/// What a GraphQL document does (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphqlOperation {
    Query,
    /// A mutation or subscription, or a document that can't be read.
    Mutation,
}

/// Reads the body's `query` string: comments and string literals are
/// dropped, then each top-level definition's first word decides. `{` or
/// `query` is a query, `fragment` is skipped, and `mutation`, `subscription`
/// or anything else (unbalanced braces, a batch array, a missing query)
/// makes the whole call a [`GraphqlOperation::Mutation`].
pub fn graphql_operation(body: Option<&Value>) -> GraphqlOperation {
    let document = body
        .and_then(Value::as_object)
        .and_then(|object| object.get("query"))
        .and_then(Value::as_str);
    match document
        .and_then(tokens)
        .map(|tokens| only_queries(&tokens))
    {
        Some(true) => GraphqlOperation::Query,
        _ => GraphqlOperation::Mutation,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token<'a> {
    Word(&'a str),
    Open(u8),
    Close(u8),
}

/// Splits a document into words and brackets, dropping whitespace, commas,
/// comments, string literals and other punctuation. `None` for a string
/// that never ends.
fn tokens(document: &str) -> Option<Vec<Token<'_>>> {
    let bytes = document.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        match byte {
            b'#' => {
                while bytes.get(at).is_some_and(|c| !matches!(c, b'\n' | b'\r')) {
                    at += 1;
                }
            }
            b'"' if bytes.get(at..at + 3) == Some(b"\"\"\"") => {
                at += 3;
                loop {
                    match bytes.get(at..at + 3)? {
                        b"\"\"\"" => break,
                        [b'\\', b'"', b'"'] if bytes.get(at + 3) == Some(&b'"') => at += 4,
                        _ => at += 1,
                    }
                }
                at += 3;
            }
            b'"' => {
                at += 1;
                loop {
                    match bytes.get(at)? {
                        b'"' => break,
                        b'\\' => at += 2,
                        b'\n' | b'\r' => return None,
                        _ => at += 1,
                    }
                }
                at += 1;
            }
            b'{' | b'(' | b'[' => {
                out.push(Token::Open(byte));
                at += 1;
            }
            b'}' | b')' | b']' => {
                out.push(Token::Close(byte));
                at += 1;
            }
            _ if byte.is_ascii_alphanumeric() || byte == b'_' => {
                let start = at;
                while bytes
                    .get(at)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
                {
                    at += 1;
                }
                out.push(Token::Word(document.get(start..at)?));
            }
            _ => at += 1,
        }
    }
    Some(out)
}

/// Whether every top-level definition is a query or a fragment.
fn only_queries(tokens: &[Token<'_>]) -> bool {
    let mut at = 0;
    let mut definitions = 0;
    while let Some(token) = tokens.get(at) {
        let next = match token {
            Token::Open(b'{') => skip_selection(tokens, at),
            Token::Word("query" | "fragment") => skip_definition(tokens, at + 1),
            _ => None,
        };
        let Some(next) = next else {
            return false;
        };
        at = next;
        definitions += 1;
    }
    definitions > 0
}

/// Skips a definition's name, variables and directives to its selection
/// set, then the set. Brackets inside parentheses (default values) nest
/// and must balance; only a `{` outside them starts the set.
fn skip_definition(tokens: &[Token<'_>], mut at: usize) -> Option<usize> {
    let mut depth = 0usize;
    loop {
        match tokens.get(at)? {
            Token::Open(b'{') if depth == 0 => return skip_selection(tokens, at),
            Token::Open(_) => depth += 1,
            Token::Close(_) => depth = depth.checked_sub(1)?,
            Token::Word(_) => {}
        }
        at += 1;
    }
}

/// From a `{`, the index after its matching `}`.
fn skip_selection(tokens: &[Token<'_>], mut at: usize) -> Option<usize> {
    let mut depth = 0usize;
    loop {
        match tokens.get(at)? {
            Token::Open(b'{') => depth += 1,
            Token::Close(b'}') => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(at + 1);
                }
            }
            _ => {}
        }
        at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::{connector, APP_CREDENTIAL_REFUSED, CONNECTORS};
    use super::*;
    use serde_json::json;

    #[test]
    fn paths_never_leave_the_app() {
        let ok = |path: &str| validate_path(path).is_ok();
        assert!(
            ok("/repos/o/r/issues")
                && ok("/v1/databases/abc%2D1/query")
                && ok("/api/chat.postMessage")
        );
        for bad in [
            "repos",
            "//evil.com/x",
            "/a/../b",
            "/a/./b",
            "/a//b",
            "/a?b=1",
            "/a#b",
            "/a\\b",
            "/a b",
            "/%2e%2e/x",
            "/a%2Fb",
            "/a%5cb",
            "/a%3fb",
            "/a%00",
            "/a%4",
            "/@evil.com",
            "/x\u{7f}",
        ] {
            assert!(!ok(bad), "{bad}");
        }
        assert!(!ok(&format!("/{}", "a".repeat(MAX_PATH_BYTES))));
    }

    #[test]
    fn paths_normalize_to_one_form() {
        assert_eq!(
            normalize_path("/v1/databases/abc%2D1/query"),
            Ok("/v1/databases/abc-1/query".into())
        );
        assert_eq!(
            normalize_path("/repos/o/r/pulls/1/%6Derge"),
            Ok("/repos/o/r/pulls/1/merge".into())
        );
        // Other escapes stay escaped, with upper-case hex.
        assert_eq!(
            normalize_path("/files/a%2cb%c3%a9"),
            Ok("/files/a%2Cb%C3%A9".into())
        );
        assert_eq!(normalize_path("/%2e%2E/x"), Err(APP_PATH));
        assert_eq!(normalize_path("/a/%2E"), Err(APP_PATH));
        assert_eq!(normalize_path("/a%2Fb"), Err(APP_PATH_ESCAPE));
        assert_eq!(normalize_path("/a%40b"), Err(APP_PATH_ESCAPE));
        assert_eq!(normalize_path("/a%0A"), Err(APP_PATH_ESCAPE));
        assert_eq!(normalize_path("/a/"), Err(APP_PATH));
        assert_eq!(normalize_path("/"), Err(APP_PATH));
        assert_eq!(normalize_path("/caf\u{e9}"), Err(APP_PATH));
    }

    #[test]
    fn calls_are_bounded_before_they_reach_an_app() {
        let call = |method: &str, body: Option<Value>| AppCall {
            app: "notion".into(),
            account: None,
            method: method.into(),
            path: "/v1/pages".into(),
            query: BTreeMap::new(),
            body,
        };
        assert!(validate_call(&call("POST", Some(json!({"a": 1})))).is_ok());
        assert_eq!(validate_call(&call("HEAD", None)), Err(APP_METHOD));
        assert_eq!(validate_call(&call("get", None)), Err(APP_METHOD));
        assert_eq!(
            validate_call(&call("GET", Some(json!({})))),
            Err(APP_GET_BODY)
        );
        assert_eq!(
            validate_call(&call("DELETE", Some(json!({})))),
            Err(APP_GET_BODY)
        );
        assert_eq!(
            validate_call(&call("POST", Some(json!("x".repeat(APP_BODY_MAX_BYTES))))),
            Err(APP_BODY_TOO_LARGE)
        );
        let mut many = call("GET", None);
        many.query = (0..51).map(|i| (format!("k{i}"), json!(1))).collect();
        assert_eq!(validate_call(&many), Err(APP_QUERY_TOO_LARGE));
        let mut nested = call("GET", None);
        nested.query.insert("a".into(), json!({"b": 1}));
        assert_eq!(validate_call(&nested), Err(APP_QUERY_VALUES));
        assert!(serde_json::from_value::<AppCall>(
            json!({"app": "notion", "method": "GET", "path": "/v1/x", "headers": {}})
        )
        .is_err());

        let mut bad_path = call("GET", None);
        bad_path.path = "x".into();
        assert_eq!(validate_call(&bad_path), Err(APP_PATH));
        let mut long_value = call("GET", None);
        long_value
            .query
            .insert("q".into(), json!("x".repeat(MAX_QUERY_ITEM_BYTES + 1)));
        assert_eq!(validate_call(&long_value), Err(APP_QUERY_VALUES));
        let mut big = call("GET", None);
        big.query = (0..5)
            .map(|i| (format!("k{i}"), json!("x".repeat(1000))))
            .collect();
        assert_eq!(validate_call(&big), Err(APP_QUERY_TOO_LARGE));
        let mut fine = call("GET", None);
        fine.query = BTreeMap::from([
            ("page_size".into(), json!(50)),
            ("archived".into(), json!(false)),
        ]);
        assert!(validate_call(&fine).is_ok());
        // The query can't change the method the gate saw (F4).
        for key in [
            "_method",
            "_METHOD",
            "X-HTTP-Method-Override",
            "method-override",
        ] {
            let mut sneaky = call("GET", None);
            sneaky.query.insert(key.into(), json!("DELETE"));
            assert_eq!(validate_call(&sneaky), Err(APP_QUERY_METHOD), "{key}");
        }
        // A model-written "as" or "host" never parses.
        for field in ["as", "host", "token"] {
            let mut input = json!({"app": "notion", "method": "GET", "path": "/v1/x"});
            input[field] = json!("x");
            assert!(serde_json::from_value::<AppCall>(input).is_err(), "{field}");
        }
    }

    fn action(app: &str, method: &str, path: &str, body: Option<Value>) -> AppAction {
        classify(connector(app).unwrap(), method, path, body.as_ref())
    }

    #[test]
    fn calls_are_classified_by_app_rules() {
        assert_eq!(
            action("github", "GET", "/repos/o/r/issues", None),
            AppAction::Read
        );
        assert_eq!(
            action(
                "github",
                "POST",
                "/repos/o/r/issues/1/comments",
                Some(json!({}))
            ),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("github", "PUT", "/repos/o/r/pulls/3/merge", None),
            AppAction::Delivery
        );
        assert_eq!(
            action("github", "POST", "/repos/o/r/pulls", Some(json!({}))),
            AppAction::Delivery
        );
        assert_eq!(
            action(
                "github",
                "PUT",
                "/repos/o/r/contents/src/a.rs",
                Some(json!({}))
            ),
            AppAction::Delivery
        );
        assert_eq!(
            action(
                "github",
                "POST",
                "/graphql",
                Some(json!({"query": "{ viewer { login } }"}))
            ),
            AppAction::Read
        );
        assert_eq!(
            action(
                "github",
                "POST",
                "/graphql",
                Some(
                    json!({"query": "mutation { mergePullRequest(input: {}) { clientMutationId } }"})
                )
            ),
            AppAction::Delivery
        );
        assert_eq!(
            action("notion", "POST", "/v1/search", Some(json!({}))),
            AppAction::Read
        );
        assert_eq!(
            action("notion", "PATCH", "/v1/pages/p", Some(json!({}))),
            AppAction::Write
        );
        assert_eq!(
            action("notion", "POST", "/v1/comments", Some(json!({}))),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("slack", "GET", "/api/conversations.history", None),
            AppAction::Read
        );
        assert_eq!(
            action("slack", "POST", "/api/users.info", Some(json!({}))),
            AppAction::Read
        );
        assert_eq!(
            action("slack", "POST", "/api/chat.postMessage", Some(json!({}))),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("slack", "GET", "/api/chat.postMessage", None),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("stripe", "POST", "/v1/refunds", Some(json!({}))),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("stripe", "POST", "/v1/customers", Some(json!({}))),
            AppAction::Write
        );
        assert_eq!(
            action(
                "linear",
                "POST",
                "/graphql",
                Some(json!({"query": "query { viewer { id } }"}))
            ),
            AppAction::Read
        );
        assert_eq!(
            action(
                "linear",
                "POST",
                "/graphql",
                Some(json!({"query": "mutation { issueCreate(input: {}) { success } }"}))
            ),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("jira", "POST", "/rest/api/3/search/jql", Some(json!({}))),
            AppAction::Read
        );
        assert_eq!(
            action(
                "cloudflare",
                "DELETE",
                "/client/v4/zones/z/dns_records/r",
                None
            ),
            AppAction::Write
        );
        assert!(
            path_glob("/repos/*/*/contents/**", "/repos/o/r/contents/a/b/c")
                && !path_glob("/repos/*/*/pulls", "/repos/o/r/pulls/1")
        );
        assert!(
            path_glob("/api/*.list", "/api/conversations.list")
                && !path_glob("/api/*.list", "/api/x/y.list")
        );
    }

    #[test]
    fn escapes_and_case_never_dodge_a_rule() {
        // F4: the rules see the path the app routes.
        assert_eq!(
            action("github", "PUT", "/repos/o/r/pulls/1/%6Derge", None),
            AppAction::Delivery
        );
        assert_eq!(
            action("github", "PUT", "/REPOS/o/r/PULLS/1/MERGE", None),
            AppAction::Delivery
        );
        assert_eq!(
            action("stripe", "POST", "/v1/re%66unds", Some(json!({}))),
            AppAction::SendAsOwner
        );
        let mutation = json!({"query": "mutation { updateRef(input: {}) { clientMutationId } }"});
        assert_eq!(
            action("github", "POST", "/graph%71l", Some(mutation)),
            AppAction::Delivery
        );
        // F5: GraphQL over another method is the rule's action, even a GET.
        assert_eq!(
            action("github", "GET", "/graphql", None),
            AppAction::Delivery
        );
        assert_eq!(
            action("linear", "GET", "/graphql", None),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action("shopify", "GET", "/admin/api/2026-10/graphql.json", None),
            AppAction::SendAsOwner
        );
        assert_eq!(
            action(
                "shopify",
                "POST",
                "/admin/api/2026-10/graphql.json",
                Some(json!({"query": "{ shop { name } }"}))
            ),
            AppAction::Read
        );
        // A path that doesn't normalize is the strictest action.
        assert_eq!(
            action("notion", "GET", "/%2e%2E/x", None),
            AppAction::Delivery
        );
        // Globs: `**` takes any number of segments, literals ignore case.
        assert!(
            path_glob("/a/**", "/a")
                && path_glob("/a/**/z", "/a/b/c/z")
                && !path_glob("/a/**/z", "/a/b/c")
        );
        assert!(
            path_glob("/api/*.LIST", "/api/users.list")
                && !path_glob("/api/*.list", "/api/users.listx")
        );
        assert!(path_glob("/*/*-*", "/x/a-b-c") && !path_glob("/x", "/x/y"));
    }

    #[test]
    fn money_and_delivery_endpoints_are_classified_so() {
        // F6 (b): access to code and repositories is delivery on GitHub.
        for (method, path) in [
            ("PUT", "/repos/o/r/collaborators/eve"),
            ("PATCH", "/repos/o/r"),
            ("DELETE", "/repos/o/r"),
            ("POST", "/repos/o/r/transfer"),
            ("POST", "/repos/o/r/dispatches"),
            ("POST", "/repos/o/r/actions/workflows/ci.yml/dispatches"),
            ("PUT", "/repos/o/r/pulls/1/update-branch"),
            ("POST", "/repos/o/r/merges"),
            ("DELETE", "/repos/o/r/contents/a.rs"),
            ("POST", "/repos/o/r/git/refs"),
            ("PATCH", "/repos/o/r/git/refs/heads/main"),
        ] {
            assert_eq!(
                action("github", method, path, None),
                AppAction::Delivery,
                "{method} {path}"
            );
        }
        assert_eq!(action("github", "GET", "/repos/o/r", None), AppAction::Read);
        // F6 (c): money and customer notices are sent as the owner.
        for (app, method, path) in [
            ("stripe", "POST", "/v1/payment_intents/pi_1/capture"),
            ("stripe", "POST", "/v1/charges/ch_1/capture"),
            ("stripe", "POST", "/v1/subscriptions"),
            ("stripe", "POST", "/v1/subscriptions/sub_1"),
            ("stripe", "DELETE", "/v1/subscriptions/sub_1"),
            ("stripe", "POST", "/v1/invoices/in_1/void"),
            ("stripe", "POST", "/v1/transfers/tr_1/reversals"),
            ("stripe", "POST", "/v1/credit_notes"),
            ("stripe", "POST", "/v1/payouts"),
            ("shopify", "POST", "/admin/api/2026-10/orders.json"),
            ("shopify", "POST", "/admin/api/2026-10/orders/1/cancel.json"),
            (
                "shopify",
                "POST",
                "/admin/api/2026-10/orders/1/transactions.json",
            ),
            (
                "shopify",
                "POST",
                "/admin/api/2026-10/customers/1/send_invite.json",
            ),
            (
                "shopify",
                "POST",
                "/admin/api/2026-10/orders/1/refunds.json",
            ),
            ("jira", "POST", "/rest/api/3/issue"),
            ("jira", "POST", "/rest/api/3/issue/X-1/comment"),
            (
                "hubspot",
                "POST",
                "/marketing/v3/transactional/single-email/send",
            ),
            ("figma", "POST", "/v1/files/f/comments"),
            ("clickup", "POST", "/api/v2/task/t/comment"),
            ("trello", "POST", "/1/cards/c/actions/comments"),
            ("todoist", "POST", "/api/v1/comments"),
        ] {
            assert_eq!(
                action(app, method, path, Some(json!({}))),
                AppAction::SendAsOwner,
                "{app} {method} {path}"
            );
        }
        assert_eq!(
            action(
                "jira",
                "POST",
                "/rest/api/3/issue/bulkfetch",
                Some(json!({}))
            ),
            AppAction::Read
        );
        assert_eq!(
            action("jira", "PUT", "/rest/api/3/issue/X-1", Some(json!({}))),
            AppAction::Write
        );
        assert_eq!(
            action(
                "hubspot",
                "POST",
                "/crm/v3/objects/contacts/search",
                Some(json!({}))
            ),
            AppAction::Read
        );
        assert_eq!(
            action(
                "airtable",
                "POST",
                "/v0/app1/tbl1/listRecords",
                Some(json!({}))
            ),
            AppAction::Read
        );
        assert_eq!(
            action("airtable", "PATCH", "/v0/app1/tbl1", Some(json!({}))),
            AppAction::Write
        );
        assert_eq!(
            action(
                "clickup",
                "POST",
                "/api/v3/workspaces/1/chat/channels/c/messages",
                Some(json!({}))
            ),
            AppAction::SendAsOwner
        );
        // Unknown endpoints fall back to the connector's default, never to a read.
        for connector in &CONNECTORS {
            for method in ["POST", "PUT", "PATCH", "DELETE"] {
                assert_ne!(
                    classify(connector, method, "/no/such/endpoint", None),
                    AppAction::Read,
                    "{} {method}",
                    connector.id
                );
            }
        }
    }

    #[test]
    fn credential_key_and_webhook_endpoints_are_refused() {
        let refused = |app: &str, method: &str, path: &str| {
            refused_endpoint(connector(app).unwrap(), method, path)
        };
        for (app, method, path) in [
            ("github", "POST", "/user/keys"),
            ("github", "POST", "/user/gpg_keys"),
            ("github", "POST", "/user/ssh_signing_keys"),
            ("github", "POST", "/repos/o/r/keys"),
            ("github", "POST", "/repos/o/r/hooks"),
            ("github", "PATCH", "/repos/o/r/hooks/1/config"),
            ("github", "POST", "/orgs/o/hooks"),
            ("github", "PATCH", "/orgs/o/hooks/1"),
            ("github", "PUT", "/repos/o/r/actions/secrets/TOKEN"),
            ("github", "PUT", "/orgs/o/actions/secrets/TOKEN"),
            (
                "github",
                "PUT",
                "/repos/o/r/environments/prod/secrets/TOKEN",
            ),
            ("github", "PUT", "/repos/o/r/dependabot/secrets/TOKEN"),
            ("github", "PUT", "/orgs/o/dependabot/secrets/TOKEN"),
            ("github", "PUT", "/user/codespaces/secrets/TOKEN"),
            (
                "github",
                "POST",
                "/repos/o/r/actions/runners/registration-token",
            ),
            (
                "github",
                "POST",
                "/orgs/o/actions/runners/registration-token",
            ),
            ("github", "POST", "/repos/o/r/actions/runners/remove-token"),
            (
                "github",
                "POST",
                "/repos/o/r/actions/runners/generate-jitconfig",
            ),
            (
                "github",
                "POST",
                "/orgs/o/actions/runners/generate-jitconfig",
            ),
            ("cloudflare", "POST", "/client/v4/user/tokens"),
            ("cloudflare", "PUT", "/client/v4/user/tokens/t/value"),
            ("cloudflare", "POST", "/client/v4/accounts/a/tokens"),
            ("cloudflare", "PUT", "/client/v4/accounts/a/tokens/t/value"),
            (
                "cloudflare",
                "POST",
                "/client/v4/accounts/a/alerting/v3/destinations/webhooks",
            ),
            (
                "shopify",
                "POST",
                "/admin/api/2026-10/storefront_access_tokens.json",
            ),
            ("shopify", "POST", "/admin/api/2026-10/webhooks.json"),
            ("shopify", "PUT", "/admin/api/2026-10/webhooks/1.json"),
            ("stripe", "POST", "/v1/webhook_endpoints"),
            ("stripe", "POST", "/v1/webhook_endpoints/we_1"),
            ("stripe", "POST", "/v1/ephemeral_keys"),
            ("stripe", "POST", "/v1/apps/secrets"),
            ("stripe", "GET", "/v1/apps/secrets/find"),
            ("slack", "POST", "/api/apps.connections.open"),
            ("slack", "GET", "/api/apps.manifest.export"),
            ("slack", "POST", "/api/oauth.v2.access"),
            ("slack", "POST", "/api/openid.connect.token"),
            ("slack", "POST", "/api/auth.revoke"),
            ("airtable", "POST", "/v0/bases/app1/webhooks"),
            ("figma", "POST", "/v2/webhooks"),
            ("figma", "PUT", "/v2/webhooks/1"),
            ("trello", "POST", "/1/webhooks"),
            ("trello", "PUT", "/1/webhooks/1"),
            ("trello", "POST", "/1/tokens/t/webhooks"),
            ("clickup", "POST", "/api/v2/team/1/webhook"),
            ("clickup", "PUT", "/api/v2/webhook/1"),
            ("jira", "POST", "/rest/api/3/webhook"),
            ("jira", "PUT", "/rest/api/3/webhook/refresh"),
            // Escapes and case don't get round it (F4).
            ("github", "POST", "/user/%6Beys"),
            ("stripe", "POST", "/V1/WEBHOOK_ENDPOINTS"),
            // Nor does a path that doesn't normalize.
            ("notion", "GET", "/a/../b"),
        ] {
            assert!(refused(app, method, path), "{app} {method} {path}");
        }
        for (app, method, path) in [
            ("github", "GET", "/user"),
            ("github", "GET", "/repos/o/r/hooks"),
            ("github", "DELETE", "/repos/o/r/hooks/1"),
            ("github", "GET", "/repos/o/r/actions/secrets"),
            ("cloudflare", "GET", "/client/v4/user/tokens/verify"),
            ("slack", "POST", "/api/chat.postMessage"),
            ("slack", "GET", "/api/auth.test"),
            ("stripe", "GET", "/v1/webhook_endpoints"),
            ("notion", "POST", "/v1/search"),
        ] {
            assert!(!refused(app, method, path), "{app} {method} {path}");
        }
        assert!(APP_CREDENTIAL_REFUSED.ends_with("Ask the owner."));
    }

    #[test]
    fn graphql_writes_count_as_writes_however_they_are_written() {
        let op = |query: &str| graphql_operation(Some(&json!({"query": query})));
        assert_eq!(op("{ viewer { id } }"), GraphqlOperation::Query);
        assert_eq!(
            op("query Q { a(s: \"mutation\") }"),
            GraphqlOperation::Query
        );
        assert_eq!(op("# mutation\nquery { a }"), GraphqlOperation::Query);
        assert_eq!(
            op("fragment F on X { a } query { ...F }"),
            GraphqlOperation::Query
        );
        assert_eq!(
            op("query A { a } mutation B { b }"),
            GraphqlOperation::Mutation
        );
        assert_eq!(op("subscription { s }"), GraphqlOperation::Mutation);
        assert_eq!(
            op("\"\"\"doc\"\"\" mutation { b }"),
            GraphqlOperation::Mutation
        );
        assert_eq!(op("query { a "), GraphqlOperation::Mutation);
        assert_eq!(op("MUTATION { b }"), GraphqlOperation::Mutation);
        assert_eq!(
            graphql_operation(Some(&json!({"query": 5}))),
            GraphqlOperation::Mutation
        );
        assert_eq!(
            graphql_operation(Some(&json!([{"query": "{ a }"}]))),
            GraphqlOperation::Mutation
        );
        assert_eq!(graphql_operation(None), GraphqlOperation::Mutation);

        // Variables, defaults, escapes and block strings never hide a mutation.
        assert_eq!(
            op("query Q($a: In = {b: \"}\"}) @x(y: [1]) { a(b: $a) }"),
            GraphqlOperation::Query
        );
        assert_eq!(
            op("query { a(s: \"\\\" } mutation { b }\") }"),
            GraphqlOperation::Query
        );
        assert_eq!(
            op("query { a(s: \"\"\"x \\\"\"\" } mutation { b }\"\"\") }"),
            GraphqlOperation::Query
        );
        assert_eq!(op("query mutation { a }"), GraphqlOperation::Query);
        assert_eq!(op("{ a } mutation{b}"), GraphqlOperation::Mutation);
        assert_eq!(op("{ a } }"), GraphqlOperation::Mutation);
        assert_eq!(op("query Q($a: In = {) { a }"), GraphqlOperation::Mutation);
        assert_eq!(
            op("query { a(s: \"never ends) }"),
            GraphqlOperation::Mutation
        );
        assert_eq!(op("type Query { a: Int }"), GraphqlOperation::Mutation);
        assert_eq!(op(""), GraphqlOperation::Mutation);
        assert_eq!(op("# only a comment"), GraphqlOperation::Mutation);
    }
}
