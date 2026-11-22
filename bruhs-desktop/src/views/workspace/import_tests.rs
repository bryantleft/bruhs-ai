//! Import thread and the export actions: converting messages, where an
//! import lands, the saved session, the Export file tab, and the palette
//! actions end to end.

use super::import_ui::export_tab::{export_lines, export_rows, ExportCalls};
use super::import_ui::{
    import_place, import_row_detail, imported_message, imported_messages, portable_messages,
    t3_failure, tilde_path, visible_candidates, ImportPlace, ImportTab,
};
use super::persistence::restored_session;
use super::{
    DesktopState, EpochMs, Message, MessageRole, PermissionMode, PersistedThread, TopLayer,
    TurnState, Workspace,
};
use crate::theme;
use bruhs_core::import::ImportError;
use bruhs_models::agent::{AgentProvider, Role};
use bruhs_models::imports::{
    batch_lines, export_row_detail, import_key, imported_line, reading_line, BatchCounts,
    ImportBatch, ImportJob, ImportJobState, ALREADY_HERE, CHOOSE_ANOTHER, CHOOSE_EXPORT,
    EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK, GROK_BEST_EFFORT, IMPORT_ALL,
    NOT_AN_EXPORT, T3_EXPORT_FAILED, T3_NOT_INSTALLED, T3_RUNTIME_UNSUPPORTED,
};
use bruhs_models::logic::portable::{
    ImportCandidate, ImportSource, ImportedThread, PortableMessage, PortablePart, PortableToolCall,
};
use gpui::{AppContext as _, Entity, FocusHandle, Focusable as _, Task, VisualTestContext};
use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

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
        "~/shop, 3h, resumes"
    );
    assert_eq!(
        import_row_detail(&candidate(None, None), Some("/Users/me"), now),
        "no folder, 3h, transcript"
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
    let visible = visible_candidates(&found, &known, &HashSet::new());
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
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
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

const BATCH: &str = "0123456789abcdef0123456789abcdef";

fn export_row(source: ImportSource, id: &str, title: &str, at: i64) -> ImportCandidate {
    ImportCandidate {
        source,
        id: id.into(),
        title: title.into(),
        cwd: None,
        updated_at: EpochMs::saturating_from_i64(at),
        provider: None,
        session_id: None,
        model: None,
    }
}

/// A read with ChatGPT and Grok chats, so the batch is best effort.
fn export_batch() -> ImportBatch {
    ImportBatch {
        id: BATCH.into(),
        candidates: vec![
            export_row(ImportSource::ChatGpt, "a1", "Trip to Lisbon", 1_600_000_000_000),
            export_row(ImportSource::Grok, "g1", "Rocket notes", 1_600_000_100_000),
            export_row(ImportSource::ChatGpt, "a2", "lisbon restaurants", 1_600_000_200_000),
        ],
        counts: BatchCounts {
            conversations: 3,
            skipped_empty: 1,
            ..BatchCounts::default()
        },
        best_effort: true,
        ..ImportBatch::default()
    }
}

fn ready(batch: ImportBatch) -> ImportJob {
    ImportJob {
        id: batch.id.clone(),
        state: ImportJobState::Ready {
            batch: Box::new(batch),
        },
    }
}

fn exported(row: &ImportCandidate) -> ImportedThread {
    ImportedThread::new(
        row.clone(),
        vec![PortableMessage {
            role: Role::User,
            parts: vec![PortablePart::Text(format!("about {}", row.title))],
            started_at_ms: None,
            model: None,
        }],
    )
}

/// The server, as the Export file tab's client seam sees it: answers in
/// order, and every call it was asked for.
#[derive(Default)]
struct FakeImports {
    reads: RefCell<VecDeque<Result<ImportJob, String>>>,
    jobs: RefCell<VecDeque<Result<ImportJob, String>>>,
    threads: RefCell<Vec<ImportCandidate>>,
    failing_thread: RefCell<Option<String>>,
    calls: RefCell<Vec<String>>,
}

impl FakeImports {
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

fn fake_calls(fake: &Rc<FakeImports>) -> ExportCalls {
    let read = Rc::clone(fake);
    let job = Rc::clone(fake);
    let thread = Rc::clone(fake);
    let discard = Rc::clone(fake);
    ExportCalls {
        read: Rc::new(move |paths: Vec<String>, _cx: &gpui::App| {
            read.calls.borrow_mut().push(format!("read {}", paths.join(",")));
            let answer = read.reads.borrow_mut().pop_front();
            Task::ready(answer.unwrap_or_else(|| Err(NOT_AN_EXPORT.into())))
        }),
        job: Rc::new(move |id: String, _cx: &gpui::App| {
            job.calls.borrow_mut().push(format!("job {id}"));
            let answer = job.jobs.borrow_mut().pop_front();
            Task::ready(answer.unwrap_or_else(|| Err(NOT_AN_EXPORT.into())))
        }),
        thread: Rc::new(
            move |(source, upload, id): (ImportSource, String, String), _cx: &gpui::App| {
                thread
                    .calls
                    .borrow_mut()
                    .push(format!("thread {}:{id} {upload}", source.as_str()));
                if thread.failing_thread.borrow().as_deref() == Some(id.as_str()) {
                    return Task::ready(Err(T3_EXPORT_FAILED.to_string()));
                }
                let row = thread
                    .threads
                    .borrow()
                    .iter()
                    .find(|row| row.source == source && row.id == id)
                    .cloned();
                Task::ready(row.map(|row| exported(&row)).ok_or_else(|| "gone".to_string()))
            },
        ),
        discard: Rc::new(move |id: String| {
            discard.calls.borrow_mut().push(format!("discard {id}"));
        }),
    }
}

fn leak(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn drawn(cx: &mut VisualTestContext, selector: String) -> bool {
    cx.debug_bounds(leak(selector)).is_some()
}

fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
    cx.update(|window, cx| window.focused(cx))
}

/// A key press as the app gets it: gpui clicks a focused button on key up.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

/// The Import thread dialog on its Export file tab, its calls answered by
/// `fake`.
fn export_tab<'a>(
    cx: &'a mut gpui::TestAppContext,
    fake: &Rc<FakeImports>,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (workspace, cx) = workspace_window(cx);
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    let calls = fake_calls(fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.import_picker.export.calls = Some(calls);
        workspace.open_import_thread(&super::ImportThread, window, cx);
        workspace.set_import_tab(ImportTab::Export, window, cx);
    });
    cx.run_until_parked();
    (workspace, cx)
}

/// Choose export file, through the native picker, with `fake` answering
/// the read with `answer`.
fn read_export(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    fake: &Rc<FakeImports>,
    answer: ImportJob,
) {
    fake.reads.borrow_mut().push_back(Ok(answer));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.choose_export_files(window, cx);
    });
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(options.files && options.directories && options.multiple);
        assert_eq!(options.prompt.as_deref(), Some(CHOOSE_EXPORT));
        Some(vec![PathBuf::from("/tmp/bruhs-import-test/export.zip")])
    });
    cx.run_until_parked();
}

fn export_handles(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> [FocusHandle; 5] {
    workspace.read_with(cx, |workspace, cx| {
        let ui = workspace.import_picker.export.ui.as_ref().expect("export tab");
        [
            ui.search.focus_handle(cx),
            ui.list.clone(),
            ui.all.clone(),
            ui.another.clone(),
            ui.choose.clone(),
        ]
    })
}

#[test]
fn export_rows_hide_imported_chats() {
    let batch = export_batch();
    let known: HashSet<String> = [import_key(ImportSource::Grok, "g1")].into();
    let ids = |rows: Vec<ImportCandidate>| -> Vec<String> {
        rows.into_iter().map(|row| row.id).collect()
    };
    let visible = visible_candidates(&batch.candidates, &HashSet::new(), &known);
    assert_eq!(
        visible.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        ["a1", "a2"]
    );
    assert_eq!(ids(export_rows(&batch, &known, "")), ["a1", "a2"]);
    assert_eq!(ids(export_rows(&batch, &known, "  LISBON ")), ["a1", "a2"]);
    assert_eq!(ids(export_rows(&batch, &HashSet::new(), "rocket")), ["g1"]);
    // The same id from another source is another chat.
    let chatgpt_g1: HashSet<String> = [import_key(ImportSource::ChatGpt, "g1")].into();
    assert_eq!(ids(export_rows(&batch, &chatgpt_g1, "rocket")), ["g1"]);
}

#[test]
fn the_grok_note_shows_once_among_the_lines() {
    let batch = export_batch();
    let lines = export_lines(&batch);
    let mut expected: Vec<(String, bool)> = batch_lines(&batch)
        .into_iter()
        .map(|line| (line, false))
        .collect();
    expected.push((GROK_BEST_EFFORT.to_string(), true));
    assert_eq!(lines, expected);
    // The read's own warning carries the note: it shows once, in durian.
    let mut warned = batch;
    warned.warnings = vec![GROK_BEST_EFFORT.to_string()];
    let lines = export_lines(&warned);
    assert_eq!(
        lines.iter().filter(|(line, _)| line == GROK_BEST_EFFORT).count(),
        1
    );
    assert_eq!(lines.last(), Some(&(GROK_BEST_EFFORT.to_string(), true)));
}

#[test]
fn t3_failures_name_t3_once() {
    for copy in [T3_NOT_INSTALLED, T3_RUNTIME_UNSUPPORTED, T3_EXPORT_FAILED] {
        assert_eq!(t3_failure(&ImportError::Refused(copy)), copy);
    }
    assert_eq!(
        t3_failure(&ImportError::Refused(NOT_AN_EXPORT)),
        format!("T3: {NOT_AN_EXPORT}")
    );
}

#[test]
fn an_export_import_keeps_its_key_across_a_save() {
    let legacy = r##"{"id":"t1","title":"Old","project_name":"","project_path":null,"model_id":"claude-sonnet-4-6","permission_mode":"default","messages":[],"import_context":"# Old"}"##;
    let old: PersistedThread = serde_json::from_str(legacy).expect("legacy record loads");
    assert_eq!(old.import_key, None);
    // A thread without a key writes no key.
    let written = serde_json::to_string(&old).expect("serializes");
    assert!(!written.contains("import_key"), "{written}");

    let mut saved = old;
    saved.import_key = Some(import_key(ImportSource::Grok, "g1"));
    let reread: PersistedThread =
        serde_json::from_str(&serde_json::to_string(&saved).expect("serializes"))
            .expect("round trips");
    assert_eq!(reread.import_key.as_deref(), Some("grok:g1"));
    assert_eq!(reread.import_context.as_deref(), Some("# Old"));
}

#[gpui::test]
fn an_export_chat_keeps_its_key_and_time_and_the_dialog(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    let row = export_row(ImportSource::Grok, "g1", "Rocket notes", 1_600_000_100_000);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_top_layer(TopLayer::ImportThread);
        workspace.open_imported_thread(exported(&row), window, cx);
        assert_eq!(
            workspace.top_layer,
            Some(TopLayer::ImportThread),
            "an export import keeps the dialog open (F23)"
        );
        let thread = workspace.active_thread().expect("the import is open");
        assert_eq!(thread.updated_at_ms, row.updated_at, "sorts by when it happened");
        assert!(thread.project_name.is_empty());
        let chat = thread.chat.read(cx);
        assert_eq!(chat.import_key.as_deref(), Some("grok:g1"));
        assert_eq!(chat.session_id, None);
        assert!(!chat.run_locked);
        assert!(chat.import_context.is_some());
        assert!(workspace.known_import_keys(cx).contains("grok:g1"));

        // The key survives a save and a restart.
        let saved = thread.snapshot(cx);
        assert_eq!(saved.import_key.as_deref(), Some("grok:g1"));
        let restored = Workspace::create_thread_from_persisted(
            saved,
            "claude-sonnet-4-6",
            PermissionMode::Default,
            window,
            cx,
        );
        assert_eq!(
            restored.chat.read(cx).import_key.as_deref(),
            Some("grok:g1")
        );
        assert_eq!(restored.updated_at_ms, row.updated_at);

        // A local tool's import still closes the dialog and keeps no key.
        workspace.open_imported_thread(
            ImportedThread::new(candidate(None, None), Vec::new()),
            window,
            cx,
        );
        assert!(workspace.top_layer.is_none());
        let local = workspace.active_thread().expect("local import open");
        assert_eq!(local.chat.read(cx).import_key, None);
    });
}

#[gpui::test]
fn the_export_tab_uses_the_shared_lines(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let (workspace, cx) = export_tab(cx, &fake);
    for line in [EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK] {
        assert!(drawn(cx, format!("desktop-import-export-help:{line}")), "{line}");
    }
    assert!(drawn(cx, format!("desktop-import-export-warn:{GROK_BEST_EFFORT}")));
    assert!(drawn(cx, format!("desktop-import-export-button:{CHOOSE_EXPORT}")));
    assert!(!drawn(cx, format!("desktop-import-export-button:{IMPORT_ALL}")));
    // The local search belongs to the local tabs only (F24).
    assert!(!drawn(cx, "desktop-import-search".into()));

    let batch = export_batch();
    read_export(&workspace, cx, &fake, ready(batch.clone()));
    let lines = batch_lines(&batch);
    assert!(!lines.is_empty());
    for line in &lines {
        assert!(drawn(cx, format!("desktop-import-export-line:{line}")), "{line}");
    }
    // Grok's note: once, in durian.
    assert!(drawn(cx, format!("desktop-import-export-warn:{GROK_BEST_EFFORT}")));
    assert!(!drawn(cx, format!("desktop-import-export-line:{GROK_BEST_EFFORT}")));
    let now = EpochMs::now();
    for row in &batch.candidates {
        let detail = export_row_detail(row, now);
        assert!(drawn(cx, format!("desktop-import-export-detail:{detail}")), "{detail}");
        assert!(drawn(cx, format!("desktop-import-export-row-{}", import_key(row.source, &row.id))));
    }
    assert!(export_row_detail(&batch.candidates[1], now).ends_with(", best effort"));
    assert!(drawn(cx, format!("desktop-import-export-button:{IMPORT_ALL}")));
    assert!(drawn(cx, format!("desktop-import-export-button:{CHOOSE_ANOTHER}")));
    assert!(!drawn(cx, format!("desktop-import-export-button:{CHOOSE_EXPORT}")));
    for line in [EXPORT_HELP_CHATGPT, EXPORT_HELP_CLAUDE, EXPORT_HELP_GROK] {
        assert!(!drawn(cx, format!("desktop-import-export-help:{line}")), "{line}");
    }
    assert!(!drawn(cx, format!("desktop-import-export-line:{ALREADY_HERE}")));
}

#[gpui::test]
fn tab_reaches_choose_export_file(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let (workspace, cx) = export_tab(cx, &fake);
    let [.., choose] = export_handles(&workspace, cx);
    let mut reached = false;
    for _ in 0..12 {
        cx.simulate_keystrokes("tab");
        if focused(cx).as_ref() == Some(&choose) {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab reaches Choose export file");
    // Enter on it opens the picker.
    press(cx, "enter");
    assert!(cx.did_prompt_for_paths());
}

#[gpui::test]
fn a_ready_batch_tabs_from_the_search_to_the_rows_then_the_buttons(
    cx: &mut gpui::TestAppContext,
) {
    let fake = Rc::new(FakeImports::default());
    let (workspace, cx) = export_tab(cx, &fake);
    read_export(&workspace, cx, &fake, ready(export_batch()));
    let [search, list, all, another, _] = export_handles(&workspace, cx);
    assert!(focused(cx).as_ref() == Some(&search), "a ready batch focuses the search");
    cx.simulate_keystrokes("tab");
    assert!(focused(cx).as_ref() == Some(&list), "then the rows");
    cx.simulate_keystrokes("tab");
    assert!(focused(cx).as_ref() == Some(&all), "then Import all");
    cx.simulate_keystrokes("tab");
    assert!(focused(cx).as_ref() == Some(&another), "then Choose another file");
}

#[gpui::test]
fn down_then_enter_imports_the_second_row(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let batch = export_batch();
    *fake.threads.borrow_mut() = batch.candidates.clone();
    let (workspace, cx) = export_tab(cx, &fake);
    read_export(&workspace, cx, &fake, ready(batch.clone()));
    let [search, ..] = export_handles(&workspace, cx);
    press(cx, "down");
    press(cx, "enter");
    assert_eq!(
        fake.calls(),
        [
            "read /tmp/bruhs-import-test/export.zip".to_string(),
            format!("thread grok:g1 {BATCH}"),
        ]
    );
    let status = imported_line(1, ImportSource::Grok);
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.top_layer, Some(TopLayer::ImportThread));
        let thread = workspace.active_thread().expect("the chat came in");
        assert_eq!(thread.chat.read(cx).import_key.as_deref(), Some("grok:g1"));
        assert_eq!(thread.updated_at_ms, batch.candidates[1].updated_at);
        let left: Vec<String> = workspace
            .visible_export_rows(cx)
            .into_iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(left, ["a1", "a2"], "the row leaves the list");
        assert_eq!(workspace.import_picker.export.status.as_deref(), Some(status.as_str()));
    });
    assert!(drawn(cx, format!("desktop-import-export-status:{status}")));
    assert!(focused(cx).as_ref() == Some(&search), "focus stays on the search");
}

#[gpui::test]
fn import_all_brings_in_the_rows_in_order(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let batch = export_batch();
    *fake.threads.borrow_mut() = batch.candidates.clone();
    let (workspace, cx) = export_tab(cx, &fake);
    read_export(&workspace, cx, &fake, ready(batch.clone()));
    let [_, _, all, another, _] = export_handles(&workspace, cx);
    cx.update(|window, cx| window.focus(&all, cx));
    press(cx, "enter");
    assert_eq!(
        fake.calls()[1..],
        [
            format!("thread chatgpt:a1 {BATCH}"),
            format!("thread grok:g1 {BATCH}"),
            format!("thread chatgpt:a2 {BATCH}"),
        ]
    );
    let done = format!(
        "{} {}",
        imported_line(2, ImportSource::ChatGpt),
        imported_line(1, ImportSource::Grok)
    );
    workspace.read_with(cx, |workspace, cx| {
        let keys = workspace.known_import_keys(cx);
        for row in &batch.candidates {
            assert!(keys.contains(&import_key(row.source, &row.id)));
        }
        assert_eq!(workspace.top_layer, Some(TopLayer::ImportThread));
        assert!(!workspace.import_picker.export.importing);
        assert_eq!(workspace.import_picker.export.status.as_deref(), Some(done.as_str()));
    });
    assert!(drawn(cx, format!("desktop-import-export-line:{ALREADY_HERE}")));
    assert!(!drawn(cx, format!("desktop-import-export-button:{IMPORT_ALL}")));
    assert!(focused(cx).as_ref() == Some(&another), "nothing left: Choose another file");
}

#[gpui::test]
fn import_all_stops_at_a_failure_and_says_so(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let batch = export_batch();
    *fake.threads.borrow_mut() = batch.candidates.clone();
    *fake.failing_thread.borrow_mut() = Some("g1".into());
    let (workspace, cx) = export_tab(cx, &fake);
    read_export(&workspace, cx, &fake, ready(batch));
    workspace.update_in(cx, |workspace, window, cx| workspace.import_all_exports(window, cx));
    cx.run_until_parked();
    assert_eq!(fake.calls().len(), 3, "a1, then g1 fails, and a2 waits");
    let done = imported_line(1, ImportSource::ChatGpt);
    workspace.read_with(cx, |workspace, _| {
        let export = &workspace.import_picker.export;
        assert_eq!(export.error.as_deref(), Some(T3_EXPORT_FAILED));
        assert_eq!(export.status.as_deref(), Some(done.as_str()));
        assert!(!export.importing);
    });
    assert!(drawn(cx, format!("desktop-import-export-error:{T3_EXPORT_FAILED}")));
}

#[gpui::test]
fn a_read_is_polled_until_its_batch_is_ready(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let (workspace, cx) = export_tab(cx, &fake);
    let reading = |files_done, conversations| ImportJob {
        id: BATCH.into(),
        state: ImportJobState::Reading {
            files_done,
            files: 1,
            conversations,
        },
    };
    fake.jobs.borrow_mut().push_back(Ok(reading(0, 40)));
    fake.jobs.borrow_mut().push_back(Ok(ready(export_batch())));
    read_export(&workspace, cx, &fake, reading(0, 0));
    let line = reading_line(0, 1, 0);
    assert!(drawn(cx, format!("desktop-import-export-status:{line}")));
    // Still reading the same file: the line stays as it was.
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert!(drawn(cx, format!("desktop-import-export-status:{line}")));
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_eq!(
        fake.calls()[1..],
        [format!("job {BATCH}"), format!("job {BATCH}")]
    );
    assert!(drawn(cx, format!("desktop-import-export-button:{IMPORT_ALL}")));
    // Ready ends the polling.
    cx.executor().advance_clock(Duration::from_millis(2_000));
    cx.run_until_parked();
    assert_eq!(fake.calls().len(), 3);
}

#[gpui::test]
fn a_failed_read_shows_its_copy_and_focuses_choose(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let (workspace, cx) = export_tab(cx, &fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.read_export_paths(vec![PathBuf::from("/tmp/notes.txt")], window, cx);
    });
    cx.run_until_parked();
    let [.., choose] = export_handles(&workspace, cx);
    assert!(drawn(cx, format!("desktop-import-export-error:{NOT_AN_EXPORT}")));
    assert!(focused(cx).as_ref() == Some(&choose));
}

#[gpui::test]
fn escape_closes_the_dialog_and_discards_the_batch(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeImports::default());
    let (workspace, cx) = export_tab(cx, &fake);
    read_export(&workspace, cx, &fake, ready(export_batch()));
    press(cx, "escape");
    workspace.read_with(cx, |workspace, _| {
        assert!(workspace.top_layer.is_none(), "Escape closes the dialog");
        assert!(workspace.import_picker.export.job.is_none());
    });
    assert_eq!(fake.calls().last(), Some(&format!("discard {BATCH}")));
    // Closed: nothing is discarded twice.
    workspace.update(cx, |workspace, _| workspace.close_transient_surfaces());
    assert_eq!(
        fake.calls()
            .iter()
            .filter(|call| call.starts_with("discard"))
            .count(),
        1
    );
}
