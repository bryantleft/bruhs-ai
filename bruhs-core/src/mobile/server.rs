//! Isolated HTTPS surface. Never mounts companion-server routes or trusts proxy identity.
mod io;
#[cfg(test)]
mod tests;
use super::{
    authority::*, MobileBroker, MobileError, MobilePending, MobileResult, REQUEST_TIMEOUT,
};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Router,
};
use bruhs_models::mobile::*;
use futures_util::stream;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{
    collections::BTreeMap, convert::Infallible, net::SocketAddr, path::PathBuf, sync::Arc,
    time::Duration,
};
use tokio::{
    net::TcpListener,
    sync::{mpsc, Semaphore},
    task::{JoinHandle, JoinSet},
};
use tokio_rustls::{rustls, TlsAcceptor};
use tower::ServiceExt;
use url::Url;

pub enum MobileTransport {
    /// Only a loopback bind is accepted; an existing trusted HTTPS proxy owns TLS.
    HttpsProxy,
    DirectTls {
        certificate: PathBuf,
        private_key: PathBuf,
    },
}
pub struct MobileHostConfig {
    pub bind: SocketAddr,
    pub origin: Url,
    pub transport: MobileTransport,
}
pub struct MobileHost {
    authority: MobileAuthority,
    origin: Url,
    address: SocketAddr,
    worker: JoinHandle<()>,
}
#[derive(Clone)]
struct HttpState {
    authority: MobileAuthority,
    broker: MobileBroker,
    origin: String,
    host: String,
    proxy_host: Option<String>,
}
impl MobileHost {
    pub async fn start(
        config: MobileHostConfig,
        authority: MobileAuthority,
    ) -> MobileResult<(Self, mpsc::Receiver<MobilePending>)> {
        validate_config(&config)?;
        authority.healthy()?;
        let tls = match &config.transport {
            MobileTransport::HttpsProxy => None,
            MobileTransport::DirectTls {
                certificate,
                private_key,
            } => {
                let cert = certificate.clone();
                let key = private_key.clone();
                Some(
                    tokio::task::spawn_blocking(move || load_tls(&cert, &key))
                        .await
                        .map_err(|_| config_error())??,
                )
            }
        };
        let listener = TcpListener::bind(config.bind).await.map_err(|_| {
            MobileError::Invalid(
                "The mobile address cannot be opened. Check its interface and port.".into(),
            )
        })?;
        let address = listener.local_addr().map_err(|_| config_error())?;
        let mut origin = config.origin;
        if origin.port() == Some(0) {
            origin
                .set_port(Some(address.port()))
                .map_err(|()| config_error())?;
        }
        let (broker, requests) = MobileBroker::new();
        let router = {
            let state = HttpState {
                authority: authority.clone(),
                broker,
                host: authority_host(&origin),
                origin: origin.origin().ascii_serialization(),
                proxy_host: matches!(config.transport, MobileTransport::HttpsProxy)
                    .then(|| address.to_string()),
            };
            Router::new().fallback(dispatch).with_state(state)
        };
        let worker = tokio::spawn(serve(listener, router, tls));
        Ok((
            Self {
                authority,
                origin,
                address,
                worker,
            },
            requests,
        ))
    }
    pub fn authority(&self) -> MobileAuthority {
        self.authority.clone()
    }
    pub fn origin(&self) -> &Url {
        &self.origin
    }
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    pub fn stop_now(&self) {
        self.authority.disable();
        self.worker.abort();
    }
    pub async fn shutdown(self) {
        self.stop_now();
        self.authority.release().await;
    }
}
impl Drop for MobileHost {
    fn drop(&mut self) {
        self.stop_now();
    }
}
fn validate_config(config: &MobileHostConfig) -> MobileResult<()> {
    let u = &config.origin;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(MobileError::Invalid(
            "Enter the trusted HTTPS origin, without a path, credentials, query or fragment."
                .into(),
        ));
    }
    if matches!(config.transport, MobileTransport::HttpsProxy) && !config.bind.ip().is_loopback() {
        return Err(MobileError::Invalid("HTTPS proxy mode must bind to a loopback address. Use direct TLS to bind a network interface.".into()));
    }
    Ok(())
}
fn config_error() -> MobileError {
    MobileError::Invalid("The TLS certificate or key could not be loaded. Supply PEM files for a certificate your phone trusts.".into())
}
fn load_tls(cert: &std::path::Path, key: &std::path::Path) -> MobileResult<TlsAcceptor> {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    use std::io::Read;
    fn bounded(path: &std::path::Path) -> MobileResult<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK);
        }
        let file = options.open(path).map_err(|_| config_error())?;
        if !file.metadata().map_err(|_| config_error())?.is_file() {
            return Err(config_error());
        }
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| config_error())?;
        if bytes.len() > 1024 * 1024 {
            return Err(config_error());
        }
        Ok(bytes)
    }
    let certificate = bounded(cert)?;
    let private_key = zeroize::Zeroizing::new(bounded(key)?);
    let certificates = CertificateDer::pem_slice_iter(&certificate)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| config_error())?;
    let key = PrivateKeyDer::from_pem_slice(&private_key).map_err(|_| config_error())?;
    let server = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| config_error())?
    .with_no_client_auth()
    .with_single_cert(certificates, key)
    .map_err(|_| config_error())?;
    Ok(TlsAcceptor::from(Arc::new(server)))
}
fn authority_host(origin: &Url) -> String {
    let mut host = origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
    if origin.port() == Some(443) {
        host = origin[url::Position::BeforeHost..url::Position::AfterHost].to_owned();
    }
    host
}
async fn serve(listener: TcpListener, router: Router, tls: Option<TlsAcceptor>) {
    // One stream and one RPC per maximum device, plus the native broker's backlog.
    let connections = Arc::new(Semaphore::new(PAGE_SIZE * 2 + super::BROKER_CAPACITY));
    let handshakes = Arc::new(Semaphore::new(super::BROKER_CAPACITY));
    let mut tasks = JoinSet::new();
    loop {
        let permit = match connections.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let Ok((socket, _)) = listener.accept().await else {
            break;
        };
        let _ = socket.set_nodelay(true);
        let app = router.clone();
        let tls = tls.clone();
        let gate = handshakes.clone();
        tasks.spawn(async move {
            let _permit = permit;
            let socket = io::IdleIo::new(socket);
            if let Some(tls) = tls {
                let handshake = async {
                    let _slot = gate.acquire_owned().await.ok()?;
                    tls.accept(socket).await.ok()
                };
                if let Ok(Some(socket)) = tokio::time::timeout(REQUEST_TIMEOUT, handshake).await {
                    serve_connection(socket, app).await;
                }
            } else {
                serve_connection(socket, app).await;
            }
        });
        while tasks.try_join_next().is_some() {}
    }
}
async fn serve_connection<T>(socket: T, app: Router)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper::service::service_fn(move |request| {
        let app = app.clone();
        async move { app.oneshot(request.map(Body::new)).await }
    });
    let _ = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(REQUEST_TIMEOUT)
        .max_buf_size(REQUEST_BYTES)
        .keep_alive(false)
        .serve_connection(TokioIo::new(socket), service)
        .await;
}
async fn dispatch(State(state): State<HttpState>, request: Request) -> Response {
    let mut response = match handle(&state, request).await {
        Ok(reply) => reply,
        Err(error) => error_response(error),
    };
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("content-security-policy", HeaderValue::from_static("default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"));
    response
}
fn one_header<'a>(headers: &'a HeaderMap, name: &str) -> MobileResult<Option<&'a str>> {
    let mut values = headers.get_all(name).iter();
    let first = values.next();
    if values.next().is_some() {
        return Err(MobileError::Unauthorized);
    }
    first
        .map(|value| value.to_str().map_err(|_| MobileError::Unauthorized))
        .transpose()
}
fn check_origin(state: &HttpState, request: &Request) -> MobileResult<()> {
    let host = one_header(request.headers(), "host")?.ok_or(MobileError::Unauthorized)?;
    if host != state.host && state.proxy_host.as_deref() != Some(host) {
        return Err(MobileError::Unauthorized);
    }
    if let Some(origin) = one_header(request.headers(), "origin")? {
        if origin != state.origin {
            return Err(MobileError::Unauthorized);
        }
    }
    // Never interpret ambiguous Authorization, even on a public bootstrap route.
    one_header(request.headers(), "authorization")?;
    if request.uri().to_string().len() > REQUEST_BYTES {
        return Err(MobileError::Invalid("The request URL is too long.".into()));
    }
    Ok(())
}
fn lease(state: &HttpState, request: &Request) -> MobileResult<MobileLease> {
    let token = one_header(request.headers(), "authorization")?
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(MobileError::Unauthorized)?;
    state.authority.authenticate(token)
}
fn json<T: serde::Serialize>(value: &T) -> MobileResult<Response> {
    let bytes = serde_json::to_vec(value).map_err(|_| MobileError::Unavailable)?;
    if bytes.len() > RESPONSE_BYTES {
        return Err(MobileError::Invalid(
            "This response is too large. Read it on the laptop.".into(),
        ));
    }
    Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
}
fn error_response(error: MobileError) -> Response {
    let status = match error {
        MobileError::Unauthorized | MobileError::Disabled => StatusCode::UNAUTHORIZED,
        MobileError::Forbidden => StatusCode::FORBIDDEN,
        MobileError::Conflict(_) => StatusCode::CONFLICT,
        MobileError::Invalid(_) => StatusCode::BAD_REQUEST,
        MobileError::Storage | MobileError::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        axum::Json(serde_json::json!({"error": error.to_string()})),
    )
        .into_response()
}
async fn body<T: serde::de::DeserializeOwned>(request: Request) -> MobileResult<T> {
    if one_header(request.headers(), "content-type")?
        .is_none_or(|s| !s.eq_ignore_ascii_case("application/json"))
    {
        return Err(MobileError::Invalid(
            "Use an application/json request.".into(),
        ));
    }
    let bytes = tokio::time::timeout(
        REQUEST_TIMEOUT,
        to_bytes(request.into_body(), REQUEST_BYTES),
    )
    .await
    .map_err(|_| MobileError::Unavailable)?
    .map_err(|_| MobileError::Invalid("The request body exceeds 64 KiB.".into()))?;
    serde_json::from_slice(&bytes)
        .map_err(|_| MobileError::Invalid("The mobile request is malformed.".into()))
}
fn query(request: &Request, allowed: &[&str]) -> MobileResult<BTreeMap<String, String>> {
    let mut result = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(request.uri().query().unwrap_or("").as_bytes())
    {
        if !allowed.contains(&key.as_ref())
            || result
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return Err(MobileError::Invalid(
                "Unknown or repeated mobile query field.".into(),
            ));
        }
    }
    Ok(result)
}
fn number(q: &BTreeMap<String, String>, key: &str) -> MobileResult<Option<usize>> {
    q.get(key)
        .map(|s| {
            s.parse()
                .map_err(|_| MobileError::Invalid(format!("Invalid {key}.")))
        })
        .transpose()
}
fn required(q: &BTreeMap<String, String>, key: &str) -> MobileResult<String> {
    q.get(key)
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .cloned()
        .ok_or_else(|| MobileError::Invalid(format!("Missing or invalid {key}.")))
}
async fn handle(state: &HttpState, request: Request) -> MobileResult<Response> {
    check_origin(state, &request)?;
    let path = request.uri().path();
    if request.method() == Method::GET {
        match path {
            "/" => {
                return Ok((
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    include_str!("web/index.html"),
                )
                    .into_response())
            }
            "/mobile.js" => {
                return Ok((
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("web/mobile.js"),
                )
                    .into_response())
            }
            "/mobile.css" => {
                return Ok((
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("web/mobile.css"),
                )
                    .into_response())
            }
            _ => {}
        }
    }
    if path == "/api/mobile/pair" && request.method() == Method::POST {
        let issued = state.authority.pair(body(request).await?).await?;
        return json(&issued);
    }
    let lease = lease(state, &request)?;
    match (request.method(), path) {
        (&Method::GET, "/api/mobile/session") => json(&state.authority.session(&lease)?),
        (&Method::GET, "/api/mobile/events") => {
            lease.check(MobileScope::Read)?;
            Ok(events(lease))
        }
        (&Method::POST, "/api/mobile/disconnect") => {
            state.authority.disconnect(&lease).await?;
            json(&serde_json::json!({"disconnected":true}))
        }
        (&Method::POST, "/api/mobile/command") => {
            json(&state.broker.command(lease, body(request).await?).await?)
        }
        (&Method::GET, "/api/mobile/threads") => {
            let q = query(&request, &["offset"])?;
            json(
                &state
                    .broker
                    .read(
                        lease,
                        MobileRead::Threads {
                            offset: number(&q, "offset")?.unwrap_or(0),
                        },
                    )
                    .await?,
            )
        }
        (&Method::GET, "/api/mobile/thread") => {
            let q = query(&request, &["thread_id", "before"])?;
            json(
                &state
                    .broker
                    .read(
                        lease,
                        MobileRead::Thread {
                            thread_id: required(&q, "thread_id")?,
                            before: number(&q, "before")?,
                        },
                    )
                    .await?,
            )
        }
        (&Method::GET, "/api/mobile/message") => {
            let q = query(
                &request,
                &["thread_id", "index", "reference", "offset", "revision"],
            )?;
            json(
                &state
                    .broker
                    .read(
                        lease,
                        MobileRead::Message {
                            thread_id: required(&q, "thread_id")?,
                            index: number(&q, "index")?.ok_or_else(|| {
                                MobileError::Invalid("Missing message index.".into())
                            })?,
                            reference: required(&q, "reference")?,
                            offset: number(&q, "offset")?.unwrap_or(0),
                            revision: q.get("revision").cloned(),
                        },
                    )
                    .await?,
            )
        }
        (&Method::GET, "/api/mobile/diff") => {
            let q = query(&request, &["thread_id", "offset", "revision"])?;
            json(
                &state
                    .broker
                    .read(
                        lease,
                        MobileRead::Diff {
                            thread_id: required(&q, "thread_id")?,
                            offset: number(&q, "offset")?.unwrap_or(0),
                            revision: q.get("revision").cloned(),
                        },
                    )
                    .await?,
            )
        }
        _ => Ok(StatusCode::NOT_FOUND.into_response()),
    }
}
fn events(lease: MobileLease) -> Response {
    let changes = lease.changes();
    let failures = lease.failures();
    let deadline = tokio::time::Instant::now() + lease.remaining().unwrap_or_default();
    let interval = tokio::time::interval(Duration::from_millis(REFRESH_MS));
    let events = stream::unfold(
        (lease, changes, failures, deadline, interval, false),
        |(lease, mut changes, mut failures, deadline, mut interval, closed)| async move {
            if closed {
                return None;
            }
            loop {
                if lease.check(MobileScope::Read).is_err() {
                    return Some((
                        Ok::<_, Infallible>(Event::default().event("revoked").data("{}")),
                        (lease, changes, failures, deadline, interval, true),
                    ));
                }
                tokio::select! {
                    _ = interval.tick() => return Some((Ok(Event::default().event("refresh").data("{}")), (lease, changes, failures, deadline, interval, false))),
                    _ = changes.changed() => {},
                    _ = failures.changed() => {},
                    _ = tokio::time::sleep_until(deadline) => {},
                }
            }
        },
    );
    Sse::new(events)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("awake"),
        )
        .into_response()
}
