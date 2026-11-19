use super::*;
use axum::{
    body::{to_bytes, Body},
    http::{Method, StatusCode},
    Router,
};

#[tokio::test]
async fn media_streams_ranges_and_head_without_buffering_the_whole_file() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("demo.mp4"), b"0123456789").unwrap();
    for (method, range, expected_status, expected_body) in [
        (
            Method::GET,
            Some("bytes=3-5"),
            StatusCode::PARTIAL_CONTENT,
            "345",
        ),
        (
            Method::GET,
            Some("bytes=-2"),
            StatusCode::PARTIAL_CONTENT,
            "89",
        ),
        (Method::HEAD, None, StatusCode::OK, ""),
        (
            Method::GET,
            Some("bytes=20-25"),
            StatusCode::RANGE_NOT_SATISFIABLE,
            "",
        ),
    ] {
        let mut request = Request::builder().method(method).uri("/api/agent/media");
        if let Some(range) = range {
            request = request.header(header::RANGE, range);
        }
        let response = get_media(
            Query(MediaQuery {
                download: false,
                path: "demo.mp4".into(),
                project: Some(directory.path().to_string_lossy().into_owned()),
            }),
            request.body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), expected_status);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "video/mp4");
        if expected_status == StatusCode::PARTIAL_CONTENT {
            assert!(response.headers().contains_key(header::CONTENT_RANGE));
        }
        assert_eq!(
            to_bytes(response.into_body(), usize::MAX).await.unwrap(),
            expected_body
        );
    }
}

#[test]
fn local_media_rejects_traversal_foreign_files_and_nonmedia() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    std::fs::create_dir(&project).unwrap();
    std::fs::write(directory.path().join("outside.png"), b"outside").unwrap();
    std::fs::write(project.join("secret.txt"), b"secret").unwrap();
    std::fs::write(project.join("inside.png"), b"inside").unwrap();
    let project_str = project.to_str().unwrap();
    assert!(resolve_media("inside.png", Some(project_str)).is_ok());
    assert!(matches!(
        resolve_media("../outside.png", Some(project_str)),
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        resolve_media(
            directory.path().join("outside.png").to_str().unwrap(),
            Some(project_str)
        ),
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        resolve_media("secret.txt", Some(project_str)),
        Err(AppError::BadRequest(_))
    ));
    assert!(matches!(
        resolve_media("inside.png", None),
        Err(AppError::BadRequest(_))
    ));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            directory.path().join("outside.png"),
            project.join("escape.png"),
        )
        .unwrap();
        assert!(matches!(
            resolve_media("escape.png", Some(project_str)),
            Err(AppError::Forbidden(_))
        ));
    }
    let upload = bruhs_core::write_upload("inline.png", b"image").unwrap();
    assert!(resolve_media(&upload.path, None).is_ok());
    assert!(resolve_media(
        &upload.path,
        Some(directory.path().join("deleted-worktree").to_str().unwrap())
    )
    .is_ok());
    std::fs::remove_file(&upload.path).unwrap();
}

#[tokio::test]
async fn only_rich_documents_can_be_framed_and_origin_guards_still_apply() {
    let policy =
        crate::local_access::Policy::from_config(None, Some("https://bruhs.localhost")).unwrap();
    let app = Router::new()
        .route("/api/agent/rich-response", axum::routing::get(get_host))
        .route("/api/private", axum::routing::get(|| async { "private" }))
        .route("/api/agent/rich-media", axum::routing::get(get_media_host))
        .layer(axum::middleware::from_fn_with_state(
            policy,
            crate::local_access::guard,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("{base}/api/agent/rich-response"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-bruhs-rich-response"], "1");
    assert!(response.headers()["permissions-policy"]
        .to_str()
        .unwrap()
        .contains("microphone=()"));
    assert!(!response.headers().contains_key(header::X_FRAME_OPTIONS));
    let policies: Vec<_> = response
        .headers()
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .map(|h| h.to_str().unwrap().to_string())
        .collect();
    assert!(policies.iter().any(|p| p.contains("connect-src 'none'")));
    assert!(policies
        .iter()
        .any(|p| p.contains("frame-ancestors") && p.contains("https://bruhs.localhost")));
    assert!(response
        .text()
        .await
        .unwrap()
        .contains("sandbox', 'allow-scripts"));
    let response = client
        .get(format!("{base}/api/private"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
    for path in [
        "/api/agent/rich-response",
        "/api/agent/rich-media",
        "/api/private",
    ] {
        let response = client
            .get(format!("{base}{path}"))
            .header("Origin", "null")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = client
            .get(format!("{base}{path}"))
            .header("sec-fetch-site", "cross-site")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    server.abort();
}

#[tokio::test]
async fn media_shell_cannot_execute_an_interactive_html_fragment() {
    let response = get_media_host().await;
    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        MEDIA_HOST_CSP
    );
    let body = String::from_utf8(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("JSON.parse"));
    assert!(!body.contains("srcdoc="));
    assert!(!body.contains("innerHTML"));
    assert!(body.contains("media.setAttribute('sandbox','')"));
    let response = get_host().await;
    assert!(response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("frame-src about:;"));
}

/// `/api/agent/media` is exempt from the guard's inert policy (it is a
/// framed rich-response route), yet it serves files an agent wrote. Its own
/// sandbox is all that keeps an SVG opened from it off the server's origin,
/// where it would pass the vault's owner check (ruling F26).
#[tokio::test]
async fn agent_written_svg_from_media_stays_sandboxed_behind_the_guard() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("chart.svg"),
        br#"<svg xmlns="http://www.w3.org/2000/svg"><script>fetch('/api/vault')</script></svg>"#,
    )
    .unwrap();
    let app = Router::new()
        .route("/api/agent/media", axum::routing::get(get_media))
        .layer(axum::middleware::from_fn_with_state(
            crate::local_access::Policy::default(),
            crate::local_access::guard,
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let response = client
        .get(format!("{base}/api/agent/media"))
        .query(&[
            ("path", "chart.svg"),
            ("project", directory.path().to_str().unwrap()),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
    let policies: Vec<_> = response
        .headers()
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect();
    assert!(
        policies.iter().any(|policy| policy
            .split(';')
            .any(|directive| directive.trim() == "sandbox")),
        "{policies:?}"
    );
    assert!(
        policies
            .iter()
            .all(|policy| !policy.contains("allow-same-origin")),
        "{policies:?}"
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    server.abort();
}
