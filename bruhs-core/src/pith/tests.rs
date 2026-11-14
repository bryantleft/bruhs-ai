use super::*;

fn connection() -> Connection {
    Connection::new(
        "/Applications/Pith Test.app/Contents/MacOS/storage-scanner".into(),
        "/Users/person/Folder \" quotes $(literal)".into(),
    )
    .unwrap()
}

#[tokio::test]
async fn copied_configuration_round_trips_and_removal_disables_the_next_load() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pith-mcp.json");
    assert!(Connection::load(&path).await.unwrap().is_none());
    connection().save(&path).unwrap();
    assert_eq!(Connection::load(&path).await.unwrap(), Some(connection()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_file(path.clone()).unwrap();
    assert!(Connection::load(&path).await.unwrap().is_none());
}

#[tokio::test]
async fn malformed_oversized_or_command_widening_configuration_fails_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("pith-mcp.json");
    for config in [
        json!({"mcpServers":{"pith":{"command":"relative-scanner","args":["mcp","--root","/Users/person"]}}}),
        json!({"mcpServers":{"pith":{"command":"/bin/scanner","args":["trash","--path","/Users/person"]}}}),
        json!({"mcpServers":{"pith":{"command":"/bin/scanner","args":["mcp","--root","relative"]}}}),
        json!({"mcpServers":{"pith":{"command":"/bin/scanner","args":["mcp","--root","/Users/person"],"env":{"PRIVATE":"no"}}}}),
        json!({"mcpServers":{"different":{"command":"/bin/scanner","args":[]}}}),
    ] {
        std::fs::write(&path, config.to_string()).unwrap();
        assert!(Connection::load(&path).await.is_err(), "{config}");
    }
    std::fs::write(&path, vec![b' '; 65537]).unwrap();
    assert!(Connection::load(&path)
        .await
        .unwrap_err()
        .to_string()
        .contains("64 KiB"));
    assert!(
        serde_json::from_value::<Connection>(json!({"command":"/bin/scanner","args":[]})).is_err(),
        "a Connection cannot exist without a validated root"
    );
}

#[test]
fn claude_lean_turns_keep_pith_existing_brokers_and_only_read_tool_approvals() {
    let mut options = claude_sdk_rs::QueryOptions::new("Find cleanup candidates");
    crate::agent::lean_claude_settings(&mut options, &[]);
    options.mcp_config = Some(json!({"mcpServers":{"bruhs_secrets":{"command":"existing-helper"},"bruhs_threads":{"command":"threads-helper"}}}).to_string());
    options.tools = claude_sdk_rs::Tools::Allow(vec!["Read".into()]);
    let mut query = ProviderQueryOptions::Claude(options);
    connection().configure(&mut query).unwrap();
    let ProviderQueryOptions::Claude(options) = query else {
        panic!("Claude options must stay Claude");
    };
    let config: Value = serde_json::from_str(options.mcp_config.as_deref().unwrap()).unwrap();
    assert_eq!(
        config["mcpServers"]["pith"]["args"][2],
        connection().root().to_str().unwrap()
    );
    assert_eq!(
        config["mcpServers"]["bruhs_secrets"]["command"],
        "existing-helper"
    );
    assert_eq!(
        config["mcpServers"]["bruhs_threads"]["command"],
        "threads-helper"
    );
    assert_eq!(
        options.setting_sources,
        Some(Vec::new()),
        "Pith must work without loading user plugins"
    );
    let claude_sdk_rs::Tools::Allow(tools) = options.tools else {
        panic!("Pith reads must be pre-approved");
    };
    assert_eq!(tools.len(), 6);
    assert!(tools.contains(&"Read".into()));
    assert!(TOOLS
        .iter()
        .all(|tool| tools.contains(&format!("mcp__pith__{tool}"))));
    assert!(options
        .append_system_prompt
        .unwrap()
        .contains("allocated size is not reclaimable space"));
}

#[test]
fn transcript_only_claude_runs_never_gain_pith_tools() {
    let mut query = ProviderQueryOptions::Claude(
        claude_sdk_rs::QueryOptions::new("Summarize").tools(claude_sdk_rs::Tools::None),
    );
    connection().configure(&mut query).unwrap();
    let ProviderQueryOptions::Claude(options) = query else {
        panic!("Claude options must stay Claude");
    };
    assert_eq!(options.tools, claude_sdk_rs::Tools::None);
    assert!(options.mcp_config.is_none());
}

#[test]
fn codex_adds_the_same_scoped_server_without_replacing_user_or_private_settings() {
    let mut options = codex_sdk_rs::QueryOptions::new("Find cleanup candidates");
    options.config.push(
        codex_sdk_rs::ConfigOverride::new(
            "mcp_servers.bruhs_secrets",
            "{ command = \"existing-helper\" }",
        )
        .unwrap(),
    );
    crate::agent::lean_codex(&mut options);
    let mut query = ProviderQueryOptions::Codex(options);
    connection().configure(&mut query).unwrap();
    let ProviderQueryOptions::Codex(options) = query else {
        panic!("Codex options must stay Codex");
    };
    assert_eq!(options.config.len(), 3);
    assert_eq!(options.config[0].key(), "mcp_servers.bruhs_secrets");
    assert_eq!(options.config[1].key(), "features.plugins");
    assert_eq!(options.config[2].key(), "mcp_servers.pith");
    assert!(options.prompt.contains("Do not delete"));
    assert!(options.prompt.ends_with("Find cleanup candidates"));
}
