//! Reject browser-originated requests before handlers run, including simple
//! requests that CORS alone cannot prevent and loopback DNS rebinding via a
//! foreign Host.
//!
//! No browser talks to the daemon: the web app goes through bruhs-server's
//! `/api/capture/*` proxy, which holds the daemon token. So no Origin is
//! allowed at all (there is no CORS layer), and native clients, which send no
//! Origin, must still present the bearer token (see `auth`).
use axum::{
    extract::Request,
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};

fn loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
        || (host.ends_with(".localhost")
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }))
}

fn local_authority(value: &str) -> bool {
    value
        .parse::<axum::http::uri::Authority>()
        .is_ok_and(|authority| {
            !value.contains('@')
                && loopback_host(authority.host())
                && (authority.as_str() == authority.host() || authority.port_u16().is_some())
        })
}

pub async fn guard(request: Request, next: Next) -> Result<Response, StatusCode> {
    let headers = request.headers();
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    if !host.is_some_and(local_authority)
        || headers.get_all(header::HOST).iter().count() != 1
        || headers.contains_key(header::ORIGIN)
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v == "cross-site" || v == "same-site")
    {
        return Err(StatusCode::FORBIDDEN);
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_must_be_loopback_without_credentials() {
        for host in [
            "127.0.0.1:3002",
            "localhost",
            "[::1]:3002",
            "capture.bruhs.localhost:1355",
        ] {
            assert!(local_authority(host), "{host}");
        }
        for host in [
            "evil.test",
            "localhost.evil.test",
            "evil@localhost",
            "127.0.0.2",
            "localhost:99999",
            ".localhost",
            "a..localhost",
        ] {
            assert!(!local_authority(host), "{host}");
        }
    }

    #[tokio::test]
    async fn browser_requests_cannot_reach_handlers_even_without_preflight() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new()
            .route(
                "/",
                axum::routing::post(move || async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    StatusCode::NO_CONTENT
                }),
            )
            .layer(axum::middleware::from_fn(guard));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // Every browser origin is refused, loopback ones and the dev
        // `*.localhost` names included: browsers go through bruhs-server.
        for origin in [
            "https://evil.test",
            "null",
            "http://localhost.evil.test",
            "http://localhost:8088",
            "http://127.0.0.1:3001",
            "https://bruhs.localhost",
        ] {
            assert_eq!(
                client
                    .post(&url)
                    .header("origin", origin)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN,
                "{origin}"
            );
        }
        let preflight = client
            .request(reqwest::Method::OPTIONS, &url)
            .header("origin", "http://localhost:8088")
            .header("access-control-request-method", "POST")
            .send()
            .await
            .unwrap();
        assert_eq!(preflight.status(), StatusCode::FORBIDDEN);
        assert!(!preflight
            .headers()
            .contains_key("access-control-allow-origin"));
        assert_eq!(
            client
                .post(&url)
                .header("host", "rebinding.evil.test")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        for site in ["cross-site", "same-site"] {
            assert_eq!(
                client
                    .post(&url)
                    .header("sec-fetch-site", site)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // Native clients send no Origin.
        assert_eq!(
            client.post(&url).send().await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        server.abort();
        let _ = server.await;
    }
}
