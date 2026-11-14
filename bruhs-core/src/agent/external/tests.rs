use super::*;

#[test]
fn opencode_access_modes_keep_existing_tool_and_path_denials() {
    let mut policy = json!({"*":"allow","read":"allow"});
    preserve_denials(&mut policy, &json!({"*":"deny"}));
    assert_eq!(policy["read"], "deny");
    for mode in [
        PermissionMode::Plan,
        PermissionMode::AcceptEdits,
        PermissionMode::BypassPermissions,
    ] {
        let mut options = Options::new(
            AgentProvider::OpenCode,
            "hello",
            "/tmp",
            "opencode/default",
            mode,
            None,
            None,
        );
        options.permission_defaults = vec![json!({"read":{"*.env":"deny"},"bash":"deny"})];
        let mut command = Command::new("fixture");
        opencode_config(&options, &mut command).unwrap();
        let permission = command
            .as_std()
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_PERMISSION")
            .and_then(|(_, value)| value)
            .unwrap();
        let applied: Value = serde_json::from_str(permission.to_str().unwrap()).unwrap();
        assert_eq!(applied["read"]["*.env"], "deny");
        assert_eq!(applied["bash"], "deny");
    }
}

#[test]
fn opencode_policy_clone_preserves_the_configured_agent_model_and_prompt() {
    let config = json!({"default_agent":"writer","agent":{"writer":{"model":"xai/grok-code-fast-1","prompt":"Keep these instructions.","permission":{"*":"allow"}}}});
    let mut options = Options::new(
        AgentProvider::OpenCode,
        "hello",
        "/tmp",
        "opencode/default",
        PermissionMode::Plan,
        None,
        None,
    );
    options.agent_defaults = configured_opencode_agent(&config).unwrap();
    let mut command = Command::new("fixture");
    let name = opencode_config(&options, &mut command).unwrap();
    let content = command
        .as_std()
        .get_envs()
        .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
        .and_then(|(_, value)| value)
        .unwrap();
    let applied: Value = serde_json::from_str(content.to_str().unwrap()).unwrap();
    assert_eq!(applied["agent"][&name]["model"], "xai/grok-code-fast-1");
    assert_eq!(
        applied["agent"][&name]["prompt"],
        "Keep these instructions."
    );
    assert_eq!(applied["agent"][&name]["permission"]["*"], "deny");
    assert!(configured_opencode_agent(&json!({"default_agent":"missing"})).is_err());
}

fn parser(provider: AgentProvider) -> Parser {
    Parser::new(
        provider,
        &format!("{}/default", provider.as_str()),
        PermissionMode::Default,
        &HashMap::new(),
    )
}

#[test]
fn cursor_streams_only_new_deltas_and_preserves_runtime_identity() {
    let mut parser = parser(AgentProvider::Cursor);
    let init = parser.parse(
        json!({"type":"system","subtype":"init","session_id":"cursor-session","model":"sonnet"}),
    );
    assert!(matches!(&init[0],QueryEvent::Init{model:Some(model),..} if model=="cursor/default"));
    let delta=parser.parse(json!({"type":"assistant","timestamp_ms":1,"message":{"content":[{"type":"text","text":"hello"}]}}));
    assert!(matches!(&delta[0],QueryEvent::Text(text) if text=="hello"));
    assert!(parser.parse(json!({"type":"assistant","timestamp_ms":2,"model_call_id":"call-1","message":{"content":[{"type":"text","text":"hello"}]}})).is_empty());
    assert!(parser
        .parse(json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}))
        .is_empty());
    assert_eq!(parser.text, "hello");
}

#[test]
fn cursor_message_level_streams_support_old_cli_without_timestamp_fields() {
    let mut parser = parser(AgentProvider::Cursor);
    let events = parser
        .parse(json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}));
    assert!(matches!(&events[0],QueryEvent::Text(text) if text=="hello"));
    let options = Options::new(
        AgentProvider::Cursor,
        "hello",
        "/tmp",
        "cursor/default",
        PermissionMode::Default,
        None,
        None,
    );
    let prepared = options.prepare().unwrap();
    assert!(!prepared
        .command
        .as_std()
        .get_args()
        .any(|arg| arg == "--stream-partial-output"));
}

#[test]
fn grok_maps_messages_tool_calls_and_errors() {
    let mut parser = parser(AgentProvider::Grok);
    let events=parser.parse(json!({"type":"assistant","message":{"content":[{"type":"text","text":"Reading"},{"type":"tool_use","id":"t1","name":"read_file","input":{"path":"a.rs"}}]}}));
    assert!(matches!(&events[0],QueryEvent::Text(text) if text=="Reading"));
    assert!(matches!(&events[1],QueryEvent::ToolCall{id,..} if id=="t1"));
    let result=parser.parse(json!({"type":"result","subtype":"error_during_execution","errors":["rate limited"],"session_id":"s1","usage":{"input_tokens":5}}));
    assert!(
        matches!(&result[0],QueryEvent::Result{is_error:true,result,session_id,..} if result.contains("rate limited") && session_id=="s1")
    );
}

#[test]
fn subagent_transcript_does_not_append_to_the_main_answer() {
    let mut parser = parser(AgentProvider::Grok);
    let events=parser.parse(json!({"type":"assistant","parent_tool_use_id":"task1","message":{"content":[{"type":"text","text":"subagent answer"}]}}));
    assert!(matches!(&events[0], QueryEvent::Raw(_)));
    assert!(parser.text.is_empty());
}

#[test]
fn antigravity_maps_deltas_tool_results_and_disjoint_cache_usage() {
    let mut parser = parser(AgentProvider::Antigravity);
    parser.parse(json!({"event":"init","conversation_id":"a1","init":{"tools":["read_file"]}}));
    let delta=parser.parse(json!({"event":"step_update","step_update":{"step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"hi"}}));
    assert!(matches!(&delta[0],QueryEvent::Text(text) if text=="hi"));
    let tool=parser.parse(json!({"event":"step_update","step_update":{"step_index":2,"state":"DONE","step_type":"tool","tool_info":{"name":"read_file","parameters":{"path":"a.rs"},"output":"code"}}}));
    assert!(matches!(&tool[0],QueryEvent::ToolCall{id,..} if id=="step-2"));
    assert!(matches!(&tool[1],QueryEvent::ToolResult{content,..} if content=="code"));
    let result=parser.parse(json!({"event":"result","result":{"conversation_id":"a1","status":"SUCCESS","response":"hi","duration_seconds":1.25,"usage":{"input_tokens":100,"cache_read_tokens":80,"output_tokens":4}}}));
    assert!(
        matches!(&result[0],QueryEvent::Result{duration_ms:1250,usage,..} if usage.input_tokens==20 && usage.cache_read_input_tokens==80)
    );
}

#[test]
fn opencode_retains_provider_model_and_session() {
    let mut parser = parser(AgentProvider::OpenCode);
    let events =
        parser.parse(json!({"type":"text","sessionID":"oc1","part":{"id":"p1","text":"hello"}}));
    assert!(
        matches!(&events[0],QueryEvent::Init{session_id,model:Some(model),..} if session_id=="oc1" && model=="opencode/default")
    );
    assert!(matches!(&events[1],QueryEvent::Text(text) if text=="hello"));
    assert!(parser
        .parse(json!({"type":"text","sessionID":"oc1","part":{"id":"p1","text":"hello"}}))
        .is_empty());
}

#[test]
fn prompts_and_api_keys_are_not_command_arguments() {
    for provider in [
        AgentProvider::Cursor,
        AgentProvider::Antigravity,
        AgentProvider::Grok,
        AgentProvider::OpenCode,
    ] {
        let options = Options::new(
            provider,
            "secret prompt",
            "/tmp",
            &format!("{}/default", provider.as_str()),
            if provider == AgentProvider::Antigravity {
                PermissionMode::BypassPermissions
            } else {
                PermissionMode::Default
            },
            None,
            Some("resume-id"),
        );
        let prepared = options.prepare().unwrap();
        let args = prepared
            .command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(
            !args.iter().any(|arg| arg.contains("secret prompt")),
            "{provider:?}"
        );
        assert!(
            !args.iter().any(|arg| arg.starts_with("--model=")),
            "default must use CLI selection"
        );
    }
}

#[test]
fn read_only_config_overrides_inherited_write_permissions() {
    let options = Options::new(
        AgentProvider::Cursor,
        "hello",
        "/tmp",
        "cursor/default",
        PermissionMode::Plan,
        None,
        None,
    );
    let prepared = options.prepare().unwrap();
    let file = prepared._files.path().join("cli-config.json");
    let config: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(config["approvalMode"], "allowlist");
    assert!(config["permissions"]["deny"]
        .as_array()
        .unwrap()
        .contains(&json!("Write(**)")));
    assert!(config["permissions"]["deny"]
        .as_array()
        .unwrap()
        .contains(&json!("Shell(*)")));
    assert!(config["permissions"]["deny"]
        .as_array()
        .unwrap()
        .contains(&json!("Mcp(*:*)")));
}

#[test]
fn unattended_runs_fail_before_spawning() {
    let mut options = Options::new(
        AgentProvider::Grok,
        "hello",
        "/tmp",
        "grok/default",
        PermissionMode::Plan,
        None,
        None,
    );
    options.unattended();
    assert!(options
        .prepare()
        .err()
        .unwrap()
        .to_string()
        .contains("unattended"));
}

#[test]
fn cursor_does_not_claim_unapproved_per_turn_mcp_servers_are_available() {
    let options = Options::new(
        AgentProvider::Cursor,
        "hello",
        "/tmp",
        "cursor/default",
        PermissionMode::BypassPermissions,
        None,
        None,
    );
    assert!(!options.supports_mcp());
    let mut parser = parser(AgentProvider::Cursor);
    let events = parser.parse(json!({"type":"system","subtype":"init","session_id":"c1"}));
    assert!(
        matches!(&events[0],QueryEvent::Init{mcp_servers,..} if mcp_servers.iter().any(|server|server.status.contains("persistent MCP server approval")))
    );
}

#[tokio::test]
async fn process_fixture_reports_nonzero_exit_and_stderr() {
    let mut command = Command::new("sh");
    command.args(["-c","printf '%s\\n' '{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"fixture\"}'; printf '%s' 'fixture auth required' >&2; exit 7"]);
    let mut process = process::CliProcess::spawn("fixture", &mut command).unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel::<()>(1);
    let line = process
        .next_line(&tx)
        .await
        .unwrap_or_else(|_| panic!("fixture line missing"));
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["session_id"],
        "fixture"
    );
    let failure = process.finish(process::Ending::Eof).await.unwrap();
    assert_eq!(failure.exit_code, Some(7));
    assert!(failure.stderr_tail.contains("fixture auth required"));
}

#[test]
fn antigravity_rejects_modes_without_an_enforced_permission_boundary() {
    for mode in [
        PermissionMode::Default,
        PermissionMode::Plan,
        PermissionMode::AcceptEdits,
    ] {
        let options = Options::new(
            AgentProvider::Antigravity,
            "hello",
            "/tmp",
            "antigravity/default",
            mode,
            None,
            None,
        );
        assert!(options
            .prepare()
            .err()
            .unwrap()
            .to_string()
            .contains("requires Full access"));
    }
}

#[test]
fn grok_read_only_restricts_tools_and_overrides_inherited_allows() {
    let options = Options::new(
        AgentProvider::Grok,
        "hello",
        "/tmp",
        "grok/default",
        PermissionMode::Plan,
        None,
        None,
    );
    let prepared = options.prepare().unwrap();
    let args = prepared
        .command
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    for rule in [
        "--tools=read_file,grep,list_dir",
        "--deny=Bash",
        "--deny=Edit",
        "--deny=Write",
        "--deny=MCPTool",
        "--no-subagents",
    ] {
        assert!(args.iter().any(|arg| arg == rule), "missing {rule}");
    }
}

#[test]
fn opencode_requires_a_final_stop_step_and_sums_usage() {
    let mut parser = parser(AgentProvider::OpenCode);
    parser.parse(json!({"type":"step_start","sessionID":"oc1","part":{}}));
    assert!(!parser.completed);
    parser.parse(json!({"type":"step_finish","sessionID":"oc1","part":{"reason":"tool-calls","tokens":{"input":10,"output":3,"cache":{"read":2}}}}));
    assert!(!parser.completed);
    parser.parse(json!({"type":"step_finish","sessionID":"oc1","part":{"reason":"stop","tokens":{"input":20,"output":5,"cache":{"read":7}}}}));
    assert!(parser.completed);
    assert_eq!(parser.usage.input_tokens, 30);
    assert_eq!(parser.usage.output_tokens, 8);
    assert_eq!(parser.usage.cache_read_input_tokens, 9);
}

#[tokio::test]
async fn dropped_consumer_kills_a_silent_fixture() {
    let mut command = Command::new("sh");
    command.args(["-c", "exec sleep 30"]);
    let mut process = process::CliProcess::spawn("fixture", &mut command).unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel::<()>(1);
    drop(rx);
    assert!(matches!(
        process.next_line(&tx).await,
        Err(process::Ending::ConsumerGone)
    ));
    let failure = tokio::time::timeout(
        Duration::from_secs(2),
        process.finish(process::Ending::ConsumerGone),
    )
    .await
    .unwrap();
    assert!(failure.is_none());
}
