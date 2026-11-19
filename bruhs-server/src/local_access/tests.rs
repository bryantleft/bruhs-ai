use super::*;
use axum::http::StatusCode;

#[test]
fn loopback_origins_require_valid_origin_syntax() {
    let policy = Policy::default();
    for value in [
        "http://localhost:8088",
        "https://127.0.0.1:3001",
        "http://[::1]:8088",
    ] {
        assert!(policy.allows_origin(&value.parse().unwrap()), "{value}");
    }
    for value in [
        // Any local dev site can use a `*.localhost` name; only configured
        // ones are trusted.
        "http://bruhs.localhost",
        "https://bruhs.localhost:1355",
        "null",
        "https://foreign.example",
        "http://localhost.foreign.example",
        "http://foreign@localhost",
        "http://localhost/",
        "http://localhost/path",
        "http://localhost?query",
        "http://localhost#fragment",
        "ftp://localhost",
        "http://127.0.0.2",
        "http://localhost:99999",
        "http://.localhost",
        "http://a..localhost",
        "http://localhost, https://foreign.example",
    ] {
        assert!(!policy.allows_origin(&value.parse().unwrap()), "{value}");
    }
}

#[test]
fn configured_origin_is_exact_and_malformed_configuration_is_rejected() {
    let policy = Policy::from_config(Some("https://app.example/"), None).unwrap();
    assert!(policy.allows_origin(&"https://app.example".parse().unwrap()));
    for value in [
        "https://app.example:444",
        "http://app.example",
        "https://app.example.foreign.example",
    ] {
        assert!(!policy.allows_origin(&value.parse().unwrap()), "{value}");
    }
    for value in [
        "null",
        "https://user:secret@app.example",
        "https://app.example/path",
        "https://app.example?token",
        "https://app.example#fragment",
        "https://app.example:99999",
    ] {
        assert!(Policy::from_config(Some(value), None).is_err(), "{value}");
    }
    assert!(Policy::from_config(Some(""), None).is_ok());
}

#[test]
fn configured_web_origin_is_exact() {
    let policy = Policy::from_config(
        Some("https://api.bruhs.localhost"),
        Some("https://bruhs.localhost/"),
    )
    .unwrap();
    for value in ["https://bruhs.localhost", "https://api.bruhs.localhost"] {
        assert!(policy.allows_origin(&value.parse().unwrap()), "{value}");
    }
    for value in [
        "http://bruhs.localhost",
        "https://bruhs.localhost:1355",
        "https://other.localhost",
    ] {
        assert!(!policy.allows_origin(&value.parse().unwrap()), "{value}");
    }
    assert!(Policy::from_config(None, Some("https://bruhs.localhost/path")).is_err());
    assert!(Policy::from_config(None, Some("  ")).is_ok());
}

/// Requests through a local proxy keep a `*.localhost` Host; only the Origin
/// policy narrowed.
#[tokio::test]
async fn localhost_host_names_still_pass_the_rebinding_check() {
    let policy = Policy::default();
    let app = axum::Router::new()
        .route(
            "/api/x",
            axum::routing::get(|| async { StatusCode::NO_CONTENT }),
        )
        .layer(axum::middleware::from_fn_with_state(policy, guard));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let status =
        |request: reqwest::RequestBuilder| async move { request.send().await.unwrap().status() };
    assert_eq!(
        status(
            client
                .get(format!("{base}/api/x"))
                .header("host", "api.bruhs.localhost")
        )
        .await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        status(
            client
                .get(format!("{base}/api/x"))
                .header("origin", "https://evil.localhost")
        )
        .await,
        StatusCode::FORBIDDEN
    );
    server.abort();
}

#[tokio::test]
async fn hostile_requests_never_reach_handlers_and_local_preflights_work() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let handler = axum::routing::any(move || {
        let seen = seen.clone();
        async move {
            seen.fetch_add(1, Ordering::SeqCst);
            StatusCode::NO_CONTENT
        }
    });
    let policy = Policy::default();
    let app = axum::Router::new()
        .route("/api/private", handler.clone())
        .route(OAUTH_CALLBACK, handler.clone())
        .route("/", handler)
        .layer(policy.cors())
        .layer(axum::middleware::from_fn_with_state(policy, guard));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let endpoint = format!("{base}/api/private");
    for request in [
        client
            .post(&endpoint)
            .header("origin", "https://foreign.example"),
        client
            .get(&endpoint)
            .header("host", "rebinding.foreign.example"),
        client.post(&endpoint).header("origin", "null"),
        client
            .post(&endpoint)
            .header("sec-fetch-site", "cross-site"),
        client
            .get(&endpoint)
            .header("x-bruhs-verification-browser", "1"),
        client
            .get(&endpoint)
            .header("user-agent", "Mozilla/5.0 BruhsVerification/1"),
        client
            .get(&endpoint)
            .header("user-agent", "Mozilla/5.0 BruhsVerification/1")
            .header("connection", "upgrade")
            .header("upgrade", "websocket"),
        client
            .get(&endpoint)
            .header("origin", "http://localhost:8088")
            .header("origin", "https://foreign.example"),
        client
            .post(format!("{base}{OAUTH_CALLBACK}"))
            .header("sec-fetch-site", "cross-site"),
        client
            .get(format!("{base}{OAUTH_CALLBACK}"))
            .header("origin", "https://foreign.example"),
    ] {
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(!response
            .headers()
            .contains_key("access-control-allow-origin"));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let preflight = client
        .request(reqwest::Method::OPTIONS, &endpoint)
        .header("origin", "http://localhost:8088")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert!(preflight.status().is_success());
    assert_eq!(
        preflight.headers()["access-control-allow-origin"],
        "http://localhost:8088"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    for origin in [None, Some("http://localhost:8088")] {
        let mut request = client.post(&endpoint);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-frame-options"], "DENY");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    }
    // An external link can open the app. Only the exact OAuth GET callback
    // permits a cross-site API navigation without Origin; state is checked by
    // its production handler, not this synthetic handler.
    for path in ["/", OAUTH_CALLBACK] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .header("sec-fetch-site", "cross-site")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn merged_workbench_services_preserve_the_local_origin_boundary() {
    let root = tempfile::tempdir().unwrap();
    let delivery = bruhs_core::delivery::DeliveryService::open(root.path().join("delivery.json"))
        .await
        .unwrap();
    let hosts = bruhs_core::remote_hosts::RemoteHostManager::open(root.path().join("hosts.json"))
        .await
        .unwrap();
    let app = crate::workbench_services(delivery, hosts, Policy::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    for path in [
        "/api/delivery",
        "/api/remote-hosts",
        "/api/delivery/ui",
        "/api/remote-hosts/ui",
    ] {
        for (header, value) in [
            ("origin", "https://foreign.example"),
            ("host", "rebinding.example"),
            ("user-agent", "BruhsVerification/1"),
        ] {
            let response = client
                .get(format!("{base}{path}"))
                .header(header, value)
                .send()
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::FORBIDDEN,
                "{path} accepted {header}"
            );
        }
        let response = client
            .get(format!("{base}{path}"))
            .header("origin", "http://localhost:3000")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        if path.ends_with("/ui") {
            assert!(response.headers().contains_key("content-security-policy"));
        } else {
            assert_eq!(response.headers()["x-frame-options"], "DENY");
        }
    }
    let response = client
        .post(format!("{base}/api/delivery/execute-native-stack"))
        .header("origin", "https://foreign.example")
        .json(&serde_json::json!({"id":"unreviewed","acknowledge_server_heads":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    server.abort();
    assert!(server.await.unwrap_err().is_cancelled());
}

fn headers(pairs: &[(&'static str, &str)]) -> axum::http::HeaderMap {
    let mut headers = axum::http::HeaderMap::new();
    for (name, value) in pairs {
        headers.append(*name, value.parse().unwrap());
    }
    headers
}

/// Ruling F26 and the V Task 10 minors: only no `Origin`, the request's own
/// `http` Host origin, or a configured one is the owner's app.
#[test]
fn owner_requests_are_the_native_client_the_own_page_or_a_configured_origin() {
    let policy = Policy::from_config(None, Some("https://bruhs.localhost")).unwrap();
    let host = ("host", "127.0.0.1:3001");
    for allowed in [
        headers(&[host]),
        headers(&[host, ("origin", "http://127.0.0.1:3001")]),
        headers(&[
            ("host", "LOCALHOST:3001"),
            ("origin", "http://localhost:3001"),
        ]),
        headers(&[host, ("origin", "https://bruhs.localhost")]),
    ] {
        assert!(policy.owner_request(&allowed), "{allowed:?}");
    }
    for refused in [
        // A sandboxed or no-referrer page.
        headers(&[host, ("origin", "null")]),
        headers(&[
            host,
            ("origin", "http://127.0.0.1:3001"),
            ("origin", "http://127.0.0.1:3001"),
        ]),
        headers(&[host, ("origin", "https://127.0.0.1:3001")]),
        // Another port of the same host is another page.
        headers(&[host, ("origin", "http://127.0.0.1:5173")]),
        headers(&[
            ("host", "127.0.0.1:5173"),
            ("origin", "http://127.0.0.1:3001"),
        ]),
        headers(&[("host", "127.0.0.1"), ("origin", "http://127.0.0.1:3001")]),
        headers(&[host, ("origin", "http://localhost:3001")]),
        headers(&[host, ("origin", "http://127.0.0.1:3001/")]),
        headers(&[("origin", "http://127.0.0.1:3001")]),
        headers(&[host, host, ("origin", "http://127.0.0.1:3001")]),
        headers(&[host, ("origin", "https://other.localhost")]),
    ] {
        assert!(!policy.owner_request(&refused), "{refused:?}");
    }
}

/// Ruling V Task 10 C1: a file the API serves can't run as the app's own
/// page. An uploaded SVG opened directly is sandboxed with no script.
#[tokio::test]
async fn files_and_api_responses_served_as_documents_are_inert() {
    let directory = tempfile::tempdir().unwrap();
    let svg = directory.path().join("x.svg");
    std::fs::write(
        &svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script>fetch('/api/vault')</script></svg>"#,
    )
    .unwrap();
    let config = crate::config::Config::from_settings(crate::config::Settings {
        port: Some("0"),
        web_dist: directory.path().join("missing").to_str(),
        ..Default::default()
    })
    .unwrap();
    let app = crate::router(crate::config::ServerState {
        app: std::sync::Arc::new(
            bruhs_core::AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        ),
        config: std::sync::Arc::new(config),
        updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
            "", "", "unused",
        )),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    let open = client
        .get(format!("{base}/api/agent/uploads/open"))
        .query(&[("path", svg.to_str().unwrap())])
        .send()
        .await
        .unwrap();
    assert_eq!(open.status(), StatusCode::OK);
    assert_eq!(open.headers()["content-type"], "image/svg+xml");
    assert_eq!(open.headers()["content-security-policy"], INERT_CSP);
    assert!(INERT_CSP.starts_with("sandbox;") && INERT_CSP.contains("default-src 'none'"));
    assert!(!INERT_CSP.contains("script"));
    assert_eq!(open.headers()["x-content-type-options"], "nosniff");
    // JSON, errors and the OAuth return page get it too.
    for path in [
        "/api/health",
        "/api/no-such-route",
        "/api/calendar/google/oauth/callback?error=x",
    ] {
        let response = client.get(format!("{base}{path}")).send().await.unwrap();
        assert_eq!(
            response.headers()["content-security-policy"],
            INERT_CSP,
            "{path}"
        );
    }
    // App pages keep their own scripts; a route's own policy stays.
    let workbench = client
        .get(format!("{base}/api/workbench/ui"))
        .send()
        .await
        .unwrap();
    let policies: Vec<_> = workbench
        .headers()
        .get_all("content-security-policy")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect();
    assert!(
        policies.iter().all(|policy| !policy.contains("sandbox")),
        "{policies:?}"
    );
    let worktrees = client
        .get(format!("{base}/api/worktrees/ui"))
        .send()
        .await
        .unwrap();
    assert!(!worktrees.headers().contains_key("content-security-policy"));
    server.abort();
}
