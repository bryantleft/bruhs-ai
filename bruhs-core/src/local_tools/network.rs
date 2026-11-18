//! The network guard: an HTTP proxy on loopback that every bruh browser goes
//! through (Decision D7). It refuses this computer by name and by every
//! address a name resolves to, then connects to the address it checked, so
//! DNS rebinding can't slip in between the check and the connection. LAN
//! addresses pass, as in the shell rule. It chains to the owner's http://
//! upstream proxy and honours `no_proxy`.
//!
//! The guard asks no credential of its clients. Chrome can't carry proxy
//! credentials in `--proxy-server`; it would answer a 407 challenge only
//! through a CDP `Fetch.authRequired` handler on every page and worker target,
//! and a target it missed would fail to load. A local process that finds the
//! port gains nothing it lacks: the guard refuses this computer, and any local
//! process can reach the LAN and the internet itself. Web pages in the owner's
//! own browser can't drive it either: fetch can't send `CONNECT` or an
//! absolute-form target, and origin-form requests answer 400.

use std::{
    convert::Infallible,
    fmt, io,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use base64::Engine as _;
use bruhs_models::crew::{is_local_host, is_local_ip, LOCAL_HOST_DENIAL};
use futures_util::future::BoxFuture;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{
    body::{Bytes, Incoming},
    header::{self, HeaderMap, HeaderName, HeaderValue},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode, Uri,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf},
    net::{TcpListener, TcpStream},
    sync::{watch, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::{Instant, Sleep},
};

use super::ToolError;

pub(super) const SOCKS_UNSUPPORTED: &str =
    "Bruhs can't use a SOCKS or HTTPS proxy for a bruh's browser yet. Set HTTPS_PROXY to an http:// proxy.";

/// The largest request head the guard takes; a longer one answers 431.
/// hyper's read buffer stops a head at about this size, and [`head_size`]
/// holds it to exactly this.
const MAX_HEAD: usize = 16 * 1024;
/// Open client connections, tunnels included. Chrome keeps at most 32
/// sockets per proxy, and at most `MAX_COMPUTERS` (4) browsers share a guard.
const MAX_CONNECTIONS: usize = 256;
const HEAD_TIMEOUT: Duration = Duration::from_secs(15);
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
/// A connection or tunnel with no bytes either way for this long is closed.
const IDLE: Duration = Duration::from_secs(300);
/// The proxy port curl assumes when a proxy URL names none.
const DEFAULT_PROXY_PORT: u16 = 1080;

const NOT_A_SERVER: &str = "This is the proxy for a bruh's browser.";
const UNREACHABLE: &str = "Bruhs couldn't reach that site.";
const TOO_LARGE: &str = "This request is too large.";

/// Hop-by-hop headers, dropped in both directions.
const HOP_HEADERS: [&str; 9] = [
    "connection",
    "proxy-connection",
    "keep-alive",
    "proxy-authorization",
    "proxy-authenticate",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// An upstream http:// proxy.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct ProxyUrl {
    pub host: String,
    pub port: u16,
    /// The `Basic …` header value from the URL's credentials.
    pub authorization: Option<String>,
}

impl fmt::Debug for ProxyUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyUrl")
            .field("host", &self.host)
            .field("port", &self.port)
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl ProxyUrl {
    /// Reads a proxy URL as curl does: no scheme means `http://`, no port
    /// means 1080. Only `http://` works.
    pub(super) fn parse(name: &str, value: &str) -> Result<Self, ToolError> {
        let value = value.trim();
        let (scheme, rest) = value.split_once("://").unwrap_or(("http", value));
        if !scheme.eq_ignore_ascii_case("http") {
            return Err(ToolError::Unavailable(SOCKS_UNSUPPORTED.into()));
        }
        let unreadable =
            || ToolError::Unavailable(format!("Bruhs can't read the proxy address in {name}."));
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let (userinfo, host_port) = match authority.rsplit_once('@') {
            Some((userinfo, host_port)) => (Some(userinfo), host_port),
            None => (None, authority),
        };
        let (host, port) = split_host_port(host_port).ok_or_else(unreadable)?;
        let port = match port {
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(unreadable)?,
            None => DEFAULT_PROXY_PORT,
        };
        let authorization = userinfo.map(|userinfo| {
            let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
            let decode = |part: &str| {
                percent_encoding::percent_decode_str(part)
                    .decode_utf8_lossy()
                    .into_owned()
            };
            let credentials = format!("{}:{}", decode(user), decode(password));
            format!(
                "Basic {}",
                base64::engine::general_purpose::STANDARD.encode(credentials)
            )
        });
        Ok(Self {
            host: host.to_owned(),
            port,
            authorization,
        })
    }
}

/// `host`, `host:port`, `[v6]` or `[v6]:port`; the host is returned without
/// brackets.
fn split_host_port(text: &str) -> Option<(&str, Option<&str>)> {
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        host.parse::<std::net::Ipv6Addr>().ok()?;
        match after {
            "" => (host, None),
            _ => (host, Some(after.strip_prefix(':')?)),
        }
    } else {
        match text.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (text, None),
        }
    };
    let name_char = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':');
    (!host.is_empty() && host.chars().all(name_char)).then_some((host, port))
}

/// The owner's upstream proxies, read from the environment the way curl reads
/// it: lowercase wins, `all_proxy` fills in, `no_proxy` sends hosts direct.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Upstream {
    pub http: Option<ProxyUrl>,
    pub https: Option<ProxyUrl>,
    pub no_proxy: Vec<String>,
}

impl Upstream {
    pub(super) fn from_env() -> Result<Self, ToolError> {
        Self::from_vars(|key| std::env::var(key).ok())
    }

    /// A SOCKS or https upstream is an error ([`SOCKS_UNSUPPORTED`]): the
    /// browser doesn't start rather than skip the owner's proxy.
    pub(super) fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ToolError> {
        let var = |lower: &'static str, upper: &'static str| {
            [lower, upper].into_iter().find_map(|name| {
                get(name)
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| (name, value))
            })
        };
        let all = var("all_proxy", "ALL_PROXY");
        let proxy = |lower, upper| {
            var(lower, upper)
                .or_else(|| all.clone())
                .map(|(name, value)| ProxyUrl::parse(name, &value))
                .transpose()
        };
        Ok(Self {
            http: proxy("http_proxy", "HTTP_PROXY")?,
            https: proxy("https_proxy", "HTTPS_PROXY")?,
            no_proxy: var("no_proxy", "NO_PROXY")
                .map(|(_, value)| {
                    value
                        .split(',')
                        .map(|entry| normal_host(entry).trim_start_matches('.').to_owned())
                        .filter(|entry| !entry.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    /// The upstream for `host`, or `None` to connect directly. A `no_proxy`
    /// entry matches a host exactly or as a dot suffix; `*` matches all.
    pub(super) fn route(&self, host: &str, tls: bool) -> Option<&ProxyUrl> {
        let host = normal_host(host);
        let direct = self.no_proxy.iter().any(|entry| {
            entry == "*"
                || host == *entry
                || host
                    .strip_suffix(entry.as_str())
                    .is_some_and(|head| head.ends_with('.'))
        });
        if direct {
            None
        } else if tls {
            self.https.as_ref()
        } else {
            self.http.as_ref()
        }
    }
}

/// Lowercase, without brackets or a trailing dot.
fn normal_host(host: &str) -> String {
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Resolves a host name and port to addresses. Production uses
/// `tokio::net::lookup_host`.
pub(super) type Resolver =
    Arc<dyn Fn(String, u16) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> + Send + Sync>;

/// Whether to refuse a destination, given its name and the addresses it
/// resolved to (none yet on the first check). Production is [`refused`].
pub(super) type Refusal = Arc<dyn Fn(&str, &[SocketAddr]) -> bool + Send + Sync>;

/// The guard's rule: this computer by name ([`is_local_host`]) or any
/// resolved address on it ([`is_local_ip`], plus the deprecated
/// IPv4-compatible IPv6 form of one).
pub(super) fn refused(host: &str, addrs: &[SocketAddr]) -> bool {
    is_local_host(host) || addrs.iter().any(|addr| local_address(addr.ip()))
}

fn local_address(ip: IpAddr) -> bool {
    is_local_ip(ip)
        || match ip {
            IpAddr::V6(ip) => ip.to_ipv4().is_some_and(|ip| is_local_ip(ip.into())),
            IpAddr::V4(_) => false,
        }
}

fn system_resolver() -> Resolver {
    Arc::new(|host: String, port: u16| {
        Box::pin(async move {
            Ok(tokio::net::lookup_host((host.as_str(), port))
                .await?
                .collect())
        })
    })
}

/// The running guard. Dropping it stops the listener and every tunnel.
pub(super) struct NetworkGuard {
    addr: SocketAddr,
    accept: JoinHandle<()>,
    /// Never sent; every connection task ends when it drops.
    _shutdown: watch::Sender<()>,
}

impl Drop for NetworkGuard {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

struct Shared {
    upstream: Upstream,
    resolver: Resolver,
    refuse: Refusal,
    shutdown: watch::Receiver<()>,
}

impl Shared {
    /// Runs `task` until it ends or the guard drops.
    fn spawn(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            tokio::select! {
                () = task => {}
                _ = shutdown.changed() => {}
            }
        });
    }
}

impl NetworkGuard {
    /// Starts the guard on `127.0.0.1` with the system resolver and the
    /// production rule.
    pub(super) fn start(upstream: Upstream) -> Result<Self, ToolError> {
        Self::start_with(upstream, system_resolver(), Arc::new(refused))
    }

    pub(super) fn start_with(
        upstream: Upstream,
        resolver: Resolver,
        refuse: Refusal,
    ) -> Result<Self, ToolError> {
        tokio::runtime::Handle::try_current().map_err(|_| {
            ToolError::Unavailable("The network guard needs a Tokio runtime.".into())
        })?;
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let listener = TcpListener::from_std(listener)?;
        let addr = listener.local_addr()?;
        let (shutdown, receiver) = watch::channel(());
        let shared = Arc::new(Shared {
            upstream,
            resolver,
            refuse,
            shutdown: receiver,
        });
        let accept = tokio::spawn(accept(listener, shared));
        Ok(Self {
            addr,
            accept,
            _shutdown: shutdown,
        })
    }

    pub(super) fn addr(&self) -> SocketAddr {
        self.addr
    }
}

async fn accept(listener: TcpListener, shared: Arc<Shared>) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let Ok(permit) = slots.clone().acquire_owned().await else {
            return;
        };
        let socket = match listener.accept().await {
            Ok((socket, _)) => socket,
            Err(error) => {
                // Out of descriptors and the like: wait rather than spin or stop.
                tracing::warn!(%error, "network guard accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let _ = socket.set_nodelay(true);
        let connection = serve(socket, shared.clone(), Arc::new(permit));
        shared.spawn(connection);
    }
}

async fn serve(socket: TcpStream, shared: Arc<Shared>, permit: Arc<OwnedSemaphorePermit>) {
    let service = service_fn(move |request| {
        let (shared, permit) = (shared.clone(), permit.clone());
        async move {
            let response = match handle(request, &shared, permit).await {
                Ok(response) => response,
                Err(failure) => failure.response(),
            };
            Ok::<_, Infallible>(response)
        }
    });
    let _ = http1::Builder::new()
        .timer(TokioTimer::new())
        .header_read_timeout(HEAD_TIMEOUT)
        .max_buf_size(MAX_HEAD)
        .serve_connection(TokioIo::new(Idle::new(socket)), service)
        .with_upgrades()
        .await;
}

type ProxyBody = BoxBody<Bytes, hyper::Error>;

/// A response the guard gives itself.
struct Failure(StatusCode, &'static str);

impl Failure {
    fn denied() -> Self {
        Self(StatusCode::FORBIDDEN, LOCAL_HOST_DENIAL)
    }

    fn bad_request() -> Self {
        Self(StatusCode::BAD_REQUEST, NOT_A_SERVER)
    }

    fn unreachable() -> Self {
        Self(StatusCode::BAD_GATEWAY, UNREACHABLE)
    }

    fn timed_out() -> Self {
        Self(StatusCode::GATEWAY_TIMEOUT, UNREACHABLE)
    }

    fn response(self) -> Response<ProxyBody> {
        let mut response = Response::new(full(self.1));
        *response.status_mut() = self.0;
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        response
    }
}

fn full(text: &'static str) -> ProxyBody {
    Full::new(Bytes::from_static(text.as_bytes()))
        .map_err(|never| match never {})
        .boxed()
}

async fn handle(
    request: Request<Incoming>,
    shared: &Arc<Shared>,
    permit: Arc<OwnedSemaphorePermit>,
) -> Result<Response<ProxyBody>, Failure> {
    if head_size(&request) > MAX_HEAD {
        return Err(Failure(
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            TOO_LARGE,
        ));
    }
    if request.method() == Method::CONNECT {
        return tunnel(request, shared, permit).await;
    }
    forward(request, shared, permit).await
}

/// About the bytes of a request head: its target and every header line.
fn head_size<B>(request: &Request<B>) -> usize {
    let target = request.uri().to_string().len();
    let headers: usize = request
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len() + value.len() + 4)
        .sum();
    target + headers
}

/// Where a checked destination goes.
enum Route {
    /// Connect to these addresses, each already checked.
    Direct(Vec<SocketAddr>),
    /// Hand the name to the owner's upstream.
    Upstream(ProxyUrl),
}

/// Checks a destination by name, resolves it, and checks every address.
/// Through an upstream the name is still resolved here when it can be:
/// refused if any address is local, passed on by name if it doesn't resolve.
async fn check(shared: &Shared, host: &str, port: u16, tls: bool) -> Result<Route, Failure> {
    if (shared.refuse)(host, &[]) {
        return Err(Failure::denied());
    }
    let name = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    let resolved = resolve(&shared.resolver, name, port).await;
    if let Ok(addrs) = &resolved {
        if (shared.refuse)(host, addrs) {
            return Err(Failure::denied());
        }
    }
    match (shared.upstream.route(name, tls), resolved) {
        (Some(proxy), _) => Ok(Route::Upstream(proxy.clone())),
        (None, Ok(addrs)) => Ok(Route::Direct(addrs)),
        (None, Err(failure)) => Err(failure),
    }
}

async fn resolve(resolver: &Resolver, name: &str, port: u16) -> Result<Vec<SocketAddr>, Failure> {
    if let Ok(ip) = name.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }
    match tokio::time::timeout(RESOLVE_TIMEOUT, resolver(name.to_owned(), port)).await {
        Ok(Ok(addrs)) => Ok(addrs
            .into_iter()
            .map(|addr| SocketAddr::new(addr.ip(), port))
            .collect()),
        Ok(Err(_)) => Err(Failure::unreachable()),
        Err(_) => Err(Failure::timed_out()),
    }
}

/// Connects to the first of `addrs` that answers. These are the addresses
/// that were checked; nothing is resolved again.
async fn connect(addrs: &[SocketAddr]) -> Result<TcpStream, Failure> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    let mut failure = Failure::unreachable();
    for addr in addrs {
        match tokio::time::timeout_at(deadline, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                let _ = stream.set_nodelay(true);
                return Ok(stream);
            }
            Ok(Err(_)) => {}
            Err(_) => {
                failure = Failure::timed_out();
                break;
            }
        }
    }
    Err(failure)
}

/// The owner's upstream is not checked: the owner configured it, not a page.
async fn connect_upstream(shared: &Shared, proxy: &ProxyUrl) -> Result<TcpStream, Failure> {
    connect(&resolve(&shared.resolver, &proxy.host, proxy.port).await?).await
}

/// `CONNECT host:port`, for https and WebSockets.
async fn tunnel(
    request: Request<Incoming>,
    shared: &Arc<Shared>,
    permit: Arc<OwnedSemaphorePermit>,
) -> Result<Response<ProxyBody>, Failure> {
    let authority = request.uri().authority().ok_or_else(Failure::bad_request)?;
    let port = authority.port_u16().ok_or_else(Failure::bad_request)?;
    let host = authority.host().to_owned();
    let (stream, early) = match check(shared, &host, port, true).await? {
        Route::Direct(addrs) => (connect(&addrs).await?, Vec::new()),
        Route::Upstream(proxy) => upstream_connect(shared, &proxy, &host, port).await?,
    };
    shared.spawn(async move {
        let _permit = permit;
        let Ok(upgraded) = hyper::upgrade::on(request).await else {
            return;
        };
        let mut client = TokioIo::new(upgraded);
        let mut stream = Idle::new(stream);
        if !early.is_empty() && client.write_all(&early).await.is_err() {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut stream).await;
    });
    Ok(Response::new(
        Full::new(Bytes::new())
            .map_err(|never| match never {})
            .boxed(),
    ))
}

/// Opens a tunnel through the owner's upstream. Returns the stream and any
/// bytes the upstream sent after its `200`, which belong to the tunnel.
async fn upstream_connect(
    shared: &Shared,
    proxy: &ProxyUrl,
    host: &str,
    port: u16,
) -> Result<(TcpStream, Vec<u8>), Failure> {
    let mut stream = connect_upstream(shared, proxy).await?;
    // `host` came from a parsed authority, so it holds no spaces or line breaks.
    let target = format!("{host}:{port}");
    let mut head = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
    if let Some(authorization) = &proxy.authorization {
        head.push_str(&format!("Proxy-Authorization: {authorization}\r\n"));
    }
    head.push_str("\r\n");
    let exchange = async {
        stream.write_all(head.as_bytes()).await?;
        read_response_head(&mut stream).await
    };
    let (status, early) = match tokio::time::timeout(RESPONSE_TIMEOUT, exchange).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(_)) => return Err(Failure::unreachable()),
        Err(_) => return Err(Failure::timed_out()),
    };
    if !(200..300).contains(&status) {
        return Err(Failure::unreachable());
    }
    Ok((stream, early))
}

/// Reads an HTTP response head of at most [`MAX_HEAD`] bytes. Returns its
/// status and the bytes read past it.
async fn read_response_head(stream: &mut TcpStream) -> io::Result<(u16, Vec<u8>)> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid proxy response");
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    let end = loop {
        if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
        if buffer.len() > MAX_HEAD {
            return Err(invalid());
        }
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    let status = std::str::from_utf8(&buffer[..end])
        .ok()
        .and_then(|head| head.split_whitespace().nth(1))
        .and_then(|status| status.parse().ok())
        .ok_or_else(invalid)?;
    Ok((status, buffer.split_off(end)))
}

/// An absolute-form `http://` request: checked like a tunnel, then sent
/// origin-form to the checked address, or absolute-form to the upstream.
async fn forward(
    mut request: Request<Incoming>,
    shared: &Arc<Shared>,
    permit: Arc<OwnedSemaphorePermit>,
) -> Result<Response<ProxyBody>, Failure> {
    let uri = request.uri().clone();
    let authority = match (uri.scheme_str(), uri.authority()) {
        (Some(scheme), Some(authority)) if scheme.eq_ignore_ascii_case("http") => authority.clone(),
        _ => return Err(Failure::bad_request()),
    };
    let host = authority.host();
    let port = authority.port_u16().unwrap_or(80);
    strip_hop_headers(request.headers_mut());
    let stream = match check(shared, host, port, false).await? {
        Route::Direct(addrs) => {
            let stream = connect(&addrs).await?;
            let path = uri.path_and_query().map_or("/", |path| path.as_str());
            *request.uri_mut() = path.parse::<Uri>().map_err(|_| Failure::bad_request())?;
            if !request.headers().contains_key(header::HOST) {
                let value = match authority.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                };
                let value = HeaderValue::from_str(&value).map_err(|_| Failure::bad_request())?;
                request.headers_mut().insert(header::HOST, value);
            }
            stream
        }
        Route::Upstream(proxy) => {
            let stream = connect_upstream(shared, &proxy).await?;
            if let Some(authorization) = &proxy.authorization {
                let value =
                    HeaderValue::from_str(authorization).map_err(|_| Failure::unreachable())?;
                request
                    .headers_mut()
                    .insert(header::PROXY_AUTHORIZATION, value);
            }
            stream
        }
    };
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(Idle::new(stream)))
            .await
            .map_err(|_| Failure::unreachable())?;
    shared.spawn(async move {
        let _permit = permit;
        let _ = connection.await;
    });
    let response = match tokio::time::timeout(RESPONSE_TIMEOUT, sender.send_request(request)).await
    {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => return Err(Failure::unreachable()),
        Err(_) => return Err(Failure::timed_out()),
    };
    let (mut parts, body) = response.into_parts();
    strip_hop_headers(&mut parts.headers);
    Ok(Response::from_parts(parts, body.boxed()))
}

/// Drops hop-by-hop headers and any header `Connection` names.
fn strip_hop_headers(headers: &mut HeaderMap) {
    let named: Vec<HeaderName> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect();
    for name in named {
        headers.remove(name);
    }
    for name in HOP_HEADERS {
        headers.remove(name);
    }
}

/// Closes a connection with no bytes either way for [`IDLE`].
struct Idle<T> {
    inner: T,
    timer: Pin<Box<Sleep>>,
}

impl<T> Idle<T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            timer: Box::pin(tokio::time::sleep(IDLE)),
        }
    }

    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        use std::future::Future;
        if self.timer.as_mut().poll(cx).is_ready() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "network guard connection idle",
            ))
        } else {
            Ok(())
        }
    }

    fn touched(&mut self) {
        self.timer.as_mut().reset(Instant::now() + IDLE);
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for Idle<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            this.touched();
        }
        result
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Idle<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.check(cx)?;
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            this.touched();
        }
        result
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
#[path = "network_tests.rs"]
mod tests;
