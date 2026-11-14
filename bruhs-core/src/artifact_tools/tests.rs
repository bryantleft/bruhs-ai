use super::*;
use crate::artifacts::PreviewState;

#[tokio::test]
async fn tool_publication_returns_a_display_fence_and_inspection_waits_for_real_feedback() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::at(directory.path().join("artifacts"));
    let project = directory.path().to_string_lossy().into_owned();
    std::fs::write(
        directory.path().join("widget.html"),
        "<button>Widget</button>",
    )
    .unwrap();
    let response = call(
        store.clone(),
        project.clone(),
        json!({"name":"publish_artifact","arguments":{"path":"widget.html","title":"Widget"}}),
    )
    .await
    .unwrap();
    assert!(response.contains("not yet previewed"));
    let marker = response
        .split("```bruhs-artifact\n")
        .nth(1)
        .unwrap()
        .split("\n```")
        .next()
        .unwrap();
    let marker: Value = serde_json::from_str(marker).unwrap();
    let id = marker["id"].as_str().unwrap();
    let pending: Value = serde_json::from_str(
        &call(
            store.clone(),
            project.clone(),
            json!({"name":"inspect_artifact","arguments":{"id":id}}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(pending["preview"], "not yet previewed");
    store
        .record_diagnostics(id, PreviewState::Error, vec!["TypeError in widget".into()])
        .unwrap();
    let checked: Value = serde_json::from_str(
        &call(
            store.clone(),
            project.clone(),
            json!({"name":"inspect_artifact","arguments":{"id":id}}),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(checked["preview"], "client reported");
    assert_eq!(checked["diagnostics"]["errors"][0], "TypeError in widget");
}

#[tokio::test]
async fn stdio_initializes_lists_tools_and_rejects_invalid_calls_without_stdout_noise() {
    let directory = tempfile::tempdir().unwrap();
    let store = ArtifactStore::at(directory.path().join("artifacts"));
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"publish_artifact\",\"arguments\":{\"path\":\"missing.html\",\"project\":\"/\"}}}\n";
    let mut output = Vec::new();
    serve(
        input.as_slice(),
        &mut output,
        store,
        directory.path().to_string_lossy().into_owned(),
    )
    .await
    .unwrap();
    let messages: Vec<Value> = std::str::from_utf8(&output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0]["result"]["serverInfo"]["name"],
        "bruhs_artifacts"
    );
    assert_eq!(
        messages[1]["result"]["tools"][0]["name"],
        "publish_artifact"
    );
    assert_eq!(messages[2]["result"]["isError"], true);
    assert!(messages[2]["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown field"));
}

#[test]
fn provider_configuration_preserves_existing_mcp_servers_and_codex_developer_settings() {
    let mut claude = claude_sdk_rs::QueryOptions::new("hello");
    claude.mcp_config = Some(json!({"mcpServers":{"existing":{"command":"keep"}}}).to_string());
    let mut options = ProviderQueryOptions::Claude(claude);
    configure(&mut options, "/project").unwrap();
    let ProviderQueryOptions::Claude(claude) = options else {
        panic!("Claude configuration")
    };
    let config: Value = serde_json::from_str(claude.mcp_config.as_deref().unwrap()).unwrap();
    assert_eq!(config["mcpServers"]["existing"]["command"], "keep");
    assert_eq!(config["mcpServers"][SERVER]["env"][PROJECT_ENV], "/project");
    let mut codex = codex_sdk_rs::QueryOptions::new("hello");
    codex
        .config
        .push(codex_sdk_rs::ConfigOverride::new("developer_instructions", "\"keep\"").unwrap());
    let mut options = ProviderQueryOptions::Codex(codex);
    configure(&mut options, "/project").unwrap();
    let ProviderQueryOptions::Codex(codex) = options else {
        panic!("Codex configuration")
    };
    assert_eq!(codex.config[0].key(), "developer_instructions");
    assert_eq!(codex.config[0].value(), "\"keep\"");
    assert!(codex
        .config
        .iter()
        .any(|config| config.key() == format!("mcp_servers.{SERVER}")));
}
