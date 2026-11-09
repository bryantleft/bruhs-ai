//! Import thread and the export actions: converting messages, where an
//! import lands, the saved session, and the palette actions end to end.

use super::import_ui::{
    import_place, import_row_detail, imported_message, imported_messages, portable_messages,
    tilde_path, visible_candidates, ImportPlace,
};
use super::persistence::restored_session;
use super::{
    DesktopSidebarTab, DesktopState, EpochMs, Message, MessageRole, PersistedThread, TopLayer,
    TurnState, Workspace,
};
use crate::theme;
use bruhs_models::agent::{AgentProvider, Role};
use bruhs_models::logic::portable::{
    ImportCandidate, ImportSource, ImportedThread, PortableMessage, PortablePart, PortableToolCall,
};
use gpui::{AppContext as _, Entity};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

fn candidate(cwd: Option<&str>, session: Option<&str>) -> ImportCandidate {
    ImportCandidate {
        source: ImportSource::ClaudeCode,
        id: "source-id".into(),
        title: "Paging fix".into(),
        cwd: cwd.map(str::to_string),
        updated_at: EpochMs::saturating_from_i64(1_700_000_000_000),
        provider: session.map(|_| AgentProvider::Claude),
        session_id: session.map(str::to_string),
        model: Some("claude-opus-4-6".into()),
    }
}

fn assistant_parts() -> Vec<PortablePart> {
    vec![
        PortablePart::Thinking("plan".into()),
        PortablePart::ToolCall(PortableToolCall {
            id: "t1".into(),
            tool: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
            output: Some("a.txt".into()),
            is_error: true,
        }),
        PortablePart::Text("done".into()),
    ]
}

#[test]
fn an_imported_assistant_message_keeps_text_thinking_and_tools() {
    let portable = PortableMessage {
        role: Role::Assistant,
        parts: assistant_parts(),
        started_at_ms: Some(1_700_000_000_000),
        model: Some("claude-haiku-4-5".into()),
    };
    let message = imported_message(&portable).expect("message");
    assert_eq!(message.role, MessageRole::Assistant);
    assert_eq!(message.content, "done");
    assert_eq!(message.thinking, "plan");
    assert_eq!(message.tool_calls.len(), 1);
    let call = &message.tool_calls[0];
    assert_eq!(call.id, "t1");
    assert_eq!(call.tool, "Bash");
    assert_eq!(call.input, serde_json::json!({"command": "ls"}));
    assert_eq!(call.output.as_deref(), Some("a.txt"));
    assert!(call.is_error);
    assert_eq!(message.started_at_ms, Some(1_700_000_000_000));
    assert_eq!(message.model.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(message.state, TurnState::Finished);

    // Thinking, tools, then text: the order desktop messages go back out in.
    assert_eq!(portable_messages(&[message]), vec![portable]);
}

#[test]
fn imported_text_parts_join_and_empty_messages_drop() {
    let messages = imported_messages(&[
        PortableMessage {
            role: Role::User,
            parts: vec![
                PortablePart::Text("first".into()),
                PortablePart::Text("second".into()),
            ],
            started_at_ms: None,
            model: None,
        },
        PortableMessage {
            role: Role::Assistant,
            parts: vec![PortablePart::Text("  ".into())],
            started_at_ms: None,
            model: None,
        },
        PortableMessage {
            role: Role::Tool,
            parts: vec![PortablePart::Text("tool text".into())],
            started_at_ms: None,
            model: None,
        },
    ]);
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, MessageRole::User);
    assert_eq!(messages[0].content, "first\n\nsecond");
    assert_eq!(messages[1].role, MessageRole::Assistant);
    assert_eq!(messages[1].content, "tool text");
}

#[test]
fn failed_and_streaming_turns_stay_out_of_an_export() {
    let mut failed = Message::assistant_failed(None, "boom");
    failed.content = "Agent error: boom".into();
    let mut streaming = Message::assistant_streaming(1, None);
    streaming.content = "half".into();
    let exported = portable_messages(&[Message::user("hi"), failed, streaming]);
    assert_eq!(exported.len(), 1);
    assert_eq!(exported[0].parts, vec![PortablePart::Text("hi".into())]);
}

#[test]
fn an_import_lands_in_its_folder_or_as_a_home_chat() {
    assert_eq!(
        import_place(Some("/Users/me/shop/"), Some("/Users/me")),
        ImportPlace {
            project_name: "shop".into(),
            run_path: Some("/Users/me/shop".into()),
        }
    );
    // A home-directory thread is a chat without a project that still runs
    // at home, where Claude Code keeps its session.
    assert_eq!(
        import_place(Some("/Users/me/"), Some("/Users/me")),
        ImportPlace {
            project_name: String::new(),
            run_path: Some("/Users/me".into()),
        }
    );
    assert_eq!(
        import_place(None, Some("/Users/me")),
        ImportPlace {
            project_name: String::new(),
            run_path: None,
        }
    );
}

#[test]
fn rows_show_home_as_a_tilde_age_and_how_they_come_in() {
    assert_eq!(tilde_path("/Users/me/shop", Some("/Users/me")), "~/shop");
    assert_eq!(tilde_path("/Users/me", Some("/Users/me/")), "~");
    assert_eq!(
        tilde_path("/Users/mee/x", Some("/Users/me")),
        "/Users/mee/x"
    );
    assert_eq!(tilde_path("/opt/x", None), "/opt/x");

    let now = EpochMs::saturating_from_i64(1_700_000_000_000 + 3 * 3_600_000);
    assert_eq!(
        import_row_detail(
            &candidate(Some("/Users/me/shop"), Some("s-1")),
            Some("/Users/me"),
            now
        ),
        "~/shop · 3h · resumes"
    );
    assert_eq!(
        import_row_detail(&candidate(None, None), Some("/Users/me"), now),
        "no folder · 3h · transcript"
    );
}

#[test]
fn candidates_already_here_are_not_offered_again() {
    let found = vec![
        candidate(None, Some("known")),
        candidate(None, Some("new")),
        candidate(None, None),
    ];
    let known: HashSet<String> = ["known".to_string()].into();
    let visible = visible_candidates(&found, &known);
    let sessions: Vec<_> = visible
        .iter()
        .map(|candidate| candidate.session_id.as_deref())
        .collect();
    assert_eq!(sessions, [Some("new"), None]);
}

#[test]
fn a_saved_session_round_trips_and_old_state_still_loads() {
    let legacy = r#"{"id":"t1","title":"Old","project_name":"","project_path":null,"model_id":"claude-sonnet-4-6","permission_mode":"default","messages":[]}"#;
    let old: PersistedThread = serde_json::from_str(legacy).expect("legacy record loads");
    assert_eq!(old.session_id, None);
    assert_eq!(old.session_provider, None);
    assert_eq!(old.import_context, None);
    assert_eq!(restored_session(&old), (None, None));
    // Nothing new is written for a thread without a session.
    let written = serde_json::to_string(&old).expect("serializes");
    assert!(!written.contains("session_id"));
    assert!(!written.contains("import_context"));

    let mut saved = old;
    saved.session_id = Some("sess-1".into());
    saved.session_provider = Some("codex".into());
    saved.import_context = Some("# Earlier".into());
    let reread: PersistedThread =
        serde_json::from_str(&serde_json::to_string(&saved).expect("serializes"))
            .expect("round trips");
    assert_eq!(reread.import_context.as_deref(), Some("# Earlier"));
    assert_eq!(
        restored_session(&reread),
        (Some("sess-1".into()), Some(AgentProvider::Codex))
    );

    // A session whose owner this build cannot read is not resumed.
    saved.session_provider = Some("gemini".into());
    assert_eq!(restored_session(&saved), (None, None));
    saved.session_provider = None;
    assert_eq!(restored_session(&saved), (None, None));
}

fn workspace_window(
    cx: &mut gpui::TestAppContext,
) -> (Entity<Workspace>, &mut gpui::VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("workspace entity");
    (workspace, cx)
}

#[gpui::test]
fn an_imported_session_becomes_a_resumable_project_thread(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    let imported = ImportedThread::new(
        candidate(Some("/tmp/bruhs-import-test/shop"), Some("sess-9")),
        vec![
            PortableMessage {
                role: Role::User,
                parts: vec![PortablePart::Text("fix paging".into())],
                started_at_ms: Some(1_000),
                model: None,
            },
            PortableMessage {
                role: Role::Assistant,
                parts: assistant_parts(),
                started_at_ms: Some(2_000),
                model: Some("claude-opus-4-6".into()),
            },
        ],
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_top_layer(TopLayer::ImportThread);
        workspace.open_imported_thread(imported, window, cx);
        assert!(workspace.top_layer.is_none(), "the dialog closes");
        assert_ne!(
            workspace.current_model, "claude-opus-4-6",
            "an import does not change the model new threads start on"
        );
        let thread = workspace.active_thread().expect("the import is open");
        assert_eq!(thread.project_name, "shop");
        assert_eq!(thread.project_path, "/tmp/bruhs-import-test/shop");
        assert_eq!(workspace.sidebar_tab, DesktopSidebarTab::Code);
        let chat = thread.chat.read(cx);
        assert_eq!(chat.thread_title, "Paging fix");
        assert_eq!(chat.model_id, "claude-opus-4-6");
        assert_eq!(chat.session_id.as_deref(), Some("sess-9"));
        assert_eq!(chat.session_provider, Some(AgentProvider::Claude));
        assert_eq!(chat.import_context, None);
        assert!(chat.run_locked);
        assert_eq!(chat.messages.len(), 2);
        assert_eq!(chat.messages[1].content, "done");
        assert_eq!(chat.messages[1].thinking, "plan");

        let saved = thread.snapshot(cx);
        assert_eq!(saved.session_id.as_deref(), Some("sess-9"));
        assert_eq!(saved.session_provider.as_deref(), Some("claude"));

        // The same session is not offered again.
        workspace.import_picker.found = vec![
            candidate(None, Some("sess-9")),
            candidate(None, Some("other")),
        ];
        let offered = workspace.visible_import_candidates(cx);
        assert_eq!(offered.len(), 1);
        assert_eq!(offered[0].session_id.as_deref(), Some("other"));

        workspace.copy_resume_command(cx);
    });
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(
        copied.as_deref(),
        Some("cd /tmp/bruhs-import-test/shop && claude --resume sess-9")
    );

    workspace.update(cx, |workspace, cx| workspace.copy_thread_markdown(cx));
    let markdown = cx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .expect("markdown copied");
    assert!(markdown.starts_with("# Paging fix\n\n_Conversation from bruhs._"));
    assert!(markdown.contains("## User\n\nfix paging"));
    assert!(markdown.contains("- tool `Bash`: ls"));
}

#[gpui::test]
fn a_transcript_import_is_a_home_chat_carrying_its_context(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    let mut cursor = candidate(None, None);
    cursor.source = ImportSource::Cursor;
    cursor.model = None;
    let imported = ImportedThread::new(
        cursor,
        vec![PortableMessage {
            role: Role::User,
            parts: vec![PortablePart::Text("from cursor".into())],
            started_at_ms: None,
            model: None,
        }],
    );
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_imported_thread(imported, window, cx);
        let thread = workspace.active_thread().expect("the import is open");
        assert!(thread.project_name.is_empty());
        assert_eq!(workspace.sidebar_tab, DesktopSidebarTab::Chat);
        let chat = thread.chat.read(cx);
        assert_eq!(chat.session_id, None);
        assert!(!chat.run_locked);
        assert!(chat
            .import_context
            .as_deref()
            .is_some_and(|context| context.contains("from cursor")));
        assert_eq!(
            thread.snapshot(cx).import_context,
            chat.import_context.clone()
        );

        workspace.copy_resume_command(cx);
    });
    // No session: nothing new reaches the clipboard.
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_ne!(
        copied.as_deref().map(|text| text.contains("--resume")),
        Some(true)
    );
}

/// Each thread keeps its model: picking one in another thread leaves the
/// import on its session's provider. Opening a thread does not change the
/// model new threads start on; Settings changes only that (and a thread with
/// no turn yet).
#[gpui::test]
fn each_thread_keeps_its_own_model(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_imported_thread(
            ImportedThread::new(
                candidate(Some("/tmp/bruhs-import-test/shop"), Some("sess-9")),
                vec![PortableMessage {
                    role: Role::User,
                    parts: vec![PortablePart::Text("fix paging".into())],
                    started_at_ms: None,
                    model: None,
                }],
            ),
            window,
            cx,
        );
        let imported = workspace.active_thread_id.clone().expect("import open");

        workspace.new_thread(&super::NewThread, window, cx);
        let fresh = workspace.active_thread_id.clone().expect("new thread open");
        assert_ne!(fresh, imported);
        workspace.set_current_model("gpt-5.5", cx);

        let model_of = |workspace: &Workspace, id: &str, cx: &gpui::App| {
            workspace
                .threads
                .iter()
                .find(|thread| thread.id == id)
                .map(|thread| thread.chat.read(cx).model_id.clone())
        };
        assert_eq!(model_of(workspace, &fresh, cx).as_deref(), Some("gpt-5.5"));
        assert_eq!(
            model_of(workspace, &imported, cx).as_deref(),
            Some("claude-opus-4-6")
        );

        workspace.select_thread_by_id(&imported, cx);
        assert_eq!(workspace.current_model, "gpt-5.5");

        // Settings with a thread open that has turns: only the default moves.
        workspace.set_default_model("claude-sonnet-4-6", cx);
        assert_eq!(workspace.current_model, "claude-sonnet-4-6");
        let imported_model = model_of(workspace, &imported, cx);
        assert_eq!(imported_model.as_deref(), Some("claude-opus-4-6"));
    });
}
