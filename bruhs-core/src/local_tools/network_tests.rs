use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use super::*;

const WAIT: Duration = Duration::from_secs(5);

fn a(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

/// A resolver that knows only the names it is given.
fn resolver(names: &[(&str, &str)]) -> Resolver {
    let names: HashMap<String, Vec<IpAddr>> =
        names.iter().fold(HashMap::new(), |mut map, (name, ip)| {
            map.entry((*name).to_owned())
                .or_default()
                .push(ip.parse().unwrap());
            map
        });
    Arc::new(move |host: String, port: u16| {
        let found = names.get(&host).cloned();
        Box::pin(async move {
            found
                .map(|ips| {
                    ips.into_iter()
                        .map(|ip| SocketAddr::new(ip, port))
                        .collect()
                })
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, host))
        })
    })
}

/// Refuses every resolved address except the fixture's port, so the tunnel
/// and forward paths can run against a loopback fixture. The guard's own
/// rule is pinned by `refused`.
fn only_port(port: u16) -> Refusal {
    Arc::new(move |_host: &str, addrs: &[SocketAddr]| addrs.iter().any(|addr| addr.port() != port))
}

struct Fixture {
    port: u16,
    connections: Arc<AtomicUsize>,
    heads: Arc<Mutex<Vec<String>>>,
}

/// An HTTP server on loopback that records each request head and answers
/// `hello`, closing the connection.
async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(AtomicUsize::new(0));
    let heads = Arc::new(Mutex::new(Vec::new()));
    let (count, seen) = (connections.clone(), heads.clone());
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            let seen = seen.clone();
            tokio::spawn(async move {
                let head = read_head(&mut socket).await;
                seen.lock().unwrap().push(head);
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await;
            });
        }
    });
    Fixture {
        port,
        connections,
        heads,
    }
}

/// A fake upstream proxy: records each request head, accepts CONNECT with
/// `200` and then answers one request inside the tunnel, and answers a plain
/// request with `upstream`.
async fn upstream_proxy() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let seen = heads.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let head = read_head(&mut socket).await;
                let connect = head.starts_with("CONNECT ");
                seen.lock().unwrap().push(head);
                if connect {
                    socket
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await
                        .unwrap();
                    let inner = read_head(&mut socket).await;
                    seen.lock().unwrap().push(inner);
                    let _ = socket.write_all(b"tunnelled").await;
                } else {
                    let _ = socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nupstream")
                        .await;
                }
            });
        }
    });
    (port, heads)
}

async fn read_head(socket: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match tokio::time::timeout(WAIT, socket.read(&mut byte)).await {
            Ok(Ok(1)) => head.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

/// Sends `request` through the guard and returns the response head and the
/// body its `Content-Length` names.
async fn exchange(guard: SocketAddr, request: &str) -> (String, String) {
    let mut socket = TcpStream::connect(guard).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    response(&mut socket).await
}

async fn response(socket: &mut TcpStream) -> (String, String) {
    let head = read_head(socket).await;
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    tokio::time::timeout(WAIT, socket.read_exact(&mut body))
        .await
        .unwrap()
        .unwrap();
    (head, String::from_utf8(body).unwrap())
}

fn has_header(head: &str, name: &str) -> bool {
    head.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(key, _)| key.eq_ignore_ascii_case(name))
    })
}

#[test]
fn this_computer_is_refused_by_name_and_by_address() {
    assert!(refused("localhost", &[]));
    assert!(refused("app.localhost", &[]));
    assert!(refused("LOCALHOST.", &[]));
    assert!(refused("[::1]", &[]));
    assert!(refused("0x7f000001", &[]));
    assert!(refused(
        "rebind.test",
        &[a("93.184.216.34:443"), a("127.0.0.1:443")]
    ));
    assert!(refused("example.test", &[a("[::ffff:127.0.0.1]:80")]));
    assert!(refused("example.test", &[a("[::1]:80")]));
    assert!(refused("example.test", &[a("[::]:80")]));
    assert!(refused("example.test", &[a("0.0.0.0:80")]));
    assert!(refused("example.test", &[a("0.1.2.3:80")]));
    assert!(refused("example.test", &[a("127.9.9.9:80")]));
    // The deprecated IPv4-compatible form of loopback.
    assert!(refused("example.test", &[a("[::127.0.0.1]:80")]));
    assert!(!refused("example.test", &[a("93.184.216.34:443")]));
    assert!(!refused("example.test", &[a("[2606:2800:220:1::248]:443")]));
    assert!(!refused("printer.lan", &[a("192.168.1.20:80")]));
    assert!(!refused("nas.lan", &[a("10.0.0.2:80"), a("[fd00::2]:80")]));
}

#[tokio::test]
async fn a_redirect_or_subresource_to_this_computer_never_connects() {
    let fixture = fixture().await;
    let p = fixture.port;
    let guard = NetworkGuard::start_with(
        Upstream::default(),
        resolver(&[
            ("rebind.test", "127.0.0.1"),
            ("split.test", "93.184.216.34"),
            ("split.test", "::1"),
        ]),
        Arc::new(refused),
    )
    .unwrap();
    let requests = [
        format!("CONNECT 127.0.0.1:{p} HTTP/1.1\r\nHost: 127.0.0.1:{p}\r\n\r\n"),
        format!("CONNECT localhost:{p} HTTP/1.1\r\nHost: localhost:{p}\r\n\r\n"),
        format!("CONNECT [::ffff:127.0.0.1]:{p} HTTP/1.1\r\nHost: [::ffff:127.0.0.1]:{p}\r\n\r\n"),
        format!("GET http://[::1]:{p}/ HTTP/1.1\r\nHost: [::1]:{p}\r\n\r\n"),
        format!("GET http://0x7f000001:{p}/ HTTP/1.1\r\nHost: 0x7f000001:{p}\r\n\r\n"),
        format!("GET http://127.1:{p}/ HTTP/1.1\r\nHost: 127.1:{p}\r\n\r\n"),
        format!("CONNECT rebind.test:{p} HTTP/1.1\r\nHost: rebind.test:{p}\r\n\r\n"),
        format!("GET http://rebind.test:{p}/ HTTP/1.1\r\nHost: rebind.test:{p}\r\n\r\n"),
        format!("CONNECT split.test:{p} HTTP/1.1\r\nHost: split.test:{p}\r\n\r\n"),
    ];
    for request in &requests {
        let (head, body) = exchange(guard.addr(), request).await;
        assert!(head.starts_with("HTTP/1.1 403"), "{request}: {head}");
        assert_eq!(body, LOCAL_HOST_DENIAL, "{request}");
    }
    assert_eq!(fixture.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_upstream_route_still_refuses_names_that_resolve_here() {
    let fixture = fixture().await;
    let (upstream_port, heads) = upstream_proxy().await;
    let proxy = ProxyUrl {
        host: "127.0.0.1".into(),
        port: upstream_port,
        authorization: None,
    };
    let upstream = Upstream {
        http: Some(proxy.clone()),
        https: Some(proxy),
        no_proxy: Vec::new(),
    };
    let guard = NetworkGuard::start_with(
        upstream,
        resolver(&[("rebind.test", "127.0.0.1")]),
        Arc::new(refused),
    )
    .unwrap();
    let p = fixture.port;
    for request in [
        format!("CONNECT rebind.test:{p} HTTP/1.1\r\nHost: rebind.test:{p}\r\n\r\n"),
        format!("CONNECT localhost:{p} HTTP/1.1\r\nHost: localhost:{p}\r\n\r\n"),
        format!("GET http://rebind.test:{p}/ HTTP/1.1\r\nHost: rebind.test:{p}\r\n\r\n"),
    ] {
        let (head, body) = exchange(guard.addr(), &request).await;
        assert!(head.starts_with("HTTP/1.1 403"), "{request}: {head}");
        assert_eq!(body, LOCAL_HOST_DENIAL);
    }
    assert!(heads.lock().unwrap().is_empty());
    assert_eq!(fixture.connections.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_guard_tunnels_and_forwards_to_hosts_it_allows() {
    let fixture = fixture().await;
    let p = fixture.port;
    let guard = NetworkGuard::start_with(
        Upstream::default(),
        resolver(&[("site.test", "127.0.0.1")]),
        only_port(p),
    )
    .unwrap();

    let mut tunnel = TcpStream::connect(guard.addr()).await.unwrap();
    tunnel
        .write_all(
            format!("CONNECT site.test:{p} HTTP/1.1\r\nHost: site.test:{p}\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let head = read_head(&mut tunnel).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    tunnel
        .write_all(b"GET /through HTTP/1.1\r\nHost: site.test\r\n\r\n")
        .await
        .unwrap();
    let (reply, body) = response(&mut tunnel).await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert_eq!(body, "hello");
    assert!(fixture.heads.lock().unwrap()[0].starts_with("GET /through HTTP/1.1"));

    let (head, body) = exchange(
        guard.addr(),
        &format!(
            "GET http://site.test:{p}/hello?x=1 HTTP/1.1\r\nHost: site.test:{p}\r\nProxy-Connection: keep-alive\r\nProxy-Authorization: Basic eDp5\r\nX-Kept: yes\r\n\r\n"
        ),
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, "hello");
    let seen = fixture.heads.lock().unwrap()[1].clone();
    assert!(seen.starts_with("GET /hello?x=1 HTTP/1.1\r\n"), "{seen}");
    assert!(
        !has_header(&seen, "proxy-connection") && !has_header(&seen, "proxy-authorization"),
        "{seen}"
    );
    assert!(
        seen.to_ascii_lowercase()
            .contains(&format!("host: site.test:{p}")),
        "{seen}"
    );
    assert!(has_header(&seen, "x-kept"), "{seen}");
}

#[tokio::test]
async fn upstream_proxies_are_chained_with_their_credentials() {
    let (port, heads) = upstream_proxy().await;
    let upstream = Upstream {
        https: Some(ProxyUrl {
            host: "127.0.0.1".into(),
            port,
            authorization: Some("Basic dTpw".into()),
        }),
        http: Some(ProxyUrl {
            host: "127.0.0.1".into(),
            port,
            authorization: Some("Basic aDpx".into()),
        }),
        no_proxy: Vec::new(),
    };
    let guard = NetworkGuard::start_with(
        upstream,
        resolver(&[("site.test", "93.184.216.34")]),
        Arc::new(refused),
    )
    .unwrap();

    let mut tunnel = TcpStream::connect(guard.addr()).await.unwrap();
    tunnel
        .write_all(b"CONNECT site.test:443 HTTP/1.1\r\nHost: site.test:443\r\nProxy-Authorization: Basic eDp5\r\n\r\n")
        .await
        .unwrap();
    let head = read_head(&mut tunnel).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    tunnel.write_all(b"ping\r\n\r\n").await.unwrap();
    let mut reply = [0u8; 9];
    tokio::time::timeout(WAIT, tunnel.read_exact(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&reply, b"tunnelled");
    {
        let seen = heads.lock().unwrap();
        assert!(
            seen[0].starts_with("CONNECT site.test:443 HTTP/1.1\r\n"),
            "{}",
            seen[0]
        );
        assert!(
            seen[0].contains("Proxy-Authorization: Basic dTpw\r\n"),
            "{}",
            seen[0]
        );
        assert!(!seen[0].contains("eDp5"), "{}", seen[0]);
        assert_eq!(seen[1], "ping\r\n\r\n");
    }

    // A name that doesn't resolve here is passed on by name.
    let (head, body) =
        exchange(guard.addr(), "GET http://site.test/ HTTP/1.1\r\nHost: site.test\r\nProxy-Authorization: Basic eDp5\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, "upstream");
    let (head, _) = exchange(
        guard.addr(),
        "CONNECT unknown.test:443 HTTP/1.1\r\nHost: unknown.test:443\r\n\r\n",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    let seen = heads.lock().unwrap();
    assert!(
        seen[2].starts_with("GET http://site.test/ HTTP/1.1\r\n"),
        "{}",
        seen[2]
    );
    assert!(
        seen[2].contains("proxy-authorization: Basic aDpx\r\n"),
        "{}",
        seen[2]
    );
    assert!(!seen[2].contains("eDp5"), "{}", seen[2]);
    assert!(
        seen[3].starts_with("CONNECT unknown.test:443 HTTP/1.1\r\n"),
        "{}",
        seen[3]
    );
}

#[tokio::test]
async fn no_proxy_hosts_skip_the_upstream() {
    let fixture = fixture().await;
    let (port, heads) = upstream_proxy().await;
    let p = fixture.port;
    let upstream = Upstream {
        http: Some(ProxyUrl {
            host: "127.0.0.1".into(),
            port,
            authorization: None,
        }),
        https: None,
        no_proxy: vec!["corp.test".into()],
    };
    let guard = NetworkGuard::start_with(
        upstream,
        resolver(&[("wiki.corp.test", "127.0.0.1")]),
        only_port(p),
    )
    .unwrap();
    let (head, body) = exchange(
        guard.addr(),
        &format!("GET http://wiki.corp.test:{p}/ HTTP/1.1\r\nHost: wiki.corp.test:{p}\r\n\r\n"),
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert_eq!(body, "hello");
    assert!(heads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_guard_is_not_a_web_server_and_bounds_request_heads() {
    let guard = NetworkGuard::start(Upstream::default()).unwrap();
    assert!(guard.addr().ip().is_loopback());
    let (head, _) = exchange(guard.addr(), "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").await;
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
    let (head, _) = exchange(
        guard.addr(),
        "GET https://site.test/ HTTP/1.1\r\nHost: site.test\r\n\r\n",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
    let (head, _) = exchange(
        guard.addr(),
        "CONNECT site.test HTTP/1.1\r\nHost: site.test\r\n\r\n",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 400"), "{head}");
    let padded = |size: usize| {
        format!(
            "GET http://site.test/ HTTP/1.1\r\nHost: site.test\r\nX-Pad: {}\r\n\r\n",
            "a".repeat(size)
        )
    };
    let (head, _) = exchange(guard.addr(), &padded(16 * 1024)).await;
    assert!(head.starts_with("HTTP/1.1 431"), "{head}");
    let (head, _) = exchange(guard.addr(), &padded(64 * 1024)).await;
    assert!(head.starts_with("HTTP/1.1 431"), "{head}");
    // Just under the bound, the head is read and the destination checked.
    let offline =
        NetworkGuard::start_with(Upstream::default(), resolver(&[]), Arc::new(refused)).unwrap();
    let (head, _) = exchange(offline.addr(), &padded(15 * 1024)).await;
    assert!(head.starts_with("HTTP/1.1 502"), "{head}");
}

#[tokio::test]
async fn a_refused_upstream_connect_is_a_bad_gateway() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            read_head(&mut socket).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n",
                )
                .await;
        }
    });
    let upstream = Upstream {
        https: Some(ProxyUrl {
            host: "127.0.0.1".into(),
            port,
            authorization: None,
        }),
        ..Upstream::default()
    };
    let guard = NetworkGuard::start_with(upstream, resolver(&[]), Arc::new(refused)).unwrap();
    let (head, _) = exchange(
        guard.addr(),
        "CONNECT site.test:443 HTTP/1.1\r\nHost: site.test:443\r\n\r\n",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 502"), "{head}");
    // A destination that doesn't resolve, with no upstream, is a bad gateway too.
    let direct =
        NetworkGuard::start_with(Upstream::default(), resolver(&[]), Arc::new(refused)).unwrap();
    let (head, _) = exchange(
        direct.addr(),
        "CONNECT nowhere.test:443 HTTP/1.1\r\nHost: nowhere.test:443\r\n\r\n",
    )
    .await;
    assert!(head.starts_with("HTTP/1.1 502"), "{head}");
}

#[tokio::test]
async fn dropping_the_guard_closes_its_listener_and_tunnels() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = Arc::new(Mutex::new(Vec::new()));
    let keep = held.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            keep.lock().unwrap().push(socket);
        }
    });
    let guard = NetworkGuard::start_with(
        Upstream::default(),
        resolver(&[("site.test", "127.0.0.1")]),
        only_port(port),
    )
    .unwrap();
    let addr = guard.addr();
    let mut tunnel = TcpStream::connect(addr).await.unwrap();
    tunnel
        .write_all(
            format!("CONNECT site.test:{port} HTTP/1.1\r\nHost: site.test:{port}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    assert!(read_head(&mut tunnel).await.starts_with("HTTP/1.1 200"));
    drop(guard);
    let mut rest = Vec::new();
    let read = tokio::time::timeout(WAIT, tunnel.read_to_end(&mut rest))
        .await
        .expect("the tunnel stays open");
    assert!(read.is_err() || rest.is_empty());
    let refused_connect = tokio::time::timeout(WAIT, async {
        loop {
            match TcpStream::connect(addr).await {
                Err(_) => break,
                Ok(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await;
    assert!(refused_connect.is_ok(), "the listener stays open");
}

#[test]
fn proxy_variables_follow_curl_and_refuse_socks() {
    fn vars(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        }
    }
    let upstream = Upstream::from_vars(vars(&[
        ("https_proxy", "http://low:1"),
        ("HTTPS_PROXY", "http://up:2"),
        ("NO_PROXY", "corp.test,.internal"),
    ]))
    .unwrap();
    assert_eq!(upstream.https.as_ref().unwrap().host, "low");
    assert_eq!(upstream.https.as_ref().unwrap().port, 1);
    assert!(
        upstream.route("wiki.corp.test", true).is_none()
            && upstream.route("a.internal", true).is_none()
    );
    assert!(
        upstream.route("corp.test", true).is_none()
            && upstream.route("WIKI.CORP.TEST.", true).is_none()
    );
    assert!(upstream.route("notcorp.test", true).is_some());
    assert!(upstream.route("example.test", true).is_some());
    assert!(
        upstream.route("example.test", false).is_none(),
        "no http upstream"
    );
    assert_eq!(
        Upstream::from_vars(vars(&[("ALL_PROXY", "http://all:3")]))
            .unwrap()
            .http
            .unwrap()
            .host,
        "all"
    );
    assert_eq!(
        Upstream::from_vars(vars(&[
            ("ALL_PROXY", "http://all:3"),
            ("https_proxy", "http://tls:4")
        ]))
        .unwrap()
        .https
        .unwrap()
        .host,
        "tls"
    );
    assert_eq!(
        Upstream::from_vars(vars(&[("HTTPS_PROXY", "socks5://s:1080")]))
            .unwrap_err()
            .to_string(),
        SOCKS_UNSUPPORTED
    );
    assert_eq!(
        Upstream::from_vars(vars(&[("all_proxy", "socks5h://s:1080")]))
            .unwrap_err()
            .to_string(),
        SOCKS_UNSUPPORTED
    );
    assert_eq!(
        Upstream::from_vars(vars(&[("https_proxy", "https://tls.test:443")]))
            .unwrap_err()
            .to_string(),
        SOCKS_UNSUPPORTED
    );
    assert!(Upstream::from_vars(vars(&[])).unwrap().https.is_none());
    assert!(Upstream::from_vars(vars(&[("HTTPS_PROXY", "")]))
        .unwrap()
        .https
        .is_none());
    assert!(
        Upstream::from_vars(vars(&[("NO_PROXY", "*"), ("HTTPS_PROXY", "http://up:2")]))
            .unwrap()
            .route("example.test", true)
            .is_none()
    );
    assert!(Upstream::from_vars(vars(&[("HTTPS_PROXY", "http://:2")])).is_err());
}

#[test]
fn proxy_urls_carry_their_credentials_and_curl_defaults() {
    let url = ProxyUrl::parse("HTTPS_PROXY", "http://u:p@127.0.0.1:3128/").unwrap();
    assert_eq!(
        (url.host.as_str(), url.port, url.authorization.as_deref()),
        ("127.0.0.1", 3128, Some("Basic dTpw"))
    );
    let url = ProxyUrl::parse("HTTPS_PROXY", "proxy.corp:8080").unwrap();
    assert_eq!(
        (url.host.as_str(), url.port, url.authorization),
        ("proxy.corp", 8080, None)
    );
    assert_eq!(
        ProxyUrl::parse("HTTPS_PROXY", "http://proxy.corp")
            .unwrap()
            .port,
        1080
    );
    assert_eq!(
        ProxyUrl::parse("HTTPS_PROXY", "http://proxy.corp:80")
            .unwrap()
            .port,
        80
    );
    let url = ProxyUrl::parse("HTTPS_PROXY", "http://[::1]:3128").unwrap();
    assert_eq!((url.host.as_str(), url.port), ("::1", 3128));
    let url = ProxyUrl::parse("HTTPS_PROXY", "http://a%40b:p%3Aq@proxy:1").unwrap();
    assert_eq!(url.authorization.as_deref(), Some("Basic YUBiOnA6cQ=="));
    // Credentials never reach a log line.
    assert!(!format!("{url:?}").contains("YUBi"));
    assert_eq!(
        ProxyUrl::parse("HTTPS_PROXY", "http://proxy:99999")
            .unwrap_err()
            .to_string(),
        "Bruhs can't read the proxy address in HTTPS_PROXY."
    );
}

#[test]
fn the_environment_is_read_the_same_way() {
    let from_env = Upstream::from_env().map_err(|error| error.to_string());
    let from_vars =
        Upstream::from_vars(|key| std::env::var(key).ok()).map_err(|error| error.to_string());
    assert_eq!(from_env, from_vars);
}
