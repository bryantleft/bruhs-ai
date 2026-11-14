use super::*;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{any, get},
    Json, Router,
};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    directory: tempfile::TempDir,
    host: ApiClientHost,
    url: String,
    hits: Arc<AtomicUsize>,
    slow: Arc<tokio::sync::Notify>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    fn project(&self) -> String {
        self.directory
            .path()
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    }
    fn execution(&self, path: &str) -> ApiExecution {
        ApiExecution {
            project_path: self.project(),
            request_id: Uuid::new_v4(),
            request: ApiRequest {
                url: format!("{{{{base_url}}}}{path}"),
                ..Default::default()
            },
            environment: ApiEnvironment {
                values: std::collections::BTreeMap::from([("base_url".into(), self.url.clone())]),
                ..Default::default()
            },
        }
    }
}
#[derive(Clone)]
struct WebState {
    hits: Arc<AtomicUsize>,
    slow: Arc<tokio::sync::Notify>,
}
async fn fixture() -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let slow = Arc::new(tokio::sync::Notify::new());
    let state = WebState {
        hits: Arc::clone(&hits),
        slow: Arc::clone(&slow),
    };
    let app=Router::new().route("/echo",any(|State(state):State<WebState>,headers:HeaderMap,body:String|async move{
        state.hits.fetch_add(1,Ordering::SeqCst);
        let key=headers.get("x-api-key").and_then(|value|value.to_str().ok()).unwrap_or("").to_string();
        let encoded=percent_encoding::utf8_percent_encode(&key,percent_encoding::NON_ALPHANUMERIC).to_string();
        use base64::Engine;
        let base64=base64::engine::general_purpose::STANDARD.encode(&key);
        (StatusCode::CREATED,[("x-echo",key.clone()),("set-cookie","not-for-the-ui=private".into())],Json(serde_json::json!({"key":key,"encoded":encoded,"base64":base64,"body":body,"content_type":headers.get("content-type").and_then(|v|v.to_str().ok()),"nested":{"ok":true},"html":"<img src=x onerror=alert(1)>"})))
    })).route("/redirect",get(|State(state):State<WebState>,headers:HeaderMap|async move{state.hits.fetch_add(1,Ordering::SeqCst);let key=headers.get("x-api-key").and_then(|v|v.to_str().ok()).unwrap_or("");(StatusCode::FOUND,[("location",format!("/sink?token={key}"))],"redirect") }))
    .route("/sink",get(|State(state):State<WebState>|async move{state.hits.fetch_add(100,Ordering::SeqCst);"must never be followed"}))
    .route("/slow",get(|State(state):State<WebState>|async move{state.hits.fetch_add(1,Ordering::SeqCst);state.slow.notify_one();std::future::pending::<String>().await}))
    .route("/big",get(||async{"x".repeat(PAYLOAD_BYTES+1)}))
    .route("/binary",get(||async{vec![0xffu8,0xfe]}))
    .route("/header-name",get(|headers:HeaderMap|async move{
        let key=headers.get("x-api-key").unwrap().to_str().unwrap();let mut reflected=HeaderMap::new();reflected.insert(axum::http::HeaderName::from_bytes(format!("x-echo-{key}").as_bytes()).unwrap(),axum::http::HeaderValue::from_static("public"));(reflected,"public body")
    }))
    .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = ApiClientHost::new(crate::secrets::SecretStore::in_memory(), vec![]).unwrap();
    Fixture {
        directory,
        host,
        url,
        hits,
        slow,
        server,
    }
}

#[tokio::test]
async fn api_client_real_http_substitutes_and_redacts_before_leaving_secret_module() {
    let f = fixture().await;
    let key = "fixture-Key\"with\\escaping%&+=42";
    f.host
        .0
        .secrets
        .save(&f.project(), "dev key", zeroize::Zeroizing::new(key.into()))
        .await
        .unwrap();
    let mut request = f.execution("/echo");
    request.request.method = "POST".into();
    request.request.headers = vec![
        ApiHeader {
            name: "X-Api-Key".into(),
            value: "{{key}}".into(),
        },
        ApiHeader {
            name: "Content-Type".into(),
            value: "application/json".into(),
        },
    ];
    request
        .environment
        .secrets
        .insert("key".into(), "dev key".into());
    request
        .environment
        .values
        .insert("who".into(), "Ada".into());
    request.request.body =
        "{\"query\":\"{ user(name: \\\"{{who}}\\\") { id } }\",\"key\":\"{{key}}\"}".into();
    let response = f.host.send(request.clone()).await.unwrap();
    assert_eq!(response.status, 201);
    assert!(response.redacted);
    assert_eq!(f.hits.load(Ordering::SeqCst), 1);
    let encoded = serde_json::to_string(&response).unwrap();
    assert!(!encoded.contains(key));
    assert!(!encoded.contains("fixture-Key"));
    assert!(!encoded.contains("not-for-the-ui=private"));
    assert!(encoded.contains("Ada"));
    let json: serde_json::Value = serde_json::from_str(&response.body).unwrap();
    assert_eq!(json["key"], "[redacted]");
    assert_eq!(json["encoded"], "[redacted]");
    assert_eq!(json["base64"], "[redacted]");
    assert!(response
        .headers
        .iter()
        .find(|header| header.name == "set-cookie")
        .is_some_and(|header| header.value == "[redacted]"));
    assert!(!serde_json::to_string(&request).unwrap().contains(key));
}
#[tokio::test]
async fn api_client_redirects_never_forward_any_header_and_oversized_or_binary_bodies_fail() {
    let f = fixture().await;
    f.host
        .0
        .secrets
        .save(
            &f.project(),
            "key",
            zeroize::Zeroizing::new("redirect-fixture-secret".into()),
        )
        .await
        .unwrap();
    let mut request = f.execution("/redirect");
    request
        .environment
        .secrets
        .insert("key".into(), "key".into());
    request.request.headers.push(ApiHeader {
        name: "X-Api-Key".into(),
        value: "{{key}}".into(),
    });
    let response = f.host.send(request).await.unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(f.hits.load(Ordering::SeqCst), 1);
    assert!(response
        .headers
        .iter()
        .any(|header| header.name == "location" && header.value == "/sink?token=[redacted]"));
    assert!(f
        .host
        .send(f.execution("/big"))
        .await
        .unwrap_err()
        .to_string()
        .contains("32 KiB"));
    assert!(f
        .host
        .send(f.execution("/binary"))
        .await
        .unwrap_err()
        .to_string()
        .contains("UTF-8"));
}
#[tokio::test]
async fn api_client_cancel_can_overtake_send_and_stops_active_processing() {
    let f = fixture().await;
    let request = f.execution("/echo");
    f.host.cancel(request.request_id).unwrap();
    assert!(matches!(
        f.host.send(request).await,
        Err(ApiClientError::Cancelled)
    ));
    assert_eq!(f.hits.load(Ordering::SeqCst), 0);
    let request = f.execution("/slow");
    let id = request.request_id;
    let host = f.host.clone();
    let running = tokio::spawn(async move { host.send(request).await });
    tokio::time::timeout(Duration::from_secs(2), f.slow.notified())
        .await
        .unwrap();
    assert!(matches!(
        f.host.send(f.execution("/echo")).await,
        Err(ApiClientError::Busy)
    ));
    f.host.cancel(id).unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), running)
            .await
            .unwrap()
            .unwrap(),
        Err(ApiClientError::Cancelled)
    ));
    assert!(f.host.send(f.execution("/echo")).await.is_ok());
}
#[tokio::test]
async fn api_client_proposals_are_immutable_project_and_turn_scoped_and_one_use() {
    let f = fixture().await;
    let request = f.execution("/echo");
    let scope = Uuid::new_v4();
    let proposal = f
        .host
        .propose(ProposeApiRequest {
            scope,
            thread_id: "thread-a".into(),
            project_path: f.project(),
            request: request.request,
            environment: request.environment,
        })
        .await
        .unwrap();
    assert_eq!(f.hits.load(Ordering::SeqCst), 0);
    assert!(matches!(proposal.state, ApiProposalState::Pending));
    assert!(f
        .host
        .proposal_result(f.project(), scope, "thread-b", proposal.id)
        .await
        .is_err());
    assert!(f
        .host
        .proposal_result(f.project(), Uuid::new_v4(), "thread-a", proposal.id)
        .await
        .is_err());
    let other = tempfile::tempdir().unwrap();
    assert!(f
        .host
        .proposal_result(
            other.path().display().to_string(),
            scope,
            "thread-a",
            proposal.id
        )
        .await
        .is_err());
    let decision = ApiProposalDecision {
        project_path: f.project(),
        id: proposal.id,
        approve: true,
    };
    let (a, b) = tokio::join!(f.host.decide(decision.clone()), f.host.decide(decision));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(f.hits.load(Ordering::SeqCst), 1);
    let result = f
        .host
        .proposal_result(f.project(), scope, "thread-a", proposal.id)
        .await
        .unwrap();
    assert!(matches!(result.state, ApiProposalState::Finished { .. }));
    let request = f.execution("/echo");
    let proposal = f
        .host
        .propose(ProposeApiRequest {
            scope,
            thread_id: "thread-a".into(),
            project_path: f.project(),
            request: request.request,
            environment: request.environment,
        })
        .await
        .unwrap();
    assert!(matches!(
        f.host
            .decide(ApiProposalDecision {
                project_path: f.project(),
                id: proposal.id,
                approve: false
            })
            .await
            .unwrap()
            .state,
        ApiProposalState::Declined
    ));
    assert!(f
        .host
        .decide(ApiProposalDecision {
            project_path: f.project(),
            id: proposal.id,
            approve: true
        })
        .await
        .is_err());
    assert_eq!(f.hits.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn api_client_collection_conflicts_and_malformed_files_preserve_external_content() {
    let f = fixture().await;
    let snapshot = f.host.collection(f.project()).await.unwrap();
    assert!(snapshot.revision.is_none());
    let request = f.execution("/echo");
    let collection = ApiCollection {
        requests: vec![request.request],
        environment: request.environment,
        ..Default::default()
    };
    let saved = f
        .host
        .save(SaveApiCollection {
            project_path: f.project(),
            expected_revision: None,
            collection: collection.clone(),
        })
        .await
        .unwrap();
    assert_eq!(f.host.collection(f.project()).await.unwrap(), saved);
    let file = f.directory.path().join(COLLECTION_FILE);
    let content = std::fs::read(&file).unwrap();
    assert!(matches!(
        f.host
            .save(SaveApiCollection {
                project_path: f.project(),
                expected_revision: None,
                collection: collection.clone()
            })
            .await,
        Err(ApiClientError::Conflict)
    ));
    assert_eq!(std::fs::read(&file).unwrap(), content);
    std::fs::write(&file, "{ invalid external edit").unwrap();
    assert!(f
        .host
        .save(SaveApiCollection {
            project_path: f.project(),
            expected_revision: saved.revision,
            collection
        })
        .await
        .is_err());
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "{ invalid external edit"
    );
}
#[cfg(unix)]
#[tokio::test]
async fn api_client_collection_refuses_symlink_components_and_file_targets() {
    let f = fixture().await;
    let external = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(external.path(), f.directory.path().join(".bruhs")).unwrap();
    assert!(f.host.collection(f.project()).await.is_err());
    assert!(f
        .host
        .save(SaveApiCollection {
            project_path: f.project(),
            expected_revision: None,
            collection: ApiCollection::default()
        })
        .await
        .is_err());
    assert_eq!(std::fs::read_dir(external.path()).unwrap().count(), 0);
    let another = fixture().await;
    std::fs::create_dir_all(another.directory.path().join(".bruhs/api")).unwrap();
    let outside = external.path().join("outside.json");
    std::fs::write(&outside, "do not replace").unwrap();
    std::os::unix::fs::symlink(&outside, another.directory.path().join(COLLECTION_FILE)).unwrap();
    assert!(another.host.collection(another.project()).await.is_err());
    assert_eq!(std::fs::read_to_string(outside).unwrap(), "do not replace");
}
#[tokio::test]
async fn api_client_validation_blocks_private_ports_url_secrets_and_literal_sensitive_headers() {
    let f = fixture().await;
    let mut request = f.execution("/echo");
    request.request.headers.push(ApiHeader {
        name: "Authorization".into(),
        value: "Bearer literal-must-not-be-saved".into(),
    });
    assert!(f.host.send(request).await.is_err());
    let mut request = f.execution("/echo");
    request.request.url = "http://user:password@example.test/".into();
    assert!(f.host.send(request).await.is_err());
    let mut request = f.execution("/echo");
    request.request.url = "http://{{key}}/".into();
    request
        .environment
        .secrets
        .insert("key".into(), "named key".into());
    assert!(f.host.send(request).await.is_err());
    let port = reqwest::Url::parse(&f.url).unwrap().port().unwrap();
    let private = ApiClientHost::new(crate::secrets::SecretStore::in_memory(), vec![port]).unwrap();
    assert!(private.send(f.execution("/echo")).await.is_err());
    assert_eq!(f.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn api_client_redacts_exact_secret_reflections_in_response_header_names() {
    let f = fixture().await;
    let key = "header-name-canary";
    f.host
        .save_credential(ApiCredentialInput {
            project_path: f.project(),
            name: "header fixture".into(),
            value: key.into(),
        })
        .await
        .unwrap();
    let mut request = f.execution("/header-name");
    request
        .environment
        .secrets
        .insert("key".into(), "header fixture".into());
    request.request.headers.push(ApiHeader {
        name: "X-Api-Key".into(),
        value: "{{key}}".into(),
    });
    let result = f.host.send(request).await.unwrap();
    assert!(result.redacted);
    assert!(!serde_json::to_string(&result).unwrap().contains(key));
    assert!(result
        .headers
        .iter()
        .any(|header| header.name == "x-echo-[redacted]"));
}

#[tokio::test]
async fn api_client_abandoned_approval_finishes_and_proxy_aliases_are_blocked() {
    let f = fixture().await;
    let request = f.execution("/slow");
    let scope = Uuid::new_v4();
    let proposal = f
        .host
        .propose(ProposeApiRequest {
            scope,
            thread_id: "aborted-turn".into(),
            project_path: f.project(),
            request: request.request,
            environment: request.environment,
        })
        .await
        .unwrap();
    let host = f.host.clone();
    let project = f.project();
    let id = proposal.id;
    let decision = tokio::spawn(async move {
        host.decide(ApiProposalDecision {
            project_path: project,
            id,
            approve: true,
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), f.slow.notified())
        .await
        .unwrap();
    decision.abort();
    assert!(decision.await.unwrap_err().is_cancelled());
    assert!(matches!(
        f.host
            .proposal_result(f.project(), scope, "aborted-turn", id)
            .await
            .unwrap()
            .state,
        ApiProposalState::Failed { .. }
    ));
    assert!(f.host.send(f.execution("/echo")).await.is_ok());
    for destination in [
        "https://api.bruhs.localhost/api/credentials",
        "http://api.bruhs.localhost:1355/api/api-client/decision",
        "https://bruhs.localhost/",
    ] {
        let mut request = f.execution("/echo");
        request.request.url = destination.into();
        assert!(matches!(
            f.host.send(request).await,
            Err(ApiClientError::Invalid(_))
        ));
    }
    let host = ApiClientHost::with_origins(
        crate::secrets::SecretStore::in_memory(),
        vec![],
        vec!["https://custom-api.localhost:7443".into()],
    )
    .unwrap();
    let mut request = f.execution("/echo");
    request.request.url = "https://custom-api.localhost:7443/api/credentials".into();
    assert!(matches!(
        host.send(request).await,
        Err(ApiClientError::Invalid(_))
    ));
}

#[tokio::test]
async fn api_client_imported_curl_preserves_the_http_content_type() {
    let f = fixture().await;
    for (options, expected) in [
        ("-d 'a=b'", "application/x-www-form-urlencoded"),
        (
            "--json '{}' -H 'Content-Type: application/vnd.api+json'",
            "application/vnd.api+json",
        ),
    ] {
        let request = import_curl(&format!("curl '{}/echo' {options}", f.url)).unwrap();
        let result = f
            .host
            .send(ApiExecution {
                project_path: f.project(),
                request_id: Uuid::new_v4(),
                request,
                environment: ApiEnvironment::default(),
            })
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&result.body).unwrap();
        assert_eq!(body["content_type"], expected);
    }
}
