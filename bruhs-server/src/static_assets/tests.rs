use std::{io::Write as _, path::Path, sync::Arc, time::Duration};

use axum::http::{header, HeaderMap};
use flate2::{write::GzEncoder, Compression};
use reqwest::{Client, StatusCode};

struct Server {
    task: tokio::task::JoinHandle<()>,
    url: String,
    client: Client,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    async fn start(directory: &Path) -> Self {
        let calendar = bruhs_core::google_calendar::GoogleCalendarHandle::new(
            None,
            bruhs_core::google_calendar::tokens::TokenStore::in_memory(),
            bruhs_core::google_calendar::cursor::CalendarFiles::at(directory),
        );
        let state = bruhs_core::AppState::builder()
            .secret_store(bruhs_core::secrets::SecretStore::in_memory())
            .connections(bruhs_core::connections::Connections::in_memory())
            .google_calendar(Arc::new(calendar))
            .build();
        let config = crate::config::Config::from_settings(crate::config::Settings {
            web_dist: Some(directory.to_str().expect("temporary UTF-8 path")),
            ..Default::default()
        })
        .expect("fixture configuration");
        let router =
            crate::router(crate::config::ServerState {
                app: Arc::new(state),
                config: Arc::new(config),
                updates: bruhs_core::updates::UpdateChecker::new(
                    bruhs_core::updates::BuildInfo::new("", "", "static-asset-test"),
                ),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("private fixture listener");
        let url = format!("http://{}", listener.local_addr().expect("fixture address"));
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.expect("fixture server");
        });
        let client = Client::builder()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .timeout(Duration::from_secs(5))
            .build()
            .expect("fixture HTTP client");
        Self { task, url, client }
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .get(format!("{}{path}", self.url))
            .header(header::ORIGIN, &self.url)
    }
}

fn varies_on(headers: &HeaderMap, name: &str) -> bool {
    headers.get_all(header::VARY).iter().any(|value| {
        value
            .to_str()
            .expect("ASCII Vary")
            .split(',')
            .any(|token| token.trim().eq_ignore_ascii_case(name))
    })
}

fn fixture(directory: &Path) -> (Vec<u8>, Vec<u8>) {
    let raw = b"\0asm reproducible release asset ".repeat(2048);
    let mut compressor = GzEncoder::new(Vec::new(), Compression::best());
    compressor.write_all(&raw).expect("compress fixture");
    let gzip = compressor.finish().expect("finish fixture compression");
    std::fs::write(directory.join("app.wasm"), &raw).expect("original asset");
    std::fs::write(directory.join("app.wasm.gz"), &gzip).expect("gzip sidecar");
    std::fs::write(directory.join("boot.js"), b"export default 1;")
        .expect("asset without a sidecar");
    (raw, gzip)
}

#[tokio::test]
async fn negotiation_keeps_mime_validators_and_identity_fallback() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let (raw, gzip) = fixture(directory.path());
    let server = Server::start(directory.path()).await;
    let response = server
        .get("/app.wasm")
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .expect("gzip response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/wasm");
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
    assert_eq!(response.content_length(), u64::try_from(gzip.len()).ok());
    assert!(varies_on(response.headers(), "accept-encoding"));
    assert!(varies_on(response.headers(), "origin"));
    assert_eq!(&response.bytes().await.expect("gzip body")[..], &gzip);

    for accept in [
        None,
        Some("identity"),
        Some("gzip;q=0"),
        Some("br, gzip;q=0"),
    ] {
        let request = server.get("/app.wasm");
        let request = match accept {
            Some(value) => request.header(header::ACCEPT_ENCODING, value),
            None => request,
        };
        let response = request.send().await.expect("identity response");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
        assert!(varies_on(response.headers(), "accept-encoding"));
        assert_eq!(&response.bytes().await.expect("identity body")[..], &raw);
    }
    let response = server
        .get("/boot.js")
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .expect("missing-sidecar fallback");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
    assert_eq!(response.text().await.expect("JS body"), "export default 1;");

    let response = server
        .client
        .head(format!("{}/app.wasm", server.url))
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .expect("HEAD response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_ENCODING], "gzip");
    // Reqwest reports the empty HEAD body length from its size hint; inspect
    // the wire metadata that describes the selected GET representation.
    assert_eq!(
        response.headers()[header::CONTENT_LENGTH],
        gzip.len().to_string()
    );
    assert!(response.bytes().await.expect("HEAD body").is_empty());
}

#[tokio::test]
async fn ranges_and_conditionals_apply_to_the_selected_representation() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let (raw, gzip) = fixture(directory.path());
    let server = Server::start(directory.path()).await;
    let response = server
        .get("/app.wasm")
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .expect("asset metadata");
    let modified = response.headers()[header::LAST_MODIFIED].clone();
    drop(response);
    let response = server
        .get("/app.wasm")
        .header(header::ACCEPT_ENCODING, "gzip")
        .header(header::IF_MODIFIED_SINCE, modified)
        .send()
        .await
        .expect("conditional response");
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert!(varies_on(response.headers(), "accept-encoding"));
    assert!(response.bytes().await.expect("304 body").is_empty());

    for (accept, selected) in [("gzip", &gzip), ("identity", &raw)] {
        let response = server
            .get("/app.wasm")
            .header(header::ACCEPT_ENCODING, accept)
            .header(header::RANGE, "bytes=0-31")
            .send()
            .await
            .expect("range response");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response.headers()[header::CONTENT_RANGE],
            format!("bytes 0-31/{}", selected.len())
        );
        assert_eq!(
            &response.bytes().await.expect("range body")[..],
            &selected[..32]
        );
    }
    let response = server
        .get("/app.wasm")
        .header(header::ACCEPT_ENCODING, "gzip")
        .header(header::RANGE, format!("bytes={}-", gzip.len()))
        .send()
        .await
        .expect("unsatisfiable range");
    assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert!(varies_on(response.headers(), "accept-encoding"));
    let response = server
        .get("/missing.js")
        .send()
        .await
        .expect("missing asset");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_and_live_stream_bypass_static_asset_encoding() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let server = Server::start(directory.path()).await;
    let response = server
        .get("/api/terminal/sessions")
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .expect("real in-memory API response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
    assert!(!varies_on(response.headers(), "accept-encoding"));
    assert_eq!(
        response
            .json::<serde_json::Value>()
            .await
            .expect("API body"),
        serde_json::json!({"sessions": []})
    );

    let mut response = server
        .get("/api/events/stream")
        .header(header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .expect("real combined SSE stream");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/event-stream"
    );
    assert!(!response.headers().contains_key(header::CONTENT_ENCODING));
    assert!(!varies_on(response.headers(), "accept-encoding"));
    let mut text = String::new();
    while ["tasks", "calendar", "meetings"]
        .iter()
        .any(|name| !text.contains(&format!("event: {name}\n")))
    {
        let chunk = response
            .chunk()
            .await
            .expect("SSE before HTTP timeout")
            .expect("SSE remains open");
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
}
