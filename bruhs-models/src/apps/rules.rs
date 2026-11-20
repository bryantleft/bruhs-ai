//! The call a bruh writes and the rules it follows before it reaches an app:
//! method, path, query and account limits (D12), one normalized path for
//! every check (F4), and the one entry point, [`classify`], that refuses any
//! endpoint the app's allowlist doesn't name and returns the class of the
//! rest (D5, D6, ruling A).

use super::{
    AppAction, AppConnector, APP_ACCOUNT_SHAPE, APP_BODY_MAX_BYTES, APP_BODY_TOO_LARGE,
    APP_CREDENTIAL_REFUSED, APP_FIELD_REFUSED, APP_GET_BODY, APP_GRAPHQL_SHAPE, APP_METHOD,
    APP_PATH, APP_PATH_ESCAPE, APP_QUERY_METHOD, APP_QUERY_ONLY, APP_QUERY_TOO_LARGE,
    APP_QUERY_VALUES, MAX_ACCOUNT_CHARS, MAX_PATH_BYTES, MAX_QUERY_BYTES, MAX_QUERY_ITEM_BYTES,
    MAX_QUERY_PARAMS,
};
use crate::secrets::review_text;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The methods a call may use. A bruh writes them upper case; [`classify`]
/// compares them in any case, so a lower-case one can't slip past a rule.
pub const APP_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// One `app_request` call as the bruh wrote it. There is no host and no
/// header: the connector fixes the host and Bruhs adds the token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppCall {
    pub app: String,
    /// Picks the account by its label or entry name when the bruh may use
    /// more than one for this app. At most [`MAX_ACCOUNT_CHARS`].
    #[serde(default)]
    pub account: Option<String>,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub query: BTreeMap<String, Value>,
    #[serde(default)]
    pub body: Option<Value>,
}

/// Checks a call's method, path, query, body and account against D12's
/// limits. It doesn't look the app up; the gate does that next, then
/// [`classify`].
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
        // Stripe reads form fields, never JSON: its calls go by query only.
        if call.app == "stripe" {
            return Err(APP_QUERY_ONLY);
        }
    }
    if let Some(account) = &call.account {
        let chars = account.chars().count();
        // `review_text` escapes control and bidi or other format characters,
        // so a name it changes would show differently on the card.
        if chars == 0 || chars > MAX_ACCOUNT_CHARS || review_text(account) != *account {
            return Err(APP_ACCOUNT_SHAPE);
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
/// `/`, is at most [`MAX_PATH_BYTES`] of printable ASCII with no space or
/// any of `?#\@;"{}<>|^` and backtick, and has no empty, `.` or `..`
/// segment. Escapes of unreserved characters (letters, digits, `-`, `.`,
/// `_`, `~`) are decoded, so `%6Derge` is `merge` and `%2e%2e` is `..`;
/// every other escape keeps its place with upper-case hex. An escape hiding
/// a separator, `;` (matrix parameters), `%` (a second decoding) or a control
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
            } else if matches!(decoded, b'/' | b'\\' | b'?' | b'#' | b'@' | b';' | b'%')
                || decoded.is_ascii_control()
            {
                return Err(APP_PATH_ESCAPE);
            } else {
                out.push('%');
                out.push(char::from(HEX_UPPER[usize::from(high)]));
                out.push(char::from(HEX_UPPER[usize::from(low)]));
            }
        } else if byte.is_ascii_graphic() && !RAW_REFUSED.contains(&byte) {
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

/// Printable bytes a path may not hold raw: the URL's own delimiters, `;`
/// (servers strip matrix parameters before routing), and the characters a
/// URL builder would escape, so the path shown is the path sent.
const RAW_REFUSED: &[u8] = b"?#\\@;\"{}<>|^`";

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

/// Matches an endpoint's path pattern against a normalized path, segment by
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

/// The one entry point for what a call may do (ruling A). On the call's
/// normalized path and its method in any case:
///
/// - The connector's GraphQL path takes only a POST with no URL query and a
///   body of [`GRAPHQL_BODY_KEYS`] whose document can be read
///   (`APP_GRAPHQL_SHAPE` otherwise). A word the rule refuses is
///   `APP_CREDENTIAL_REFUSED`. A query operation is a read; a mutation,
///   subscription or anything else is the rule's `mutation`.
/// - Otherwise the first allowlisted endpoint that matches gives the class.
/// - Anything else is refused with the connector's `unlisted` copy, so an
///   alias, a credential endpoint or a new money action is never sent until
///   the catalog names it.
pub fn classify(connector: &AppConnector, call: &AppCall) -> Result<AppAction, &'static str> {
    let method = call.method.to_ascii_uppercase();
    let path = normalize_path(&call.path)?;
    if let Some(graphql) = connector
        .graphql
        .as_ref()
        .filter(|graphql| path_glob(graphql.path, &path))
    {
        if method != "POST" || !call.query.is_empty() {
            return Err(APP_GRAPHQL_SHAPE);
        }
        let tokens = graphql_document(call.body.as_ref())
            .and_then(tokens)
            .ok_or(APP_GRAPHQL_SHAPE)?;
        let refused = tokens.iter().any(|token| match token {
            Token::Word(word) => graphql
                .refused_names
                .iter()
                .any(|name| segment_match(name.as_bytes(), word.as_bytes())),
            Token::Open(_) | Token::Close(_) => false,
        });
        if refused {
            return Err(APP_CREDENTIAL_REFUSED);
        }
        return if only_queries(&tokens) {
            Ok(AppAction::Read)
        } else {
            graphql.mutation
        };
    }
    connector
        .endpoints
        .iter()
        .find(|endpoint| {
            endpoint.methods.contains(&method.as_str()) && path_glob(endpoint.path, &path)
        })
        .ok_or(connector.unlisted)
        .and_then(|endpoint| {
            if sends_refused_key(call, endpoint.refused_keys) {
                Err(APP_FIELD_REFUSED)
            } else {
                Ok(endpoint.action)
            }
        })
}

/// Whether the query or the body's top-level object names one of `keys`,
/// in any case, alone or as `key[…]` (form-style nesting).
fn sends_refused_key(call: &AppCall, keys: &[&str]) -> bool {
    let body_keys = call
        .body
        .as_ref()
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|object| object.keys());
    call.query.keys().chain(body_keys).any(|sent| {
        let sent = sent.to_ascii_lowercase();
        keys.iter().any(|key| {
            sent == *key
                || sent
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.starts_with('['))
        })
    })
}

/// The only keys a GraphQL body may hold. Anything else (`doc_id`,
/// `extensions.persistedQuery`, …) could make the app run a stored document
/// the classifier never read (final review A1 M2).
pub const GRAPHQL_BODY_KEYS: [&str; 3] = ["query", "variables", "operationName"];

/// The document of a GraphQL body that holds only [`GRAPHQL_BODY_KEYS`] and
/// a string `query`.
fn graphql_document(body: Option<&Value>) -> Option<&str> {
    let object = body?.as_object()?;
    if !object
        .keys()
        .all(|key| GRAPHQL_BODY_KEYS.contains(&key.as_str()))
    {
        return None;
    }
    object.get("query")?.as_str()
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
    use super::super::{connector, APP_GITHUB_GRAPHQL_WRITES, CONNECTORS};
    use super::*;
    use serde_json::json;

    fn call(app: &str, method: &str, path: &str, body: Option<Value>) -> AppCall {
        AppCall {
            app: app.into(),
            account: None,
            method: method.into(),
            path: path.into(),
            query: BTreeMap::new(),
            body,
        }
    }

    fn outcome(
        app: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<AppAction, &'static str> {
        classify(connector(app).unwrap(), &call(app, method, path, body))
    }

    /// The class of an allowlisted call; panics with the refusal otherwise.
    fn action(app: &str, method: &str, path: &str, body: Option<Value>) -> AppAction {
        outcome(app, method, path, body)
            .unwrap_or_else(|refusal| panic!("{app} {method} {path}: {refusal}"))
    }

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
        // Ruling A, I1, M2, M6: matrix parameters, a second decoding and
        // characters a URL builder would escape are refused, raw or escaped.
        for raw in [
            "/a;x/b", "/a/b;", "/a{b}", "/a<b>", "/a|b", "/a^b", "/a\"b", "/a`b",
        ] {
            assert_eq!(normalize_path(raw), Err(APP_PATH), "{raw}");
        }
        for escaped in [
            "/a%3Bx/b",
            "/a%3bx",
            "/repos/o/r/pulls/1/%256Derge",
            "/a%25",
        ] {
            assert_eq!(normalize_path(escaped), Err(APP_PATH_ESCAPE), "{escaped}");
        }
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
        // Stripe reads form fields, so a JSON body would be dropped.
        let mut stripe = call("POST", Some(json!({"email": "a@b.test"})));
        stripe.app = "stripe".into();
        stripe.path = "/v1/customers".into();
        assert_eq!(validate_call(&stripe), Err(APP_QUERY_ONLY));
        stripe.body = None;
        stripe.query.insert("email".into(), json!("a@b.test"));
        assert!(validate_call(&stripe).is_ok());
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
        // M7: the account a call names is 1 to 64 characters.
        for (account, verdict) in [
            ("ws", Ok(())),
            (&*"é".repeat(MAX_ACCOUNT_CHARS), Ok(())),
            (&*"a".repeat(MAX_ACCOUNT_CHARS + 1), Err(APP_ACCOUNT_SHAPE)),
            ("", Err(APP_ACCOUNT_SHAPE)),
            // Fix round 2: control and bidi characters would show differently
            // on the card than the name the lookup matches.
            ("me\nyou", Err(APP_ACCOUNT_SHAPE)),
            ("me\u{7}", Err(APP_ACCOUNT_SHAPE)),
            ("\u{202e}em", Err(APP_ACCOUNT_SHAPE)),
            ("me\u{2066}x\u{2069}", Err(APP_ACCOUNT_SHAPE)),
            ("me\u{200b}", Err(APP_ACCOUNT_SHAPE)),
            ("Acme Café", Ok(())),
        ] {
            let mut named = call("GET", None);
            named.account = Some(account.to_string());
            assert_eq!(validate_call(&named), verdict, "{account}");
        }
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
        // Final review A1 I1: GitHub writes go by REST only.
        assert_eq!(
            outcome(
                "github",
                "POST",
                "/graphql",
                Some(
                    json!({"query": "mutation { mergePullRequest(input: {}) { clientMutationId } }"})
                )
            ),
            Err(APP_GITHUB_GRAPHQL_WRITES)
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
            AppAction::SendAsOwner
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
        // F4: the allowlist sees the path the app routes.
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
            outcome("github", "POST", "/graph%71l", Some(mutation)),
            Err(APP_GITHUB_GRAPHQL_WRITES)
        );
        // I2: methods match in any case, so a lower-case one can't slip by.
        assert_eq!(
            action("github", "put", "/repos/o/r/pulls/1/merge", None),
            AppAction::Delivery
        );
        assert_eq!(
            action("slack", "get", "/api/chat.postMessage", None),
            AppAction::SendAsOwner
        );
        assert!(outcome("github", "post", "/user/keys", Some(json!({}))).is_err());
        assert_eq!(
            outcome("github", "HEAD", "/user", None),
            Err(connector("github").unwrap().unlisted)
        );
        // A path that doesn't normalize is refused with its reason.
        assert_eq!(outcome("notion", "GET", "/%2e%2E/x", None), Err(APP_PATH));
        assert_eq!(
            outcome(
                "jira",
                "POST",
                "/rest/api/3/issue;x/X-1/comment",
                Some(json!({}))
            ),
            Err(APP_PATH)
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
    fn graphql_takes_its_document_only_from_a_post_body() {
        let with_query = |app: &str, path: &str, key: &str, value: Value| {
            let mut call = call(
                app,
                "POST",
                path,
                Some(json!({"query": "{ viewer { login } }"})),
            );
            call.query.insert(key.into(), value);
            classify(connector(app).unwrap(), &call)
        };
        // C2: a mutation, operation name or variables in the URL query never ride along.
        for (app, path) in [
            ("github", "/graphql"),
            ("linear", "/graphql"),
            ("shopify", "/admin/api/2026-10/graphql.json"),
        ] {
            assert_eq!(
                with_query(app, path, "query", json!("mutation { a }")),
                Err(APP_GRAPHQL_SHAPE),
                "{app}"
            );
            assert_eq!(
                with_query(app, path, "operationName", json!("B")),
                Err(APP_GRAPHQL_SHAPE),
                "{app}"
            );
            assert_eq!(
                with_query(app, path, "variables", json!("{}")),
                Err(APP_GRAPHQL_SHAPE),
                "{app}"
            );
            // F5, now refused outright: no other method reaches GraphQL.
            for method in ["GET", "PUT", "PATCH", "DELETE"] {
                assert_eq!(
                    outcome(app, method, path, None),
                    Err(APP_GRAPHQL_SHAPE),
                    "{app} {method}"
                );
            }
            // No body, or one that isn't readable, is never a read.
            assert_eq!(
                outcome(app, "POST", path, None),
                Err(APP_GRAPHQL_SHAPE),
                "{app}"
            );
            assert_ne!(
                outcome(app, "POST", path, Some(json!({"query": "{ a "}))),
                Ok(AppAction::Read),
                "{app}"
            );
        }
        // I5: Shopify's unversioned GraphQL path is GraphQL too.
        let refund = json!({"query": "mutation { refundCreate(input: {}) { refund { id } } }"});
        for path in [
            "/admin/api/graphql.json",
            "/admin/graphql.json",
            "/admin/api/2025-01/graphql.json",
        ] {
            assert_eq!(
                action("shopify", "POST", path, Some(refund.clone())),
                AppAction::SendAsOwner,
                "{path}"
            );
            assert_eq!(
                outcome("shopify", "GET", path, None),
                Err(APP_GRAPHQL_SHAPE),
                "{path}"
            );
        }
        assert_eq!(
            action(
                "shopify",
                "POST",
                "/admin/api/graphql.json",
                Some(json!({"query": "{ shop { name } }"}))
            ),
            AppAction::Read
        );
        assert_eq!(
            outcome(
                "github",
                "POST",
                "/graphql",
                Some(json!({"query": "subscription { a }"}))
            ),
            Err(APP_GITHUB_GRAPHQL_WRITES)
        );
    }

    /// Final review A1 I1: a GitHub mutation would be Delivery, which runs
    /// without a card for a bruh with Can deliver but not Send as me, so a
    /// comment or approval sent over GraphQL would skip the card REST gets.
    #[test]
    fn github_graphql_takes_queries_only() {
        for document in [
            "mutation { addComment(input: {subjectId: \"I_1\", body: \"fixed\"}) { clientMutationId } }",
            "mutation { a: addPullRequestReview(input: {event: APPROVE}) { clientMutationId } }",
            "mutation { createSponsorship(input: {}) { clientMutationId } }",
            "query A { viewer { login } } mutation B { createIssue(input: {}) { clientMutationId } }",
            "subscription { a }",
            "{ a ",
        ] {
            assert_eq!(
                outcome("github", "POST", "/graphql", Some(json!({"query": document}))),
                Err(APP_GITHUB_GRAPHQL_WRITES),
                "{document}"
            );
        }
        assert_eq!(
            action(
                "github",
                "POST",
                "/graphql",
                Some(json!({"query": "query { viewer { login } }"}))
            ),
            AppAction::Read
        );
    }

    /// Final review A1 I2: a GraphQL field is selected only by its literal
    /// name, so an alias or a fragment can't hide one the rule refuses, in
    /// a mutation or a query.
    #[test]
    fn graphql_never_names_a_credential_field() {
        let shopify = "/admin/api/2026-10/graphql.json";
        for (app, path, document) in [
            ("shopify", shopify, "mutation { t: storefrontAccessTokenCreate(input: {title: \"x\"}) { storefrontAccessToken { id } } }"),
            ("shopify", shopify, "mutation { ...F } fragment F on Mutation { d: delegateAccessTokenCreate(input: {delegateAccessScope: [\"read_orders\"]}) { userErrors { field } } }"),
            ("shopify", shopify, "mutation { delegateAccessTokenDestroy(accessToken: \"x\") { status } }"),
            ("shopify", shopify, "mutation { w: webhookSubscriptionCreate(topic: ORDERS_CREATE, webhookSubscription: {callbackUrl: \"https://x.test\"}) { userErrors { field } } }"),
            ("shopify", shopify, "mutation { ...H } fragment H on Mutation { webhookSubscriptionUpdate(id: \"1\") { userErrors { field } } }"),
            ("shopify", shopify, "mutation { eventBridgeWebhookSubscriptionCreate(topic: A) { userErrors { field } } }"),
            ("shopify", shopify, "mutation { pubSubWebhookSubscriptionCreate(topic: A) { userErrors { field } } }"),
            ("shopify", shopify, "{ shop { ...S } } fragment S on Shop { t: storefrontAccessTokens(first: 5) { edges { node { id } } } }"),
            ("shopify", shopify, "{ shop { tokens: storefrontAccessTokens(first: 5) { edges { node { value: accessToken } } } } }"),
            ("shopify", shopify, "{ shop { STOREFRONTACCESSTOKENS(first: 1) { edges { node { id } } } } }"),
            ("linear", "/graphql", "mutation { w: webhookCreate(input: {url: \"https://x.test\"}) { success } }"),
            ("linear", "/graphql", "mutation { ...U } fragment U on Mutation { webhookUpdate(id: \"1\", input: {}) { success } }"),
            ("linear", "/graphql", "{ ...W } fragment W on Query { hooks: webhooks { nodes { s: secret } } }"),
            ("linear", "/graphql", "{ w: webhook(id: \"1\") { id } }"),
            ("linear", "/graphql", "mutation { apiKeyCreate(input: {}) { success } }"),
            ("linear", "/graphql", "mutation { oauthClientRotateSecret(id: \"1\") { success } }"),
        ] {
            assert_eq!(
                outcome(app, "POST", path, Some(json!({"query": document}))),
                Err(APP_CREDENTIAL_REFUSED),
                "{app} {document}"
            );
        }
        // Names inside strings and comments aren't fields; other calls go on.
        for (app, path, document, class) in [
            ("shopify", shopify, "{ shop { name } }", AppAction::Read),
            ("shopify", shopify, "# accessToken\n{ orders(first: 5, query: \"webhookSubscriptionCreate\") { edges { node { id } } } }", AppAction::Read),
            ("shopify", shopify, "mutation { productUpdate(input: {}) { product { id } } }", AppAction::SendAsOwner),
            ("linear", "/graphql", "{ issues(filter: {title: {contains: \"webhook secret\"}}) { nodes { id } } }", AppAction::Read),
            ("linear", "/graphql", "mutation { issueCreate(input: {title: \"x\"}) { success } }", AppAction::SendAsOwner),
        ] {
            assert_eq!(
                outcome(app, "POST", path, Some(json!({"query": document}))),
                Ok(class),
                "{app} {document}"
            );
        }
    }

    /// Re-review N2 and M4: a Shopify bulk operation carries a whole
    /// document in a string, which the classifier never reads, and a few
    /// mutations plant lasting outbound endpoints or code. Each is refused
    /// by name, with the document inline or in variables.
    #[test]
    fn shopify_bulk_operations_and_planted_endpoints_are_refused() {
        let shopify = "/admin/api/2026-10/graphql.json";
        let refused = |body: Value| outcome("shopify", "POST", shopify, Some(body));
        let inner = "{ shop { storefrontAccessTokens { edges { node { accessToken } } } } }";
        for body in [
            json!({"query": "mutation($q: String!) { bulkOperationRunQuery(query: $q) { bulkOperation { id } } }",
                "variables": {"q": inner}}),
            json!({"query": "mutation { b: bulkOperationRunQuery(query: \"{ orders { edges { node { id } } } }\") { bulkOperation { id } } }"}),
            json!({"query": "mutation($m: String!, $p: String!) { bulkOperationRunMutation(mutation: $m, stagedUploadPath: $p) { bulkOperation { id } } }",
                "variables": {"m": "mutation { productCreate(input: {}) { product { id } } }", "p": "x"}}),
            json!({"query": "mutation { stagedUploadsCreate(input: []) { stagedTargets { url } } }"}),
            json!({"query": "mutation { fulfillmentServiceCreate(name: \"x\", callbackUrl: \"https://x.test\") { userErrors { field } } }"}),
            json!({"query": "mutation { ...C } fragment C on Mutation { carrierServiceCreate(input: {}) { userErrors { field } } }"}),
            json!({"query": "mutation { s: scriptTagCreate(input: {src: \"https://x.test/a.js\"}) { userErrors { field } } }"}),
            json!({"query": "mutation { WEBPIXELCREATE(webPixel: {}) { userErrors { field } } }"}),
        ] {
            assert_eq!(refused(body.clone()), Err(APP_CREDENTIAL_REFUSED), "{body}");
        }
        // Reading a finished bulk operation is a plain query.
        assert_eq!(
            refused(json!({"query": "{ currentBulkOperation { status } }"})),
            Ok(AppAction::Read)
        );
    }

    /// Final review A1 M2: a stored-document key would make the app run a
    /// document the classifier never read, so the body holds the document,
    /// its variables and its operation name only.
    #[test]
    fn graphql_bodies_hold_only_the_document() {
        for (app, path) in [
            ("github", "/graphql"),
            ("linear", "/graphql"),
            ("shopify", "/admin/api/2026-10/graphql.json"),
        ] {
            for body in [
                json!({"query": "{ a }", "doc_id": "1"}),
                json!({"query": "{ a }", "documentId": "1"}),
                json!({"query": "{ a }", "id": "1"}),
                json!({"query": "{ a }", "queryId": "1"}),
                json!({"query": "{ a }", "extensions": {"persistedQuery": {"sha256Hash": "x"}}}),
                json!({"doc_id": "1"}),
                json!({"query": 5}),
                json!([{"query": "{ a }"}]),
                json!("{ a }"),
                // An unterminated string can't be read, so nothing in it is checked.
                json!({"query": "{ a(s: \"x }"}),
            ] {
                assert_eq!(
                    outcome(app, "POST", path, Some(body.clone())),
                    Err(APP_GRAPHQL_SHAPE),
                    "{app} {body}"
                );
            }
            assert_eq!(
                outcome(
                    app,
                    "POST",
                    path,
                    Some(json!({
                        "query": "query Q($n: Int) { a(n: $n) }",
                        "variables": {"n": 1},
                        "operationName": "Q"
                    }))
                ),
                Ok(AppAction::Read),
                "{app}"
            );
        }
    }

    #[test]
    fn allowed_money_and_send_actions_are_sent_as_the_owner() {
        for (app, method, path) in [
            ("stripe", "POST", "/v1/charges"),
            ("stripe", "POST", "/v1/payment_intents"),
            ("stripe", "POST", "/v1/payment_intents/pi_1/confirm"),
            ("stripe", "POST", "/v1/payment_intents/pi_1/capture"),
            ("stripe", "POST", "/v1/charges/ch_1/capture"),
            ("stripe", "POST", "/v1/subscriptions"),
            ("stripe", "POST", "/v1/subscriptions/sub_1"),
            ("stripe", "DELETE", "/v1/subscriptions/sub_1"),
            ("stripe", "POST", "/v1/invoices/in_1/send"),
            ("stripe", "POST", "/v1/invoices/in_1/pay"),
            ("stripe", "POST", "/v1/invoices/in_1/finalize"),
            ("stripe", "POST", "/v1/invoices/in_1/void"),
            ("stripe", "POST", "/v1/transfers"),
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
                "/admin/api/2026-10/orders/1/refunds.json",
            ),
            ("shopify", "POST", "/admin/api/2026-10/fulfillments.json"),
            (
                "shopify",
                "POST",
                "/admin/api/2026-10/draft_orders/1/send_invoice.json",
            ),
            (
                "shopify",
                "POST",
                "/admin/api/2026-10/customers/1/send_invite.json",
            ),
            ("jira", "POST", "/rest/api/3/issue"),
            ("jira", "POST", "/rest/api/3/issue/X-1/comment"),
            ("jira", "POST", "/rest/api/3/issue/X-1/notify"),
            (
                "hubspot",
                "POST",
                "/marketing/v3/transactional/single-email/send",
            ),
            (
                "hubspot",
                "POST",
                "/conversations/v3/conversations/threads/1/messages",
            ),
            ("figma", "POST", "/v1/files/f/comments"),
            ("figma", "POST", "/v1/files/f/comments/1/reactions"),
            ("clickup", "POST", "/api/v2/task/t/comment"),
            ("clickup", "POST", "/api/v2/list/l/comment"),
            (
                "clickup",
                "POST",
                "/api/v3/workspaces/1/chat/channels/c/messages",
            ),
            ("trello", "POST", "/1/cards/c/actions/comments"),
            ("todoist", "POST", "/api/v1/comments"),
            ("github", "POST", "/repos/o/r/issues"),
            ("github", "PATCH", "/repos/o/r/issues/1"),
            ("github", "POST", "/repos/o/r/pulls/1/reviews"),
            ("slack", "POST", "/api/chat.update"),
            ("slack", "POST", "/api/reactions.add"),
        ] {
            assert_eq!(
                action(app, method, path, Some(json!({}))),
                AppAction::SendAsOwner,
                "{app} {method} {path}"
            );
        }
        for (method, path) in [
            ("PUT", "/repos/o/r/pulls/1/update-branch"),
            ("POST", "/repos/o/r/git/refs"),
            ("PATCH", "/repos/o/r/git/refs/heads/main"),
        ] {
            assert_eq!(
                action("github", method, path, Some(json!({}))),
                AppAction::Delivery,
                "{method} {path}"
            );
        }
        for (app, method, path, class) in [
            (
                "jira",
                "POST",
                "/rest/api/3/issue/bulkfetch",
                AppAction::Read,
            ),
            // N2: an edit or transition can carry a comment.
            (
                "jira",
                "PUT",
                "/rest/api/3/issue/X-1",
                AppAction::SendAsOwner,
            ),
            (
                "jira",
                "POST",
                "/rest/api/3/issue/X-1/transitions",
                AppAction::SendAsOwner,
            ),
            // N1: invoices and invoice items can charge or email a customer.
            ("stripe", "POST", "/v1/invoices", AppAction::SendAsOwner),
            (
                "stripe",
                "POST",
                "/v1/invoices/in_1",
                AppAction::SendAsOwner,
            ),
            ("stripe", "POST", "/v1/invoiceitems", AppAction::SendAsOwner),
            ("stripe", "POST", "/v1/customers/cus_1", AppAction::Write),
            ("stripe", "POST", "/v1/products/prod_1", AppAction::Write),
            // Todoist quick add's note is a comment.
            (
                "todoist",
                "POST",
                "/api/v1/tasks/quick",
                AppAction::SendAsOwner,
            ),
            ("todoist", "POST", "/api/v1/tasks/1", AppAction::Write),
            (
                "hubspot",
                "POST",
                "/crm/v3/objects/contacts/search",
                AppAction::Read,
            ),
            (
                "hubspot",
                "PATCH",
                "/crm/v3/objects/contacts/1",
                AppAction::Write,
            ),
            (
                "airtable",
                "POST",
                "/v0/app1/tbl1/listRecords",
                AppAction::Read,
            ),
            ("airtable", "PATCH", "/v0/app1/tbl1", AppAction::Write),
            ("todoist", "POST", "/api/v1/tasks/1/close", AppAction::Write),
            ("trello", "PUT", "/1/cards/c", AppAction::Write),
            ("clickup", "PUT", "/api/v2/task/t", AppAction::Write),
            (
                "shopify",
                "PUT",
                "/admin/api/2026-10/products/1.json",
                AppAction::Write,
            ),
            (
                "cloudflare",
                "POST",
                "/client/v4/zones/z/dns_records",
                AppAction::SendAsOwner,
            ),
            ("figma", "GET", "/v1/files/f/nodes", AppAction::Read),
        ] {
            assert_eq!(
                action(app, method, path, Some(json!({}))),
                class,
                "{app} {method} {path}"
            );
        }
        // N1: a bruh can't set a Stripe customer's balance, in the query
        // (where Stripe parameters go) or the body, in any case or nesting.
        let stripe = connector("stripe").unwrap();
        for path in ["/v1/customers", "/v1/customers/cus_1"] {
            for key in ["balance", "BALANCE", "balance[x]"] {
                let mut in_query = call("stripe", "POST", path, None);
                in_query.query.insert(key.into(), json!(500));
                assert_eq!(
                    classify(stripe, &in_query),
                    Err(APP_FIELD_REFUSED),
                    "{path} {key}"
                );
                let in_body = call(
                    "stripe",
                    "POST",
                    path,
                    Some(Value::Object(serde_json::Map::from_iter([(
                        key.to_string(),
                        json!(500),
                    )]))),
                );
                assert_eq!(
                    classify(stripe, &in_body),
                    Err(APP_FIELD_REFUSED),
                    "{path} {key}"
                );
            }
            let mut fine = call("stripe", "POST", path, None);
            fine.query.insert("name".into(), json!("Ann"));
            fine.query.insert("balance_note".into(), json!("x"));
            assert_eq!(classify(stripe, &fine), Ok(AppAction::Write), "{path}");
        }
        // Nothing unlisted is ever a read, or anything else.
        for connector in &CONNECTORS {
            for method in APP_METHODS {
                let refused = classify(
                    connector,
                    &call(connector.id, method, "/no/such/endpoint", None),
                );
                assert_eq!(
                    refused,
                    Err(connector.unlisted),
                    "{} {method}",
                    connector.id
                );
            }
        }
    }

    /// Final review A1 M3 and M4: a post can't show as someone else, and a
    /// write that would notify people or redirect mail asks like a send.
    #[test]
    fn sends_never_speak_as_someone_else_and_writes_never_notify() {
        let with_key = |app: &str, method: &str, path: &str, key: &str, in_query: bool| {
            let mut call = call(app, method, path, None);
            if in_query {
                call.query.insert(key.into(), json!("x"));
            } else {
                call.body = Some(Value::Object(serde_json::Map::from_iter([(
                    key.to_string(),
                    json!("x"),
                )])));
            }
            classify(connector(app).unwrap(), &call)
        };
        for path in [
            "/api/chat.postMessage",
            "/api/chat.update",
            "/api/chat.scheduleMessage",
        ] {
            for key in ["username", "icon_url", "icon_emoji", "as_user", "USERNAME"] {
                assert_eq!(
                    with_key("slack", "POST", path, key, false),
                    Err(APP_FIELD_REFUSED),
                    "{path} {key}"
                );
                assert_eq!(
                    with_key("slack", "GET", path, key, true),
                    Err(APP_FIELD_REFUSED),
                    "{path} {key}"
                );
            }
            assert_eq!(
                with_key("slack", "POST", path, "text", false),
                Ok(AppAction::SendAsOwner),
                "{path}"
            );
        }
        for (app, method, path, key, in_query) in [
            (
                "clickup",
                "POST",
                "/api/v2/list/l/task",
                "notify_all",
                false,
            ),
            ("clickup", "POST", "/api/v2/list/l/task", "assignees", false),
            ("clickup", "PUT", "/api/v2/task/t", "assignees", false),
            ("trello", "POST", "/1/cards", "idMembers", true),
            ("trello", "PUT", "/1/cards/c", "idMembers", true),
            ("todoist", "POST", "/api/v1/tasks", "assignee_id", false),
            ("todoist", "POST", "/api/v1/tasks/1", "assignee_id", false),
            (
                "todoist",
                "POST",
                "/api/v1/tasks/1",
                "responsible_uid",
                false,
            ),
            ("stripe", "POST", "/v1/customers/cus_1", "email", true),
        ] {
            assert_eq!(
                with_key(app, method, path, key, in_query),
                Err(APP_FIELD_REFUSED),
                "{app} {method} {path} {key}"
            );
        }
        // A new Stripe customer may have an email; nothing is sent to it yet.
        assert_eq!(
            with_key("stripe", "POST", "/v1/customers", "email", true),
            Ok(AppAction::Write)
        );
        assert_eq!(
            with_key("clickup", "POST", "/api/v2/list/l/task", "name", false),
            Ok(AppAction::Write)
        );
        // A DNS change can capture the owner's mail and password resets.
        for (method, path) in [
            ("POST", "/client/v4/zones/z/dns_records"),
            ("PUT", "/client/v4/zones/z/dns_records/r"),
            ("PATCH", "/client/v4/zones/z/dns_records/r"),
            ("DELETE", "/client/v4/zones/z/dns_records/r"),
        ] {
            let body = (method != "DELETE").then(|| json!({}));
            assert_eq!(
                action("cloudflare", method, path, body),
                AppAction::SendAsOwner,
                "{method} {path}"
            );
        }
    }

    /// Final review A1 M5: HubSpot's object types are named, so a type that
    /// charges or sends (invoices, quotes, payments, a custom object) is
    /// refused until the catalog names it.
    #[test]
    fn hubspot_names_its_object_types() {
        for kind in [
            "contacts",
            "companies",
            "deals",
            "tickets",
            "notes",
            "tasks",
        ] {
            let base = format!("/crm/v3/objects/{kind}");
            for (method, path, class) in [
                ("GET", base.clone(), AppAction::Read),
                ("GET", format!("{base}/1"), AppAction::Read),
                ("POST", format!("{base}/search"), AppAction::Read),
                ("POST", format!("{base}/batch/read"), AppAction::Read),
                ("POST", base.clone(), AppAction::Write),
                ("PATCH", format!("{base}/1"), AppAction::Write),
            ] {
                let body = (method != "GET").then(|| json!({}));
                assert_eq!(
                    action("hubspot", method, &path, body),
                    class,
                    "{method} {path}"
                );
            }
        }
        for kind in [
            "invoices",
            "quotes",
            "payments",
            "line_items",
            "2-123",
            "p_x",
        ] {
            for (method, path) in [
                ("GET", format!("/crm/v3/objects/{kind}")),
                ("POST", format!("/crm/v3/objects/{kind}")),
                ("PATCH", format!("/crm/v3/objects/{kind}/1")),
            ] {
                let body = (method != "GET").then(|| json!({}));
                assert_eq!(
                    outcome("hubspot", method, &path, body),
                    Err(connector("hubspot").unwrap().unlisted),
                    "{method} {path}"
                );
            }
        }
    }

    /// Endpoints that alias an allowlisted one, make, read or change a
    /// credential, key or webhook, or move money, code or access in ways
    /// the catalog doesn't name yet (F6, review C1, C3, I3, I4). Each must be
    /// refused; the allowlist refuses them by construction.
    const REFUSED: &[(&str, &str, &str)] = &[
        // C1: GitHub's id aliases.
        ("github", "PUT", "/repositories/1/pulls/1/merge"),
        ("github", "PUT", "/repositories/1/contents/a.rs"),
        ("github", "GET", "/repositories/1/issues"),
        ("github", "POST", "/repositories/1/keys"),
        ("github", "POST", "/repositories/1/hooks"),
        ("github", "PUT", "/repositories/1/actions/secrets/X"),
        ("github", "PUT", "/repositories/1/environments/p/secrets/X"),
        ("github", "POST", "/organizations/1/hooks"),
        (
            "github",
            "POST",
            "/organizations/1/actions/runners/registration-token",
        ),
        // F6 (a): GitHub keys, hooks, secrets and runner tokens.
        ("github", "POST", "/user/keys"),
        ("github", "POST", "/user/gpg_keys"),
        ("github", "POST", "/user/ssh_signing_keys"),
        ("github", "POST", "/repos/o/r/keys"),
        ("github", "POST", "/repos/o/r/hooks"),
        ("github", "GET", "/repos/o/r/hooks"),
        ("github", "PATCH", "/repos/o/r/hooks/1/config"),
        ("github", "POST", "/orgs/o/hooks"),
        ("github", "PATCH", "/orgs/o/hooks/1"),
        ("github", "PUT", "/repos/o/r/actions/secrets/X"),
        ("github", "PUT", "/orgs/o/actions/secrets/X"),
        ("github", "PUT", "/repos/o/r/environments/prod/secrets/X"),
        ("github", "PUT", "/repos/o/r/dependabot/secrets/X"),
        ("github", "PUT", "/orgs/o/dependabot/secrets/X"),
        ("github", "PUT", "/user/codespaces/secrets/X"),
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
        // I3: GitHub.
        ("github", "PUT", "/repos/o/r/codespaces/secrets/X"),
        ("github", "PUT", "/orgs/o/codespaces/secrets/X"),
        ("github", "POST", "/app-manifests/c/conversions"),
        ("github", "POST", "/orgs/o/personal-access-token-requests"),
        ("github", "POST", "/orgs/o/personal-access-tokens/1"),
        ("github", "POST", "/orgs/o/private-registries"),
        ("github", "PUT", "/orgs/o/private-registries/r"),
        (
            "github",
            "POST",
            "/enterprises/e/actions/runners/registration-token",
        ),
        (
            "github",
            "POST",
            "/enterprises/e/actions/runners/remove-token",
        ),
        (
            "github",
            "POST",
            "/enterprises/e/actions/runners/generate-jitconfig",
        ),
        ("github", "PUT", "/repos/o/r/actions/oidc/customization/sub"),
        // F6 (b) and I4: GitHub access, code and run changes not named yet.
        ("github", "PUT", "/repos/o/r/collaborators/eve"),
        ("github", "PATCH", "/repos/o/r"),
        ("github", "DELETE", "/repos/o/r"),
        ("github", "POST", "/repos/o/r/transfer"),
        ("github", "POST", "/repos/o/r/dispatches"),
        (
            "github",
            "POST",
            "/repos/o/r/actions/workflows/ci.yml/dispatches",
        ),
        ("github", "POST", "/repos/o/r/merges"),
        ("github", "DELETE", "/repos/o/r/contents/a.rs"),
        ("github", "DELETE", "/repos/o/r/git/refs/heads/main"),
        ("github", "POST", "/repos/o/r/merge-upstream"),
        ("github", "POST", "/repos/o/r/branches/main/rename"),
        ("github", "POST", "/repos/o/r/releases"),
        ("github", "PUT", "/repos/o/r/branches/main/protection"),
        (
            "github",
            "POST",
            "/repos/o/r/branches/main/protection/enforce_admins",
        ),
        (
            "github",
            "DELETE",
            "/repos/o/r/branches/main/protection/required_signatures",
        ),
        ("github", "POST", "/repos/o/r/deployments"),
        ("github", "POST", "/repos/o/r/actions/runs/1/rerun"),
        (
            "github",
            "POST",
            "/repos/o/r/actions/runs/1/rerun-failed-jobs",
        ),
        ("github", "POST", "/repos/o/r/actions/runs/1/approve"),
        ("github", "PUT", "/orgs/o/memberships/eve"),
        ("github", "PUT", "/orgs/o/teams/t/repos/o/r"),
        ("github", "PUT", "/orgs/o/outside_collaborators/eve"),
        ("github", "PATCH", "/repos/o/r/invitations/1"),
        ("github", "GET", "/repos/o/r/tarball/main"),
        ("github", "GET", "/repos/o/r/actions/runs/1/logs"),
        // F6 (a) and I3: Cloudflare.
        ("cloudflare", "POST", "/client/v4/user/tokens"),
        ("cloudflare", "PUT", "/client/v4/user/tokens/t"),
        ("cloudflare", "PUT", "/client/v4/user/tokens/t/value"),
        ("cloudflare", "POST", "/client/v4/accounts/a/tokens"),
        ("cloudflare", "PUT", "/client/v4/accounts/a/tokens/t"),
        ("cloudflare", "PUT", "/client/v4/accounts/a/tokens/t/value"),
        (
            "cloudflare",
            "POST",
            "/client/v4/accounts/a/alerting/v3/destinations/webhooks",
        ),
        (
            "cloudflare",
            "POST",
            "/client/v4/accounts/a/access/service_tokens",
        ),
        (
            "cloudflare",
            "POST",
            "/client/v4/accounts/a/access/service_tokens/t/rotate",
        ),
        (
            "cloudflare",
            "POST",
            "/client/v4/accounts/a/access/service_tokens/t/refresh",
        ),
        (
            "cloudflare",
            "POST",
            "/client/v4/zones/z/access/service_tokens",
        ),
        (
            "cloudflare",
            "GET",
            "/client/v4/accounts/a/cfd_tunnel/t/token",
        ),
        (
            "cloudflare",
            "PUT",
            "/client/v4/accounts/a/workers/scripts/s/secrets",
        ),
        ("cloudflare", "PUT", "/client/v4/accounts/a/stream/webhook"),
        ("cloudflare", "POST", "/client/v4/accounts/a/logpush/jobs"),
        ("cloudflare", "POST", "/client/v4/zones/z/logpush/jobs"),
        // F6 (a), I3 and I4: Stripe.
        ("stripe", "POST", "/v1/webhook_endpoints"),
        ("stripe", "POST", "/v1/webhook_endpoints/we_1"),
        ("stripe", "POST", "/v1/ephemeral_keys"),
        ("stripe", "POST", "/v1/apps/secrets"),
        ("stripe", "GET", "/v1/apps/secrets/find"),
        ("stripe", "POST", "/v1/terminal/connection_tokens"),
        ("stripe", "POST", "/v1/accounts/acct_1/login_links"),
        ("stripe", "POST", "/v1/account_links"),
        ("stripe", "POST", "/v1/account_sessions"),
        ("stripe", "POST", "/v1/billing_portal/sessions"),
        ("stripe", "POST", "/v2/core/event_destinations"),
        ("stripe", "GET", "/v1/issuing/cards/ic_1"),
        ("stripe", "POST", "/v1/charges/ch_1/refund"),
        ("stripe", "POST", "/v1/charges/ch_1/refunds"),
        ("stripe", "POST", "/v1/topups"),
        ("stripe", "POST", "/v1/treasury/outbound_payments"),
        ("stripe", "POST", "/v1/treasury/outbound_transfers"),
        ("stripe", "POST", "/v1/payouts/po_1/reverse"),
        ("stripe", "POST", "/v1/application_fees/fee_1/refund"),
        ("stripe", "POST", "/v1/application_fees/fee_1/refunds"),
        ("stripe", "POST", "/v1/subscription_items"),
        ("stripe", "DELETE", "/v1/subscription_items/si_1"),
        ("stripe", "POST", "/v1/subscription_schedules"),
        (
            "stripe",
            "POST",
            "/v1/subscription_schedules/sub_sched_1/release",
        ),
        ("stripe", "POST", "/v1/quotes/qt_1/accept"),
        (
            "stripe",
            "POST",
            "/v1/payment_intents/pi_1/increment_authorization",
        ),
        // F6 (a), I3 and I4: Shopify, versioned and not.
        (
            "shopify",
            "POST",
            "/admin/api/2026-10/storefront_access_tokens.json",
        ),
        ("shopify", "POST", "/admin/api/2026-10/webhooks.json"),
        ("shopify", "PUT", "/admin/api/2026-10/webhooks/1.json"),
        ("shopify", "POST", "/admin/webhooks.json"),
        ("shopify", "POST", "/admin/api/2026-10/webhooks"),
        ("shopify", "POST", "/admin/storefront_access_tokens.json"),
        ("shopify", "POST", "/admin/orders.json"),
        (
            "shopify",
            "POST",
            "/admin/api/2026-10/customers/1/account_activation_url.json",
        ),
        (
            "shopify",
            "POST",
            "/admin/api/2026-10/carrier_services.json",
        ),
        (
            "shopify",
            "POST",
            "/admin/api/2026-10/fulfillment_services.json",
        ),
        (
            "shopify",
            "POST",
            "/admin/api/2026-10/draft_orders/1/complete.json",
        ),
        ("shopify", "POST", "/admin/api/2026-10/gift_cards.json"),
        // F6 (a) and I3: Slack.
        ("slack", "POST", "/api/apps.connections.open"),
        ("slack", "GET", "/api/apps.manifest.export"),
        ("slack", "GET", "/api/apps.permissions.list"),
        ("slack", "POST", "/api/oauth.v2.access"),
        ("slack", "POST", "/api/openid.connect.token"),
        ("slack", "POST", "/api/auth.revoke"),
        ("slack", "POST", "/api/tooling.tokens.rotate"),
        ("slack", "POST", "/api/admin.apps.approve"),
        ("slack", "GET", "/api/admin.apps.requests.list"),
        ("slack", "GET", "/api/rtm.connect"),
        ("slack", "GET", "/api/rtm.start"),
        // F6 (a), I3 and I4: Airtable.
        ("airtable", "POST", "/v0/bases/app1/webhooks"),
        ("airtable", "GET", "/v0/bases/app1/webhooks"),
        ("airtable", "POST", "/v0/bases/app1/webhooks/w/refresh"),
        (
            "airtable",
            "POST",
            "/v0/bases/app1/webhooks/w/enableNotifications",
        ),
        ("airtable", "POST", "/v0/app1/tbl1/rec1/comments"),
        // F6 (a): Figma, Trello, ClickUp.
        ("figma", "POST", "/v2/webhooks"),
        ("figma", "PUT", "/v2/webhooks/1"),
        ("trello", "POST", "/1/webhooks"),
        ("trello", "PUT", "/1/webhooks/1"),
        ("trello", "POST", "/1/tokens/t/webhooks"),
        ("trello", "GET", "/1/tokens/t"),
        ("trello", "GET", "/1/members/me/tokens"),
        ("clickup", "POST", "/api/v2/team/1/webhook"),
        ("clickup", "PUT", "/api/v2/webhook/1"),
        ("clickup", "POST", "/api/v2/comment/1/reply"),
        // C3 and I4: Jira's other roots, webhooks, Service Management and Confluence.
        ("jira", "POST", "/rest/api/2/issue/X-1/comment"),
        ("jira", "POST", "/rest/api/2/issue/X-1/notify"),
        ("jira", "POST", "/rest/api/2/issue"),
        ("jira", "POST", "/rest/api/latest/issue/X-1/comment"),
        ("jira", "GET", "/rest/api/2/myself"),
        ("jira", "POST", "/rest/api/3/webhook"),
        ("jira", "PUT", "/rest/api/3/webhook/refresh"),
        ("jira", "POST", "/rest/webhooks/1.0/webhook"),
        ("jira", "PUT", "/rest/webhooks/1.0/webhook/1"),
        ("jira", "POST", "/rest/servicedeskapi/request/1/comment"),
        ("jira", "POST", "/wiki/rest/api/content/1/child/comment"),
        ("jira", "POST", "/wiki/api/v2/footer-comments"),
        // I4: Todoist sync, HubSpot email paths not named yet.
        ("todoist", "POST", "/api/v1/sync"),
        ("todoist", "POST", "/rest/v2/comments"),
        ("hubspot", "POST", "/marketing/v4/email/single-send"),
        ("hubspot", "POST", "/automation/v4/sequences/enrollments"),
        // I1: matrix parameters never reach a router that strips them.
        ("jira", "POST", "/rest/api/3/webhook;x"),
        (
            "hubspot",
            "POST",
            "/marketing/v3/transactional/single-email/send;x",
        ),
        // F4: escapes and case don't get round it.
        ("github", "POST", "/user/%6Beys"),
        ("stripe", "POST", "/V1/WEBHOOK_ENDPOINTS"),
        ("notion", "GET", "/a/../b"),
    ];

    #[test]
    fn credential_alias_and_unnamed_endpoints_are_refused() {
        for (app, method, path) in REFUSED {
            let body = (!matches!(*method, "GET" | "DELETE")).then(|| json!({}));
            let result = outcome(app, method, path, body);
            assert!(
                result.is_err(),
                "{app} {method} {path} classified as {result:?}"
            );
        }
        // A refused endpoint names its app; a bad path names the path.
        assert_eq!(
            outcome("github", "POST", "/user/keys", Some(json!({}))),
            Err("Bruhs can't call that endpoint of GitHub yet.")
        );
        assert_eq!(
            outcome("jira", "POST", "/rest/api/3/webhook;x", Some(json!({}))),
            Err(APP_PATH)
        );
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
