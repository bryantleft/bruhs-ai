//! The one client that calls apps (D13, D14).
//!
//! - Egress: https only, no proxy, no redirects, and a resolver that answers
//!   only for a declared host and refuses the whole answer when any address
//!   isn't public or is this computer. The connection goes to the addresses
//!   the resolver checked, so a second lookup can't swap them.
//! - The URL is built from the declared host and the normalized path, the
//!   query appended pair by pair (keys and values percent-encoded, so a key
//!   is sent as the literal text the gate saw), then checked again.
//! - Replies: any status, at most `APP_READ_MAX_BYTES` read and
//!   `APP_RESPONSE_MAX_BYTES` returned, only `RESPONSE_HEADERS` kept, and
//!   every form of the token scrubbed from the raw text, the parsed JSON and
//!   each kept header (F8). Failures are fixed copy.
//!
//! The debug-only test upstream (D24, F9) is a second, plain http client
//! that runs every step above, then swaps only the scheme and authority.

use super::AppsError;
use crate::secrets::AppSecret;
use base64::Engine;
use bruhs_models::apps::{
    app_host, host_declared, normalize_path, validate_call, AppAuth, AppCall, AppConnector,
    AppResponse, APP_BODY_MAX_BYTES, APP_BODY_TOO_LARGE, APP_CALL_TIMEOUT_MS, APP_CUT_NOTE,
    APP_GET_BODY, APP_HOST_REFUSED, APP_METHOD, APP_METHODS, APP_NOT_TEXT, APP_PATH,
    APP_QUERY_VALUES, APP_READ_MAX_BYTES, APP_RESPONSE_MAX_BYTES, APP_TIMED_OUT, APP_TOO_LARGE,
    APP_UNREACHABLE, EMAIL_SHAPE, MAX_APP_CALLS, RESPONSE_HEADERS, TOKEN_SHAPE, UNKNOWN_APP,
    USER_AGENT,
};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use serde_json::Value;
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    future::Future,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;
use url::Url;
use zeroize::Zeroizing;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CALL_TIMEOUT: Duration = Duration::from_millis(APP_CALL_TIMEOUT_MS);
/// What every secret is replaced with.
const REDACTED: &str = "[redacted]";
/// Shorter strings can't be scrubbed without cutting ordinary words (F8);
/// the catalog refuses tokens and keys this short.
const MIN_SECRET_BYTES: usize = 8;
/// A kept reply header longer than this is dropped, not cut, so a bruh never
/// follows half a link.
const MAX_HEADER_BYTES: usize = 4 * 1024;
/// The declared host, sent to the test upstream in place of the real one.
#[cfg(any(test, feature = "test-support", debug_assertions))]
const APP_HOST_HEADER: &str = "x-bruhs-app-host";
/// RFC 3986 unreserved characters stay; everything else is escaped.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>>;
type Lookup = Arc<dyn Fn(String) -> LookupFuture + Send + Sync>;

/// The app client: one per process, cloned cheaply, with at most
/// [`MAX_APP_CALLS`] calls in flight across every clone.
#[derive(Clone)]
pub struct AppHttp {
    client: reqwest::Client,
    route: Route,
    calls: Arc<Semaphore>,
    timeout: Duration,
}

#[derive(Clone, Copy)]
enum Route {
    /// The app's own host, through the guarded resolver.
    Apps,
    /// Every call to this loopback address over http (D24).
    #[cfg(any(test, feature = "test-support", debug_assertions))]
    Upstream(SocketAddr),
}

/// One request as [`AppHttp::call`] or [`AppHttp::raw`] hands it on.
struct Outgoing<'a> {
    method: &'a str,
    path: &'a str,
    query: &'a BTreeMap<String, Value>,
    body: Option<&'a Value>,
}

impl AppHttp {
    /// The production client: rustls, https only, no proxy, no redirects,
    /// and the [`GuardedResolver`].
    pub fn system() -> Result<Self, AppsError> {
        Self::guarded(GuardedResolver::system())
    }

    fn guarded(resolver: GuardedResolver) -> Result<Self, AppsError> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .dns_resolver(Arc::new(resolver))
            .build()
            .map_err(|_| AppsError::Unavailable(APP_UNREACHABLE))?;
        Ok(Self::with_client(client, Route::Apps))
    }

    /// Sends every call to `addr` over plain http, with the declared host in
    /// `x-bruhs-app-host` (D24). Its own client, with no resolver: the
    /// production client is never loosened for it (F9). Never compiled into
    /// a release build.
    #[cfg(any(test, feature = "test-support", debug_assertions))]
    pub fn with_upstream(addr: SocketAddr) -> Result<Self, AppsError> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|_| AppsError::Unavailable(APP_UNREACHABLE))?;
        Ok(Self::with_client(client, Route::Upstream(addr)))
    }

    /// [`Self::system`], or in a debug build the test upstream that
    /// [`super::APPS_UPSTREAM_ENV`] names. A release build ignores the
    /// variable and logs that it did.
    pub fn from_env() -> Result<Self, AppsError> {
        let value = std::env::var(super::APPS_UPSTREAM_ENV).ok();
        let upstream = upstream_from(value.as_deref(), cfg!(debug_assertions));
        #[cfg(debug_assertions)]
        if let Some(addr) = upstream {
            tracing::warn!(
                "app calls go to the test upstream {}; this build never reaches a real app",
                super::APPS_UPSTREAM_ENV
            );
            return Self::with_upstream(addr);
        }
        if value.is_some() && upstream.is_none() {
            tracing::warn!(
                "{} is ignored: release builds ignore it, and debug builds take only http://127.0.0.1:<port> or http://[::1]:<port>",
                super::APPS_UPSTREAM_ENV
            );
        }
        Self::system()
    }

    fn with_client(client: reqwest::Client, route: Route) -> Self {
        AppHttp {
            client,
            route,
            calls: Arc::new(Semaphore::new(MAX_APP_CALLS)),
            timeout: CALL_TIMEOUT,
        }
    }

    #[cfg(test)]
    fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Sends a bruh's call on its normalized path. The gate has already
    /// classified it; this checks the call's limits again and that it is
    /// for `connector`.
    #[cfg_attr(not(test), expect(dead_code, reason = "used from Task 8"))]
    pub(crate) async fn call(
        &self,
        connector: &'static AppConnector,
        params: &BTreeMap<String, String>,
        secret: &AppSecret,
        call: &AppCall,
    ) -> Result<AppResponse, AppsError> {
        if call.app != connector.id {
            return Err(AppsError::Invalid(UNKNOWN_APP));
        }
        validate_call(call).map_err(AppsError::Invalid)?;
        let outgoing = Outgoing {
            method: &call.method,
            path: &call.path,
            query: &call.query,
            body: call.body.as_ref(),
        };
        self.send(connector, params, secret, outgoing).await
    }

    /// A call Bruhs itself makes, with no query: a token's test call and
    /// the polls.
    #[cfg_attr(not(test), expect(dead_code, reason = "used from Task 8"))]
    pub(crate) async fn raw(
        &self,
        connector: &'static AppConnector,
        params: &BTreeMap<String, String>,
        secret: &AppSecret,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<AppResponse, AppsError> {
        let query = BTreeMap::new();
        let outgoing = Outgoing {
            method,
            path,
            query: &query,
            body,
        };
        self.send(connector, params, secret, outgoing).await
    }

    async fn send(
        &self,
        connector: &AppConnector,
        params: &BTreeMap<String, String>,
        secret: &AppSecret,
        outgoing: Outgoing<'_>,
    ) -> Result<AppResponse, AppsError> {
        let method = app_method(outgoing.method)?;
        let body = match outgoing.body {
            Some(_) if matches!(method, reqwest::Method::GET | reqwest::Method::DELETE) => {
                return Err(AppsError::Invalid(APP_GET_BODY))
            }
            Some(body) => Some(
                serde_json::to_vec(body)
                    .ok()
                    .filter(|bytes| bytes.len() <= APP_BODY_MAX_BYTES)
                    .ok_or(AppsError::Invalid(APP_BODY_TOO_LARGE))?,
            ),
            None => None,
        };
        let url = app_url(connector, params, outgoing.path, outgoing.query)?;
        let headers = request_headers(connector, params, secret)?;
        let secrets = secret_strings(connector, params, secret);
        let secrets: Vec<&str> = secrets.iter().map(|text| text.as_str()).collect();
        let mut request = match self.route {
            Route::Apps => self.client.request(method, url),
            #[cfg(any(test, feature = "test-support", debug_assertions))]
            Route::Upstream(addr) => {
                let host = HeaderValue::from_str(url.host_str().unwrap_or_default())
                    .map_err(|_| AppsError::Refused(APP_HOST_REFUSED))?;
                self.client
                    .request(method, upstream_url(url, addr)?)
                    .header(APP_HOST_HEADER, host)
            }
        }
        .headers(headers);
        if let Some(body) = body {
            request = request
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(body);
        }
        // One deadline covers the wait for a slot and the whole call, so a
        // call never outlives APP_CALL_TIMEOUT_MS (F11).
        let work = async {
            let _slot = self
                .calls
                .acquire()
                .await
                .map_err(|_| AppsError::Unavailable(APP_UNREACHABLE))?;
            let response = request.send().await.map_err(transport_error)?;
            read_reply(response, &secrets).await
        };
        tokio::time::timeout(self.timeout, work)
            .await
            .map_err(|_| AppsError::TimedOut(APP_TIMED_OUT))?
    }
}

fn app_method(method: &str) -> Result<reqwest::Method, AppsError> {
    let upper = method.to_ascii_uppercase();
    if !APP_METHODS.contains(&upper.as_str()) {
        return Err(AppsError::Invalid(APP_METHOD));
    }
    reqwest::Method::from_bytes(upper.as_bytes()).map_err(|_| AppsError::Invalid(APP_METHOD))
}

/// `https://{app_host}` plus the normalized path, exactly, and the query
/// pair by pair (numbers and booleans as their JSON text), then
/// [`checked_url`].
fn app_url(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
    path: &str,
    query: &BTreeMap<String, Value>,
) -> Result<Url, AppsError> {
    let host = app_host(connector, params).map_err(|_| AppsError::Refused(APP_HOST_REFUSED))?;
    let path = normalize_path(path).map_err(AppsError::Invalid)?;
    let mut url =
        Url::parse(&format!("https://{host}")).map_err(|_| AppsError::Refused(APP_HOST_REFUSED))?;
    url.set_path(&path);
    if url.path() != path {
        return Err(AppsError::Invalid(APP_PATH));
    }
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Number(number) => number.to_string(),
                Value::Bool(flag) => flag.to_string(),
                Value::Null | Value::Array(_) | Value::Object(_) => {
                    return Err(AppsError::Invalid(APP_QUERY_VALUES))
                }
            };
            pairs.append_pair(key, &text);
        }
    }
    checked_url(url, &host)
}

/// The last check before a request is built: https, exactly the declared
/// `host` on the default port, no user info and no fragment.
fn checked_url(url: Url, host: &str) -> Result<Url, AppsError> {
    let safe = url.scheme() == "https"
        && host_declared(host)
        && url.host_str() == Some(host)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none();
    if safe {
        Ok(url)
    } else {
        Err(AppsError::Refused(APP_HOST_REFUSED))
    }
}

/// The checked URL with only its scheme and authority swapped for the test
/// upstream's.
#[cfg(any(test, feature = "test-support", debug_assertions))]
fn upstream_url(mut url: Url, addr: SocketAddr) -> Result<Url, AppsError> {
    let refused = |()| AppsError::Refused(APP_HOST_REFUSED);
    url.set_scheme("http").map_err(refused)?;
    url.set_ip_host(addr.ip()).map_err(refused)?;
    url.set_port(Some(addr.port())).map_err(refused)?;
    Ok(url)
}

/// A test upstream address from [`super::APPS_UPSTREAM_ENV`]: only
/// `http://127.0.0.1:<port>` or `http://[::1]:<port>`, and only when
/// `debug` (F9).
fn upstream_from(value: Option<&str>, debug: bool) -> Option<SocketAddr> {
    if !debug {
        return None;
    }
    let url = Url::parse(value?).ok()?;
    let ip = match url.host()? {
        url::Host::Ipv4(ip) if ip == std::net::Ipv4Addr::LOCALHOST => IpAddr::V4(ip),
        url::Host::Ipv6(ip) if ip.is_loopback() => IpAddr::V6(ip),
        _ => return None,
    };
    let plain = url.scheme() == "http"
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none();
    let port = url.port().filter(|port| *port != 0)?;
    plain.then_some(SocketAddr::new(ip, port))
}

/// The connector's headers, [`USER_AGENT`] and the auth header, marked
/// sensitive so no `Debug` of the request prints it.
fn request_headers(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
    secret: &AppSecret,
) -> Result<HeaderMap, AppsError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::USER_AGENT,
        HeaderValue::from_static(USER_AGENT),
    );
    for (name, value) in connector.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| AppsError::Unavailable(APP_UNREACHABLE))?;
        headers.insert(name, HeaderValue::from_static(value));
    }
    let (name, value) = auth_header(connector, params, secret)?;
    let mut value =
        HeaderValue::from_str(value.as_str()).map_err(|_| AppsError::Invalid(TOKEN_SHAPE))?;
    value.set_sensitive(true);
    headers.insert(name, value);
    Ok(headers)
}

/// The auth header's name and whole value (D4).
fn auth_header(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
    secret: &AppSecret,
) -> Result<(HeaderName, Zeroizing<String>), AppsError> {
    let token = secret.token.as_str();
    Ok(match connector.auth {
        AppAuth::Bearer => (AUTHORIZATION, Zeroizing::new(format!("Bearer {token}"))),
        AppAuth::Basic => {
            let credentials = basic_credentials(params, secret)?;
            (
                AUTHORIZATION,
                Zeroizing::new(format!("Basic {}", credentials.as_str())),
            )
        }
        AppAuth::Header(name) => (
            HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| AppsError::Unavailable(APP_UNREACHABLE))?,
            Zeroizing::new(token.to_owned()),
        ),
        AppAuth::Trello => {
            let key = secret.key.as_ref().ok_or(AppsError::Invalid(TOKEN_SHAPE))?;
            (
                AUTHORIZATION,
                Zeroizing::new(format!(
                    r#"OAuth oauth_consumer_key="{}", oauth_token="{token}""#,
                    key.as_str()
                )),
            )
        }
    })
}

/// `base64(<email>:<token>)`.
fn basic_credentials(
    params: &BTreeMap<String, String>,
    secret: &AppSecret,
) -> Result<Zeroizing<String>, AppsError> {
    let email = params.get("email").ok_or(AppsError::Invalid(EMAIL_SHAPE))?;
    let plain = Zeroizing::new(format!("{email}:{}", secret.token.as_str()));
    Ok(Zeroizing::new(
        base64::engine::general_purpose::STANDARD.encode(plain.as_bytes()),
    ))
}

/// Every string a reply could echo a secret in, at least
/// [`MIN_SECRET_BYTES`] long: the token, the key, the whole auth value and
/// Basic's `email:token` and base64, each also JSON-escaped (`/` as `\/`)
/// and percent-encoded (upper- and lower-case hex, and form encoding) (F8).
pub(crate) fn secret_strings(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
    secret: &AppSecret,
) -> Vec<Zeroizing<String>> {
    let mut plain = vec![secret.token.clone()];
    plain.extend(secret.key.clone());
    if let Ok((_, value)) = auth_header(connector, params, secret) {
        plain.push(value);
    }
    if connector.auth == AppAuth::Basic {
        if let Some(email) = params.get("email") {
            plain.push(Zeroizing::new(format!("{email}:{}", secret.token.as_str())));
        }
        plain.extend(basic_credentials(params, secret).ok());
    }
    let mut all = Vec::with_capacity(plain.len() * 5);
    for text in plain {
        let upper = Zeroizing::new(utf8_percent_encode(&text, COMPONENT).to_string());
        all.push(Zeroizing::new(lower_escapes(&upper)));
        all.push(upper);
        all.push(Zeroizing::new(
            url::form_urlencoded::byte_serialize(text.as_bytes()).collect(),
        ));
        all.push(Zeroizing::new(text.replace('/', "\\/")));
        all.push(text);
    }
    all.retain(|text| text.len() >= MIN_SECRET_BYTES);
    all.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    all.dedup_by(|a, b| a.as_str() == b.as_str());
    all
}

/// `%2F` as `%2f`.
fn lower_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut hex_left = 0;
    for c in text.chars() {
        if hex_left > 0 {
            out.push(c.to_ascii_lowercase());
            hex_left -= 1;
        } else {
            if c == '%' {
                hex_left = 2;
            }
            out.push(c);
        }
    }
    out
}

/// Replaces every occurrence of each non-empty secret with `[redacted]`,
/// longest first, so a whole auth value goes as one.
pub(crate) fn scrub(text: &str, secrets: &[&str]) -> String {
    let mut ordered: Vec<&str> = secrets
        .iter()
        .copied()
        .filter(|secret| !secret.is_empty())
        .collect();
    ordered.sort_by_key(|secret| Reverse(secret.len()));
    let mut out = text.to_owned();
    for secret in ordered {
        if out.contains(secret) {
            out = out.replace(secret, REDACTED);
        }
    }
    out
}

/// [`scrub`] on every string value and object key.
fn scrub_value(value: Value, secrets: &[&str]) -> Value {
    match value {
        Value::String(text) => Value::String(scrub(&text, secrets)),
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| scrub_value(item, secrets))
                .collect(),
        ),
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, item)| (scrub(&key, secrets), scrub_value(item, secrets)))
                .collect(),
        ),
        other => other,
    }
}

/// Reads at most [`APP_READ_MAX_BYTES`], then scrubs the raw text, parses
/// it, scrubs the parsed JSON again and cuts what's over
/// [`APP_RESPONSE_MAX_BYTES`].
async fn read_reply(
    mut response: reqwest::Response,
    secrets: &[&str],
) -> Result<AppResponse, AppsError> {
    let status = response.status().as_u16();
    let headers = kept_headers(response.headers(), secrets);
    let too_large = AppsError::TooLarge(APP_TOO_LARGE);
    let limit = u64::try_from(APP_READ_MAX_BYTES).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(too_large);
    }
    // The raw reply may hold the token, so it's wiped when dropped.
    let mut raw = Zeroizing::new(Vec::new());
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if raw.len().saturating_add(chunk.len()) > APP_READ_MAX_BYTES {
            return Err(too_large);
        }
        raw.extend_from_slice(&chunk);
    }
    let text = std::str::from_utf8(&raw).map_err(|_| AppsError::Invalid(APP_NOT_TEXT))?;
    Ok(reply(status, headers, text, secrets))
}

fn reply(
    status: u16,
    headers: BTreeMap<String, String>,
    text: &str,
    secrets: &[&str],
) -> AppResponse {
    let scrubbed = scrub(text, secrets);
    let (shown, parsed) = match serde_json::from_str::<Value>(&scrubbed) {
        Ok(value) => {
            let clean = scrub_value(value, secrets);
            (clean.to_string(), Some(clean))
        }
        Err(_) => (scrubbed, None),
    };
    if shown.len() <= APP_RESPONSE_MAX_BYTES {
        return AppResponse {
            status,
            headers,
            body: parsed.unwrap_or(Value::String(shown)),
            truncated: false,
            note: None,
        };
    }
    let mut end = APP_RESPONSE_MAX_BYTES;
    while !shown.is_char_boundary(end) {
        end -= 1;
    }
    AppResponse {
        status,
        headers,
        body: Value::String(shown[..end].to_owned()),
        truncated: true,
        note: Some(APP_CUT_NOTE.to_owned()),
    }
}

/// Only [`RESPONSE_HEADERS`], each scrubbed, repeated values joined.
fn kept_headers(headers: &HeaderMap, secrets: &[&str]) -> BTreeMap<String, String> {
    RESPONSE_HEADERS
        .iter()
        .filter_map(|name| {
            let values: Vec<&str> = headers
                .get_all(*name)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .collect();
            let joined = values.join(", ");
            (!values.is_empty() && joined.len() <= MAX_HEADER_BYTES)
                .then(|| ((*name).to_owned(), scrub(&joined, secrets)))
        })
        .collect()
}

/// Fixed copy for a failed request: never reqwest's text, which names the
/// address.
fn transport_error(error: reqwest::Error) -> AppsError {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = cause {
        if current.is::<HostRefused>() {
            return AppsError::Refused(APP_HOST_REFUSED);
        }
        cause = current.source();
    }
    if error.is_timeout() {
        AppsError::TimedOut(APP_TIMED_OUT)
    } else {
        AppsError::Unreachable(APP_UNREACHABLE)
    }
}

/// Whether an app host's answer may not be called: not public
/// ([`crate::net::public_ip`]) or this computer (ruling F7).
pub(crate) fn refused_address(ip: IpAddr) -> bool {
    !crate::net::public_ip(ip) || crate::local_tools::is_this_computer(ip)
}

/// The resolver's refusal of an undeclared host or a non-public answer.
#[derive(Debug)]
struct HostRefused;

impl std::fmt::Display for HostRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("refused app host")
    }
}

impl std::error::Error for HostRefused {}

/// Answers only for a declared host ([`host_declared`], checked before any
/// lookup), and refuses the whole answer when any address is
/// [`refused_address`]: never a filtered list, since a public SaaS host
/// never resolves here and such an answer is tampering.
pub(crate) struct GuardedResolver {
    lookup: Lookup,
}

impl GuardedResolver {
    fn system() -> Self {
        Self::with_lookup(|name: String| async move {
            Ok(tokio::net::lookup_host((name.as_str(), 443))
                .await?
                .collect())
        })
    }

    fn with_lookup<F, Fut>(lookup: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'static,
    {
        GuardedResolver {
            lookup: Arc::new(move |name| Box::pin(lookup(name))),
        }
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let name = name.as_str().to_owned();
        let lookup = Arc::clone(&self.lookup);
        Box::pin(async move {
            if !host_declared(&name) {
                return Err(Box::new(HostRefused) as BoxError);
            }
            let addresses = lookup(name).await?;
            if addresses.is_empty() {
                return Err(io::Error::from(io::ErrorKind::NotFound).into());
            }
            if addresses
                .iter()
                .any(|address| refused_address(address.ip()))
            {
                return Err(Box::new(HostRefused) as BoxError);
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
