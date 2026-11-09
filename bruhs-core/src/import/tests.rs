use super::*;
use bruhs_models::agent::AgentProvider;
use bruhs_models::agent::Role;
use bruhs_models::logic::portable::PortablePart;
use serde_json::json;

fn write_lines(path: &Path, rows: &[serde_json::Value]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text: String = rows.iter().map(|row| format!("{row}\n")).collect();
    std::fs::write(path, text).unwrap();
}

fn claude_session(roots: &ImportRoots, id: &str, cwd: &str, entrypoint: &str) {
    write_lines(
        &roots
            .claude
            .join("projects")
            .join(cwd.replace('/', "-"))
            .join(format!("{id}.jsonl")),
        &[
            json!({"type":"user","cwd":cwd,"entrypoint":entrypoint,"isMeta":true,"message":{"role":"user","content":"<local-command-caveat>x</local-command-caveat>"}}),
            json!({"type":"user","cwd":cwd,"entrypoint":entrypoint,"timestamp":"2026-11-08T10:00:00.000Z","message":{"role":"user","content":"Fix the paging bug<system-reminder>ignore me</system-reminder>"}}),
            json!({"type":"assistant","timestamp":"2026-11-08T10:00:01.000Z","message":{"id":"m1","model":"claude-haiku-5-5","content":[{"type":"thinking","thinking":"look at api.py"}]}}),
            json!({"type":"assistant","message":{"id":"m1","model":"claude-haiku-5-5","content":[{"type":"tool_use","id":"tu1","name":"Bash","input":{"command":"pytest -q"}}]}}),
            json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"1 failed","is_error":false}]}}),
            json!({"type":"assistant","isSidechain":true,"message":{"id":"side","content":[{"type":"text","text":"subagent noise"}]}}),
            json!({"type":"assistant","message":{"id":"m2","model":"claude-haiku-5-5","content":[{"type":"text","text":"Fixed."}]}}),
            json!({"type":"user","message":{"role":"user","content":[{"type":"text","text":"Thanks"}]}}),
        ],
    );
}

#[test]
fn a_claude_code_session_imports_as_a_resumable_thread() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    claude_session(&roots, "sess-1", "/work/shop", "cli");
    claude_session(&roots, "sess-t3", "/work/shop", "t3-code");

    let listed = list(&roots, ImportSource::ClaudeCode, None, None).unwrap();
    assert_eq!(listed.len(), 1, "T3's sessions are listed under T3");
    assert_eq!(listed[0].id, "sess-1");
    assert_eq!(listed[0].title, "Fix the paging bug");
    assert_eq!(listed[0].cwd.as_deref(), Some("/work/shop"));
    assert!(
        list(&roots, ImportSource::ClaudeCode, Some("SHOP"), None)
            .unwrap()
            .len()
            == 1
    );
    assert!(
        list(&roots, ImportSource::ClaudeCode, Some("nothing"), None)
            .unwrap()
            .is_empty()
    );

    let thread = load(&roots, ImportSource::ClaudeCode, "sess-1").unwrap();
    assert_eq!(thread.candidate.session_id.as_deref(), Some("sess-1"));
    assert_eq!(thread.candidate.provider, Some(AgentProvider::Claude));
    assert_eq!(thread.candidate.model.as_deref(), Some("claude-haiku-5-5"));
    assert_eq!(
        thread.context, None,
        "a resumed session needs no transcript"
    );
    let roles: Vec<Role> = thread.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::User]);
    assert_eq!(thread.messages[0].text(), "Fix the paging bug");
    assert_eq!(thread.messages[0].started_at_ms, Some(1_794_132_000_000));
    let parts = &thread.messages[1].parts;
    assert!(matches!(&parts[0], PortablePart::Thinking(t) if t == "look at api.py"));
    assert!(
        matches!(&parts[1], PortablePart::ToolCall(c) if c.output.as_deref() == Some("1 failed"))
    );
    assert!(matches!(&parts[2], PortablePart::Text(t) if t == "Fixed."));
    assert_eq!(parts.len(), 3, "the subagent's messages stay out");

    assert!(matches!(
        load(&roots, ImportSource::ClaudeCode, "../etc"),
        Err(ImportError::NotFound { .. })
    ));
}

fn codex_rollout(roots: &ImportRoots, id: &str, originator: &str, rows: &[serde_json::Value]) {
    let mut all = vec![
        json!({"type":"session_meta","payload":{"id":id,"cwd":"/work/api","originator":originator}}),
    ];
    all.extend_from_slice(rows);
    write_lines(
        &roots.codex.join(format!(
            "sessions/2026/11/08/rollout-2026-11-08T10-00-00-{id}.jsonl"
        )),
        &all,
    );
}

#[test]
fn codex_rollouts_of_both_layouts_import() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    // Newer layout: the prompt is a response_item, after injected context.
    codex_rollout(
        &roots,
        "019a-new",
        "codex_exec",
        &[
            json!({"type":"turn_context","payload":{"model":"gpt-6.1-sol"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<t3_code_orchestration>"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions for /work/api"},{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}),
            json!({"type":"response_item","timestamp":"2026-11-08T10:00:00.000Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add a timeout"}]}}),
            json!({"type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"**Finding the client**"}]}}),
            json!({"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"rg\",\"timeout\"]}","call_id":"c1"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"client.py:3"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}),
        ],
    );
    // Older layout: prompts are user_message events (response_item user
    // messages then repeat them and must not double up).
    codex_rollout(
        &roots,
        "019a-old",
        "codex_cli_rs",
        &[
            json!({"type":"event_msg","payload":{"type":"user_message","message":"Rename the field"}}),
            json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Rename the field"}]}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Renamed."}]}}),
        ],
    );
    codex_rollout(
        &roots,
        "019a-t3",
        "T3 Code",
        &[json!({"type":"event_msg","payload":{"type":"user_message","message":"hi"}})],
    );

    let mut ids: Vec<String> = list(&roots, ImportSource::Codex, None, None)
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    ids.sort();
    assert_eq!(ids, ["019a-new", "019a-old"]);

    let new = load(&roots, ImportSource::Codex, "019a-new").unwrap();
    assert_eq!(new.candidate.title, "Add a timeout");
    assert_eq!(new.candidate.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(new.candidate.cwd.as_deref(), Some("/work/api"));
    assert_eq!(new.messages.len(), 2);
    let parts = &new.messages[1].parts;
    assert!(
        matches!(&parts[1], PortablePart::ToolCall(c) if c.tool == "shell" && c.output.as_deref() == Some("client.py:3"))
    );
    assert!(matches!(&parts[2], PortablePart::Text(t) if t == "Done."));

    let old = load(&roots, ImportSource::Codex, "019a-old").unwrap();
    let roles: Vec<Role> = old.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant]);
}

#[test]
fn a_t3_thread_imports_its_session_with_t3s_title() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    claude_session(&roots, "native-1", "/work/shop-worktree", "t3-code");
    let db = roots.t3.join("userdata/statev2.sqlite");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE orchestration_v2_projection_threads (thread_id TEXT, project_id TEXT, title TEXT, active_provider_thread_id TEXT, updated_at TEXT, archived_at TEXT, deleted_at TEXT);
             CREATE TABLE projection_projects (project_id TEXT, workspace_root TEXT);
             CREATE TABLE orchestration_v2_projection_provider_threads (provider_thread_id TEXT, thread_id TEXT, updated_at TEXT, payload_json TEXT);
             INSERT INTO projection_projects VALUES ('p1', '/work/shop');
             INSERT INTO orchestration_v2_projection_threads VALUES ('t-1', 'p1', 'Paging fix', 'pt-1', '2026-11-08T10:00:00.000Z', NULL, NULL);
             INSERT INTO orchestration_v2_projection_threads VALUES ('t-gone', 'p1', 'Deleted', 'pt-2', '2026-11-08T10:00:00.000Z', NULL, '2026-11-08');
             INSERT INTO orchestration_v2_projection_provider_threads VALUES ('pt-1', 't-1', '2026-11-08', '{\"nativeThreadRef\":{\"driver\":\"claudeAgent\",\"nativeId\":\"native-1\"}}');
             INSERT INTO orchestration_v2_projection_provider_threads VALUES ('pt-2', 't-gone', '2026-11-08', '{\"nativeThreadRef\":{\"driver\":\"claudeAgent\",\"nativeId\":\"native-1\"}}');",
        )
        .unwrap();
    drop(connection);

    let listed = list(&roots, ImportSource::T3, None, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].title, "Paging fix");
    assert_eq!(listed[0].session_id.as_deref(), Some("native-1"));

    let thread = load(&roots, ImportSource::T3, "t-1").unwrap();
    assert_eq!(thread.candidate.source, ImportSource::T3);
    assert_eq!(thread.candidate.title, "Paging fix");
    assert_eq!(
        thread.candidate.cwd.as_deref(),
        Some("/work/shop-worktree"),
        "the session's own directory"
    );
    assert_eq!(thread.messages.len(), 3);
}

#[test]
fn a_cursor_chat_imports_as_a_transcript() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    let db = roots.cursor.join("globalStorage/state.vscdb");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch("CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);")
        .unwrap();
    let put = |key: &str, value: serde_json::Value| {
        connection
            .execute(
                "INSERT INTO cursorDiskKV VALUES (?1, ?2)",
                [key, &value.to_string()],
            )
            .unwrap();
    };
    put(
        "composerData:c-1",
        json!({"composerId":"c-1","name":"Gateway update","lastUpdatedAt":1_769_472_517_801_i64,
               "fullConversationHeadersOnly":[{"bubbleId":"b1","type":1},{"bubbleId":"b2","type":2}]}),
    );
    put(
        "composerData:empty",
        json!({"composerId":"empty","fullConversationHeadersOnly":[]}),
    );
    put(
        "bubbleId:c-1:b1",
        json!({"type":1,"text":"use gateway() from ai sdk","createdAt":"2026-01-27T00:08:37.804Z"}),
    );
    put(
        "bubbleId:c-1:b2",
        json!({"type":2,"text":"Switched to gateway().","thinking":{"text":"read the agent"},
               "toolFormerData":{"toolCallId":"t1","name":"read_file_v2","rawArgs":"{\"path\":\"lib/agent.ts\"}","result":"{}","status":"completed"}}),
    );
    drop(connection);
    let workspace = roots.cursor.join("workspaceStorage/abc");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("workspace.json"),
        r#"{"folder":"file:///work/my%20app"}"#,
    )
    .unwrap();
    let ws = rusqlite::Connection::open(workspace.join("state.vscdb")).unwrap();
    ws.execute_batch(
        r#"CREATE TABLE ItemTable (key TEXT, value BLOB);
           INSERT INTO ItemTable VALUES ('composer.composerData', '{"allComposers":[{"composerId":"c-1"}]}');"#,
    )
    .unwrap();
    drop(ws);

    let listed = list(&roots, ImportSource::Cursor, None, None).unwrap();
    assert_eq!(listed.len(), 1, "an empty chat is not listed");
    assert_eq!(listed[0].title, "Gateway update");
    assert_eq!(listed[0].cwd.as_deref(), Some("/work/my app"));
    assert_eq!(listed[0].session_id, None);

    let thread = load(&roots, ImportSource::Cursor, "c-1").unwrap();
    assert_eq!(thread.messages.len(), 2);
    let parts = &thread.messages[1].parts;
    assert!(matches!(&parts[1], PortablePart::ToolCall(c) if c.tool == "read_file_v2"));
    let context = thread
        .context
        .expect("no session: the transcript goes along");
    assert!(
        context.contains("use gateway() from ai sdk") && context.contains("Switched to gateway().")
    );
}

#[test]
fn a_tool_that_is_not_installed_has_no_threads() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    for source in ImportSource::ALL {
        assert!(
            list(&roots, source, None, None).unwrap().is_empty(),
            "{source:?}"
        );
    }
}

/// Reads this machine's real stores: `cargo test -p bruhs-core import::tests::real -- --ignored --nocapture`.
#[test]
#[ignore = "reads the real ~/.claude, ~/.codex, ~/.t3 and Cursor data"]
fn real_stores_list_and_load() {
    let roots = ImportRoots::from_env();
    for source in ImportSource::ALL {
        let started = std::time::Instant::now();
        let listed = list(&roots, source, None, Some(20)).unwrap();
        println!(
            "{}: {} listed in {:?}",
            source.label(),
            listed.len(),
            started.elapsed()
        );
        for candidate in listed.iter().take(3) {
            let thread = load(&roots, source, &candidate.id).unwrap();
            println!(
                "  {:?} | {} | {} messages | session {:?} | model {:?}",
                candidate.title,
                candidate.cwd.as_deref().unwrap_or("-"),
                thread.messages.len(),
                thread.candidate.session_id,
                thread.candidate.model
            );
        }
    }
}

#[test]
fn injected_blocks_are_not_part_of_the_prompt() {
    assert_eq!(
        user_text("<bruhs_harness>\nuse the broker\n</bruhs_harness>\n\nAdd a timeout"),
        "Add a timeout"
    );
    assert_eq!(
        user_text("hi<system-reminder>x</system-reminder> there"),
        "hi there"
    );
    assert_eq!(
        user_text("<environment_context>cwd</environment_context>"),
        ""
    );
}

/// Starts a real T3 thread: run against an isolated T3 server
/// (`t3 serve --base-dir D`) with `T3CODE_HOME=D BRUHS_T3_EXPORT_DIR=<project>`.
#[test]
#[ignore = "starts a thread in the T3 server T3CODE_HOME points at"]
fn real_export_to_t3() {
    let project = std::env::var("BRUHS_T3_EXPORT_DIR").expect("BRUHS_T3_EXPORT_DIR");
    let id = export_to_t3(
        Path::new(&project),
        "Exported from bruhs",
        "claude-haiku-4-5",
        "# Paging\n\n## User\n\nFix the paging bug\n\n## Assistant\n\nFixed: the last page is kept.\n",
    )
    .unwrap();
    println!("T3 thread {id}");
    assert!(!id.is_empty());
}

/// One line that is not UTF-8 is skipped, not the whole transcript.
#[test]
fn a_line_that_is_not_utf8_is_skipped() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    claude_session(&roots, "sess-1", "/work/shop", "cli");
    let path = roots.claude.join("projects/-work-shop/sess-1.jsonl");
    let mut bytes = b"{\"type\":\"user\",\"x\":\"\xff\xfe\"}\n".to_vec();
    bytes.extend(std::fs::read(&path).unwrap());
    std::fs::write(&path, bytes).unwrap();

    let thread = load(&roots, ImportSource::ClaudeCode, "sess-1").unwrap();
    assert_eq!(thread.messages[0].text(), "Fix the paging bug");
    assert_eq!(
        list(&roots, ImportSource::ClaudeCode, None, None)
            .unwrap()
            .len(),
        1
    );
}

/// Chats Cursor keeps only as agent transcripts (the Cursor Agent CLI's)
/// are listed under Cursor; a transcript of an editor chat is not listed
/// twice. Without Cursor's editor installed, the transcripts still are.
#[test]
fn cursor_agent_transcripts_import_without_the_editor() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    // The chat's directory, encoded the way Cursor names its folder.
    let project = home.path().join("work/my_app");
    std::fs::create_dir_all(&project).unwrap();
    let project = project.canonicalize().unwrap();
    let folder: String = project
        .display()
        .to_string()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_start_matches('-')
        .to_string();
    let transcripts = roots
        .cursor_home
        .join("projects")
        .join(&folder)
        .join("agent-transcripts");
    write_lines(
        &transcripts.join("cli-1/cli-1.jsonl"),
        &[
            json!({"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Friday</timestamp>\n<user_query>\nwhy is signup failing\n</user_query>"}]}}),
            json!({"role":"assistant","message":{"content":[{"type":"text","text":"Checking the auth route."},{"type":"tool_use","name":"Read","input":{"path":"auth.ts"}}]}}),
            json!({"role":"assistant","message":{"content":[{"type":"text","text":"The callback URL is wrong."}]}}),
        ],
    );
    write_lines(
        &transcripts.join("flat-2.jsonl"),
        &[json!({"role":"user","message":{"content":[{"type":"text","text":"second chat"}]}})],
    );

    let listed = list(&roots, ImportSource::Cursor, None, None).unwrap();
    let ids: Vec<&str> = listed.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    let cli = listed.iter().find(|row| row.id == "cli-1").unwrap();
    assert_eq!(cli.title, "why is signup failing");
    assert_eq!(
        cli.cwd.as_deref(),
        Some(project.display().to_string().as_str())
    );
    assert_eq!(cli.session_id, None);

    let thread = load(&roots, ImportSource::Cursor, "cli-1").unwrap();
    let roles: Vec<Role> = thread.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant]);
    let parts = &thread.messages[1].parts;
    assert!(matches!(&parts[1], PortablePart::ToolCall(c) if c.tool == "Read"));
    assert!(matches!(&parts[2], PortablePart::Text(t) if t == "The callback URL is wrong."));
    assert!(thread
        .context
        .is_some_and(|context| context.contains("why is signup failing")));
    assert!(matches!(
        load(&roots, ImportSource::Cursor, "../cli-1"),
        Err(ImportError::NotFound { .. })
    ));

    // The editor has `flat-2`: its transcript is not listed again.
    let db = roots.cursor.join("globalStorage/state.vscdb");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            r#"CREATE TABLE cursorDiskKV (key TEXT PRIMARY KEY, value BLOB);
               INSERT INTO cursorDiskKV VALUES ('composerData:flat-2', '{"fullConversationHeadersOnly":[]}');"#,
        )
        .unwrap();
    drop(connection);
    let listed = list(&roots, ImportSource::Cursor, None, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, "cli-1");
}

/// A renamed Claude Code session lists under its newest title, which
/// Claude Code repeats near the end of the transcript.
#[test]
fn a_renamed_claude_session_lists_under_its_newest_title() {
    let home = tempfile::tempdir().unwrap();
    let roots = ImportRoots::under(home.path());
    claude_session(&roots, "sess-1", "/work/shop", "cli");
    let path = roots.claude.join("projects/-work-shop/sess-1.jsonl");
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "{}\n{}\n",
        json!({"type":"custom-title","customTitle":"Old name"}),
        json!({"type":"custom-title","customTitle":"Paging fix"})
    ));
    std::fs::write(&path, text).unwrap();

    let listed = list(&roots, ImportSource::ClaudeCode, None, None).unwrap();
    assert_eq!(listed[0].title, "Paging fix");
    let thread = load(&roots, ImportSource::ClaudeCode, "sess-1").unwrap();
    assert_eq!(thread.candidate.title, "Paging fix");
}
