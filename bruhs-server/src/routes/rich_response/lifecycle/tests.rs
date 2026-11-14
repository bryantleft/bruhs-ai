use super::*;
use axum::{http::StatusCode, routing::get, Router};

#[tokio::test]
async fn scoped_assets_support_opaque_media_without_opening_other_apis() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(project.join("demo.mp4"), b"0123456789").unwrap();
    std::fs::write(project.join("other.mp4"), b"other bytes").unwrap();
    let store = RichStore::at(directory.path().join("responses"));
    let asset = store.capture("demo.mp4", project.to_str()).unwrap();
    let other = store.capture("other.mp4", project.to_str()).unwrap();
    let policy = crate::local_access::Policy::default();
    let cors = policy.cors();
    let app = Router::new()
        .route(
            "/api/agent/rich-response/asset/{name}",
            get(
                move |Path(name): Path<String>,
                      Query(grant): Query<AssetGrant>,
                      request: Request| {
                    let store = store.clone();
                    async move { asset_response(store, name, grant.token, request).await }
                },
            ),
        )
        .route(
            "/api/agent/rich-response/state",
            get(|| async { "private" }),
        )
        .layer(cors)
        .layer(axum::middleware::from_fn_with_state(
            policy,
            crate::local_access::guard,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();

    for origin in ["null", "https://untrusted.example"] {
        let response = client
            .get(format!("{base}{}", asset.signed_url))
            .header(header::ORIGIN, origin)
            .header(header::RANGE, "bytes=3-5")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "video/mp4");
        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 3-5/10");
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
        assert_eq!(
            response.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'none'; sandbox"
        );
        assert!(!response.headers().contains_key(header::X_FRAME_OPTIONS));
        assert_eq!(response.text().await.unwrap(), "345");
    }
    let response = client
        .head(format!("{base}{}", asset.signed_url))
        .header("sec-fetch-site", "cross-site")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "10");
    assert!(response.bytes().await.unwrap().is_empty());

    let (asset_path, grant) = asset.signed_url.split_once('?').unwrap();
    let (other_path, _) = other.signed_url.split_once('?').unwrap();
    for path in [
        format!("{asset_path}?token=invalid"),
        format!("{other_path}?{grant}"),
    ] {
        let response = client
            .get(format!("{base}{path}"))
            .header(header::ORIGIN, "null")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let response = client
        .post(format!("{base}{}", asset.signed_url))
        .header(header::ORIGIN, "null")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = client
        .get(format!("{base}/api/agent/rich-response/state"))
        .header(header::ORIGIN, "null")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    server.abort();
}
