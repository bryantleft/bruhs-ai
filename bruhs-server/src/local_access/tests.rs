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
