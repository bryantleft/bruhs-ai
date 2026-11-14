use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[tokio::test]
async fn api_client_native_http_and_mcp_require_one_shot_review_and_redact_results() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory
        .path()
        .canonicalize()
        .unwrap()
        .display()
        .to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let target=axum::Router::new().route("/graphql",axum::routing::post(move|headers:axum::http::HeaderMap,body:String|{let count=Arc::clone(&count);async move{count.fetch_add(1,Ordering::SeqCst);let key=headers.get("x-api-key").and_then(|v|v.to_str().ok()).unwrap_or("");(StatusCode::CREATED,Json(serde_json::json!({"data":{"name":"Ada"},"reflected_key":key,"request_body":body})))}}));
    let target_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target_url = format!("http://{}", target_listener.local_addr().unwrap());
    let target = tokio::spawn(async move {
        axum::serve(target_listener, target).await.unwrap();
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let host = ApiClientHost::new(
        bruhs_core::secrets::SecretStore::in_memory(),
        vec![listener.local_addr().unwrap().port()],
    )
    .unwrap();
    let config = crate::config::Config::from_settings(crate::config::Settings {
        port: Some("0"),
        web_dist: directory.path().join("missing").to_str(),
        ..Default::default()
    })
    .unwrap();
    let app = crate::router(crate::config::ServerState {
        app: Arc::new(
            bruhs_core::AppState::builder()
                .secret_store(bruhs_core::secrets::SecretStore::in_memory())
                .connections(bruhs_core::connections::Connections::in_memory())
                .build(),
        ),
        config: Arc::new(config),
        updates: bruhs_core::updates::UpdateChecker::new(bruhs_core::updates::BuildInfo::new(
            "", "", "unused",
        )),
    })
    .layer(Extension(host));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let client = crate::desktop_server_client::ServerClient::new(&base, &base).unwrap();
    let empty = client.api_collection(&project).await.unwrap();
    assert!(empty.revision.is_none());
    let secret = "native-http-fixture-canary";
    client
        .save_api_credential(&ApiCredentialInput {
            project_path: project.clone(),
            name: "API fixture".into(),
            value: secret.into(),
        })
        .await
        .unwrap();
    let request = ApiRequest {
        name: "GraphQL fixture".into(),
        method: "POST".into(),
        url: "{{base_url}}/graphql".into(),
        headers: vec![
            ApiHeader {
                name: "X-Api-Key".into(),
                value: "{{key}}".into(),
            },
            ApiHeader {
                name: "Content-Type".into(),
                value: "application/json".into(),
            },
        ],
        body: "{\"query\":\"{ viewer { name } }\"}".into(),
        ..Default::default()
    };
    let environment = ApiEnvironment {
        values: std::collections::BTreeMap::from([("base_url".into(), target_url)]),
        secrets: std::collections::BTreeMap::from([("key".into(), "API fixture".into())]),
    };
    let saved = client
        .save_api_collection(&SaveApiCollection {
            project_path: project.clone(),
            expected_revision: None,
            collection: ApiCollection {
                requests: vec![request.clone()],
                environment: environment.clone(),
                ..Default::default()
            },
        })
        .await
        .unwrap();
    assert_eq!(client.api_collection(&project).await.unwrap(), saved);
    assert!(
        !std::fs::read_to_string(directory.path().join(COLLECTION_FILE))
            .unwrap()
            .contains(secret)
    );
    let cancelled = ApiExecution {
        project_path: project.clone(),
        request_id: Uuid::new_v4(),
        request: request.clone(),
        environment: environment.clone(),
    };
    client
        .cancel_api_request(cancelled.request_id)
        .await
        .unwrap();
    assert!(client.send_api_request(&cancelled).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let response = client
        .send_api_request(&ApiExecution {
            project_path: project.clone(),
            request_id: Uuid::new_v4(),
            request: request.clone(),
            environment: environment.clone(),
        })
        .await
        .unwrap();
    assert_eq!(response.status, 201);
    assert!(response.body.contains("Ada"));
    assert!(response.body.contains("[redacted]"));
    assert!(!serde_json::to_string(&response).unwrap().contains(secret));
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let denied = http
        .post(format!("{base}/api/api-client/send"))
        .header("Origin", "https://unrelated.example")
        .json(&ApiExecution {
            project_path: project.clone(),
            request_id: Uuid::new_v4(),
            request: request.clone(),
            environment: environment.clone(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let malformed = http
        .post(format!("{base}/api/api-client/credential"))
        .json(&serde_json::json!({"value":{"secret":"must-not-echo"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    assert!(!malformed.text().await.unwrap().contains("must-not-echo"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    if let Some(executable) = std::env::var_os("BRUHS_API_MCP_EXECUTABLE") {
        let scope = Uuid::new_v4();
        let mut helper = tokio::process::Command::new(executable)
            .arg("--api-client-mcp")
            .env("BRUHS_REACTIVE_ENDPOINT", &base)
            .env("BRUHS_API_PROJECT", &project)
            .env("BRUHS_API_THREAD", "synthetic-api-thread")
            .env("BRUHS_API_SCOPE", scope.to_string())
            .env("BRUHS_API_PROPOSE", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = helper.stdin.take().unwrap();
        let mut output = tokio::io::BufReader::new(helper.stdout.take().unwrap());
        let registry = exchange(&mut input, &mut output, "tools/list", serde_json::json!({})).await;
        assert_eq!(registry["tools"].as_array().unwrap().len(), 2);
        assert!(registry.to_string().contains("api_propose_request"));
        assert!(!registry.to_string().contains("api_approve_request"));
        let malformed=exchange(&mut input,&mut output,"tools/call",serde_json::json!({"name":"api_propose_request","arguments":{"request":"invalid","environment":{}}})).await;
        assert_eq!(malformed["isError"], true);
        let mut request_value = serde_json::to_value(&request).unwrap();
        request_value.as_object_mut().unwrap().remove("id");
        let proposed=exchange(&mut input,&mut output,"tools/call",serde_json::json!({"name":"api_propose_request","arguments":{"request":request_value,"environment":environment}})).await;
        let proposed = content(&proposed);
        assert_eq!(proposed["state"], "pending");
        let id = Uuid::parse_str(proposed["id"].as_str().unwrap()).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let bypass = exchange(
            &mut input,
            &mut output,
            "tools/call",
            serde_json::json!({"name":"api_approve_request","arguments":{"id":id}}),
        )
        .await;
        assert_eq!(bypass["isError"], true);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let proposal = client
            .api_proposals(&project)
            .await
            .unwrap()
            .into_iter()
            .find(|proposal| proposal.id == id)
            .unwrap();
        assert!(proposal.destination.ends_with("/graphql"));
        assert_eq!(
            proposal.request,
            ApiRequest {
                id: proposal.request.id,
                ..request.clone()
            }
        );
        let decision = ApiProposalDecision {
            project_path: project.clone(),
            id,
            approve: true,
        };
        let approved = client.decide_api_proposal(&decision).await.unwrap();
        assert!(matches!(approved.state, ApiProposalState::Finished { .. }));
        assert!(client.decide_api_proposal(&decision).await.is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let read = exchange(
            &mut input,
            &mut output,
            "tools/call",
            serde_json::json!({"name":"api_request_status","arguments":{"id":id}}),
        )
        .await;
        assert!(!read.to_string().contains(secret));
        let read = content(&read);
        assert_eq!(read["state"], "finished");
        assert!(read["response"]["body"]
            .as_str()
            .unwrap()
            .contains("[redacted]"));
        input.shutdown().await.unwrap();
        drop(input);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), helper.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
        println!("Production API MCP helper: registry, malformed input, pending proposal without send, forbidden approval tool, owner send, replay refusal and redacted result passed.");
    }
    server.abort();
    target.abort();
}
fn content(value: &serde_json::Value) -> serde_json::Value {
    serde_json::from_str(value["content"][0]["text"].as_str().unwrap()).unwrap()
}
async fn exchange(
    input: &mut tokio::process::ChildStdin,
    output: &mut tokio::io::BufReader<tokio::process::ChildStdout>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    input.write_all(format!("{}\n",serde_json::json!({"jsonrpc":"2.0","id":Uuid::new_v4().to_string(),"method":method,"params":params})).as_bytes()).await.unwrap();
    input.flush().await.unwrap();
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        output.read_line(&mut line),
    )
    .await
    .unwrap()
    .unwrap();
    serde_json::from_str::<serde_json::Value>(&line).unwrap()["result"].clone()
}
