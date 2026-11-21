//! Complete saved-thread loading, including tools, files, sessions and held queues.
use super::super::super::{current_dir_string, DesktopThread, ThreadRestorer};
use super::*;
use gpui::{Context, Window};

pub(super) fn current_threads(
    state: Option<&mut PersistedWorkspaceState>,
    model: &str,
    permission: PermissionMode,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) -> (Vec<DesktopThread>, Option<String>) {
    Workspace::restore_threads(state, model, permission, window, cx)
}

fn original_threads(
    state: Option<&mut PersistedWorkspaceState>,
    model: &str,
    permission: PermissionMode,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) -> (Vec<DesktopThread>, Option<String>) {
    Workspace::restore_original_threads(state.as_deref(), model, permission, window, cx)
}

fn rich_thread(index: usize, messages: usize) -> PersistedThread {
    let mut thread = saved_thread(index, messages);
    thread.permission_mode = PermissionMode::AcceptEdits;
    thread.session_id = Some(format!("saved-session-{index}"));
    thread.session_provider = Some(
        if index.is_multiple_of(2) {
            "claude"
        } else {
            "codex"
        }
        .into(),
    );
    thread.fresh_session = index.is_multiple_of(2);
    thread.import_context = Some("Imported context, café, 👩🏽‍💻 and \"quotes\".\n".repeat(48));
    thread.goal = Some(bruhs_models::logic::commands::ThreadGoal {
        text: format!("Saved paused goal {index}"),
        paused: true,
    });
    thread.worktree_path = Some(format!("/synthetic/worktree-{index}"));
    thread.effort = Some("high".into());
    thread.run_locked = true;
    thread.role = Some("saved-role".into());
    thread.instructions_stamp = Some("saved-instructions-stamp".into());
    thread.pinned = index.is_multiple_of(2);
    thread.settled = index.is_multiple_of(3);
    thread.pin_rank = Some(i64::try_from(index).unwrap());
    thread.active_rank = Some(i64::try_from(index).unwrap());
    thread.snoozed_until_ms = Some(1_600_000_000_001);
    for (message_index, message) in thread.messages.iter_mut().enumerate() {
        message.thinking = "Saved thinking, café 👩🏽‍💻\n".repeat(32);
        message.model = Some(
            if message_index.is_multiple_of(2) {
                "claude-sonnet-4-6"
            } else {
                "gpt-5.4"
            }
            .into(),
        );
        message.usage = Some(serde_json::from_value(json!({
            "input_tokens":12,"output_tokens":34,"cache_read_input_tokens":5,"cache_creation_input_tokens":6,
        })).unwrap());
        message.duration_ms = Some(500);
        message.started_at_ms = Some(1_600_000_000_000);
        message.completed_at_ms = Some(1_600_000_000_500);
        message.stopped = message_index.is_multiple_of(3);
        message.error =
            (message_index.is_multiple_of(5)).then(|| "Saved provider error, unchanged.".into());
        message.checkpoint = Some(if message_index.is_multiple_of(2) {
            "".into()
        } else {
            "a".repeat(40)
        });
        message.tool_calls = (0..2).map(|tool| super::super::super::DesktopToolCall {
            text_offset: None,
            id:format!("tool-{index}-{message_index}-{tool}"), tool:"Read".into(),
            input:json!({"path":format!("/synthetic/{index}/{message_index}/{tool}"),"nested":[{"text":"Saved tool input. ".repeat(8)}]}),
            output:Some("Complete tool output, café 👩🏽‍💻 and \"quotes\".\n".repeat(32)),
            is_error:tool==1,
        }).collect();
        message.files = (0..4)
            .map(|file| bruhs_models::logic::attachments::UploadedFile {
                name: format!("saved-{index}-{message_index}-{file}.txt"),
                path: format!("/synthetic/uploads/{index}/{message_index}/{file}.txt"),
            })
            .collect();
    }
    thread.prompt_queue = (0..3)
        .map(|prompt| {
            let thread_id = if prompt == 1 {
                "foreign-thread".into()
            } else {
                thread.id.clone()
            };
            let (text, _, chips) = bruhs_models::logic::context_chip::insert_chip(
                &format!("Saved queued prompt {index}/{prompt}: "),
                &[],
                format!("Saved queued prompt {index}/{prompt}: ").len(),
                format!("chip-{index}-{prompt}"),
                bruhs_models::logic::context_chip::ChipKind::Citation {
                    message_id: format!("saved-message-{index}"),
                    quote: "Complete quoted context. café 👩🏽‍💻\n".repeat(16),
                    comment: "A saved citation".into(),
                },
            );
            bruhs_models::logic::queue::QueuedPrompt {
                id: format!("queue-{index}-{prompt}"),
                thread_id,
                text,
                at_ms: 1.0,
                chips,
            }
        })
        .collect();
    thread
}

pub(super) fn thread_state(label: &str) -> PersistedWorkspaceState {
    let mut state = saved_state("thread-extra", 0, 0, 0);
    state.threads = match label {
        "thread-rich-small" => vec![rich_thread(0, 32)],
        "thread-rich-history" => (0..32).map(|index| rich_thread(index, 128)).collect(),
        "thread-assistant-history" => (0..32)
            .map(|index| {
                let mut thread = rich_thread(index, 128);
                for (message_index, message) in thread.messages.iter_mut().enumerate() {
                    if message_index.is_multiple_of(2) {
                        message.role = super::super::super::MessageRole::User;
                        message.thinking.clear();
                        message.model = None;
                        message.usage = None;
                        message.duration_ms = None;
                        message.started_at_ms = None;
                        message.completed_at_ms = None;
                        message.stopped = false;
                        message.error = None;
                        message.tool_calls.clear();
                        message.checkpoint = None;
                    } else {
                        message.role = super::super::super::MessageRole::Assistant;
                        message.content =
                            format!("### Saved result {message_index}\n\n{}", message.content);
                        message.files.clear();
                    }
                }
                thread
            })
            .collect(),
        "thread-single-history" => vec![saved_thread(0, 8192)],
        "thread-many-small" => (0..512).map(|index| saved_thread(index, 2)).collect(),
        _ => panic!("unknown saved-thread fixture {label}"),
    };
    assert!(state.automation_state.jobs.is_empty());
    assert!(state.threads.iter().all(|thread| thread.branch.is_none()));
    state
}

#[test]
fn thread_load_inputs_with_saved_tools_files_and_queues_are_reproducible() {
    let first = serde_json::to_vec(&thread_state("thread-rich-small")).unwrap();
    let second = serde_json::to_vec(&thread_state("thread-rich-small")).unwrap();
    assert_eq!(Sha256::digest(&first), Sha256::digest(&second));
}

#[gpui::test]
fn assistant_thread_load_preserves_conversation_roles_tools_and_held_queues(
    cx: &mut TestAppContext,
) {
    initialize(cx);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("assistant-threads.json");
    let state = thread_state("thread-assistant-history");
    let second = thread_state("thread-assistant-history");
    assert_eq!(
        Sha256::digest(serde_json::to_vec(&state).unwrap()),
        Sha256::digest(serde_json::to_vec(&second).unwrap())
    );
    for recovery in [false, true] {
        write_fixture(&path, &state, recovery);
        let expected = observe_with_threads(cx, &path, current, original_threads);
        let actual = observe_with_threads(cx, &path, current, current_threads);
        assert_eq!(actual.snapshot, expected.snapshot);
        assert_eq!(actual.recovered, recovery);
        let messages = actual.snapshot["threads"][0]["messages"]
            .as_array()
            .unwrap();
        assert_eq!(messages[0]["role"], json!("user"));
        assert_eq!(messages[1]["role"], json!("assistant"));
        assert_eq!(messages[0]["tool_calls"], json!([]));
        assert_eq!(messages[1]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(messages[0]["files"].as_array().unwrap().len(), 4);
        assert!(messages[1].get("files").is_none());
    }
}

#[gpui::test]
fn saved_thread_load_preserves_metadata_queues_permissions_and_provider_policy(
    cx: &mut TestAppContext,
) {
    initialize(cx);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("threads.json");
    for (provider, session) in [
        (Some("claude"), Some("claude-session")),
        (Some("codex"), Some("codex-session")),
        (Some("unknown"), Some("unsupported-session")),
        (Some("codex"), Some("")),
        (None, Some("unowned-session")),
        (Some("claude"), None),
    ] {
        let mut state = thread_state("thread-rich-small");
        let mut second = rich_thread(1, 3);
        second.permission_mode = PermissionMode::Plan;
        state.active_thread_id = Some(second.id.clone());
        state.threads.push(second);
        state.permission_mode = None;
        state.threads[0].session_provider = provider.map(str::to_string);
        state.threads[0].session_id = session.map(str::to_string);
        state.threads[0].model_id = " ".into();
        state.threads[0].project_path = Some(String::new());
        for recovery in [false, true] {
            write_fixture(&path, &state, recovery);
            let expected = observe_with_threads(cx, &path, current, original_threads);
            let actual = observe_with_threads(cx, &path, current, current_threads);
            assert_eq!(actual.snapshot, expected.snapshot);
            assert_eq!(actual.recovered, recovery);
            assert_eq!(actual.snapshot["permission_mode"], json!("plan"));
            assert_eq!(actual.snapshot["active_thread_id"], json!("thread-0"));
            assert_eq!(
                actual.snapshot["threads"][0]["prompt_queue"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(
                actual.snapshot["threads"][0]["model_id"],
                json!("claude-sonnet-4-6")
            );
        }
    }
}

#[gpui::test]
fn saved_thread_project_paths_match_original_construction(cx: &mut TestAppContext) {
    initialize(cx);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("project-context.json");
    for saved_path in [
        Some("/synthetic/saved café 日本語"),
        Some("relative/project"),
        Some(" "),
        Some(""),
        None,
    ] {
        let mut state = thread_state("thread-rich-small");
        state.threads[0].project_name = "Saved project".into();
        state.threads[0].project_path = saved_path.map(str::to_string);
        for recovery in [false, true] {
            write_fixture(&path, &state, recovery);
            let expected = observe_with_threads(
                cx,
                &path,
                current,
                super::project_path_reference::default_threads,
            );
            let actual = observe_with_threads(cx, &path, current, current_threads);
            assert_eq!(
                actual.snapshot, expected.snapshot,
                "{saved_path:?}/{recovery}"
            );
            assert_eq!(actual.recovered, recovery);
            assert_eq!(
                actual.snapshot["threads"][0]["project_path"],
                json!(saved_path
                    .filter(|path| !path.is_empty())
                    .map_or_else(current_dir_string, str::to_string))
            );
            assert_eq!(
                actual.snapshot["threads"][0]["project_name"],
                json!("Saved project")
            );
        }
    }
}

#[gpui::test]
fn saved_thread_transfer_reuses_message_and_queue_storage(cx: &mut TestAppContext) {
    let (workspace, cx) = super::super::fixture(cx);
    let mut state = thread_state("thread-rich-small");
    let input = serde_json::to_value(&state).unwrap();
    let message = &state.threads[0].messages[0];
    let content = message.content.as_ptr();
    let thinking = message.thinking.as_ptr();
    let tools = message.tool_calls.as_ptr();
    let output = message.tool_calls[0].output.as_ref().unwrap().as_ptr();
    let nested_input = message.tool_calls[0].input["nested"][0]["text"]
        .as_str()
        .unwrap()
        .as_ptr();
    let files = message.files.as_ptr();
    let file_path = message.files[0].path.as_ptr();
    let queue_text = state.threads[0].prompt_queue[0].text.as_ptr();
    let session = state.threads[0].session_id.as_ref().unwrap().as_ptr();
    let imported = state.threads[0].import_context.as_ref().unwrap().as_ptr();
    workspace.update_in(cx, |_, window, cx| {
        let (threads, active) = Workspace::restore_threads(
            Some(&mut state),
            "claude-sonnet-4-6",
            PermissionMode::Plan,
            window,
            cx,
        );
        assert_eq!(active.as_deref(), Some("thread-0"));
        assert!(state.threads.is_empty());
        let remaining = serde_json::to_value(&state).unwrap();
        for (field, expected) in input.as_object().unwrap() {
            if field != "threads" {
                assert_eq!(&remaining[field], expected, "unrelated saved field {field}");
            }
        }
        let chat = threads[0].chat.read(cx);
        let message = &chat.messages[0];
        assert_eq!(message.content.as_ptr(), content);
        assert_eq!(message.thinking.as_ptr(), thinking);
        assert_eq!(message.tool_calls.as_ptr(), tools);
        assert_eq!(
            message.tool_calls[0].output.as_ref().unwrap().as_ptr(),
            output
        );
        assert_eq!(
            message.tool_calls[0].input["nested"][0]["text"]
                .as_str()
                .unwrap()
                .as_ptr(),
            nested_input
        );
        assert_eq!(message.files.as_ptr(), files);
        assert_eq!(message.files[0].path.as_ptr(), file_path);
        assert_eq!(chat.session_id.as_ref().unwrap().as_ptr(), session);
        assert_eq!(chat.import_context.as_ref().unwrap().as_ptr(), imported);
        assert_eq!(chat.prompt_queue.len(), 2);
        assert_eq!(
            chat.prompt_queue.text_of("queue-0-0").unwrap().as_ptr(),
            queue_text
        );
        assert!(chat.queue_held && !chat.is_streaming);
        assert_eq!(chat.permission_mode, PermissionMode::Plan);
    });
}

#[gpui::test]
fn saved_thread_fresh_and_legacy_timestamp_fallbacks_match_original(cx: &mut TestAppContext) {
    let (workspace, cx) = super::super::fixture(cx);
    for mut state in [
        None,
        Some(saved_state("empty", 0, 0, 0)),
        Some(thread_state("thread-rich-small")),
    ] {
        if let Some(state) = state.as_mut() {
            if state.threads[0].messages.is_empty() {
                state.threads.clear();
            } else {
                state.threads[0].updated_at_ms = bruhs_models::EpochMs::UNIX_EPOCH;
                state.threads[0].project_path = None;
            }
        }
        workspace.update_in(cx, |_, window, cx| {
            let earliest = bruhs_models::EpochMs::now();
            let (original, _) = Workspace::restore_original_threads(
                state.as_ref(),
                "claude-sonnet-4-6",
                PermissionMode::Plan,
                window,
                cx,
            );
            let (current, _) = Workspace::restore_threads(
                state.as_mut(),
                "claude-sonnet-4-6",
                PermissionMode::Plan,
                window,
                cx,
            );
            let latest = bruhs_models::EpochMs::now();
            assert_eq!(current.len(), original.len());
            for (current, original) in current.iter().zip(&original) {
                let mut current = current.snapshot(cx);
                let mut original = original.snapshot(cx);
                assert!((earliest..=latest).contains(&current.updated_at_ms));
                assert!((earliest..=latest).contains(&original.updated_at_ms));
                current.updated_at_ms = bruhs_models::EpochMs::UNIX_EPOCH;
                original.updated_at_ms = bruhs_models::EpochMs::UNIX_EPOCH;
                if current.messages.is_empty() {
                    uuid::Uuid::parse_str(&current.id).unwrap();
                    uuid::Uuid::parse_str(&original.id).unwrap();
                    current.id = "fresh-thread".into();
                    original.id = "fresh-thread".into();
                }
                assert_eq!(
                    serde_json::to_value(current).unwrap(),
                    serde_json::to_value(original).unwrap()
                );
            }
        });
    }
}

fn measure(
    cx: &mut TestAppContext,
    path: &Path,
    restore: ThreadRestorer,
    label: &str,
    recovery: bool,
    state: PersistedWorkspaceState,
    variant: &str,
) {
    let bytes = write_fixture(path, &state, recovery);
    let expected = observe_with_threads(cx, path, current, original_threads);
    let mut total = Vec::new();
    let mut load = Vec::new();
    let mut construct = Vec::new();
    let mut settle = Vec::new();
    for iteration in 0..35 {
        let observation = observe_with_threads(cx, path, current, restore);
        assert_eq!(
            observation.snapshot, expected.snapshot,
            "{label}/{recovery}/{iteration}"
        );
        assert_eq!(observation.recovered, recovery);
        if iteration >= 5 {
            total.push(observation.total_ms);
            load.push(observation.read_decode_hash_ms);
            construct.push(observation.construct_ms);
            settle.push(observation.settle_ms);
        }
    }
    assert_eq!(
        std::fs::read(if recovery {
            super::super::super::desktop_state_backup_path(path)
        } else {
            path.to_owned()
        })
        .unwrap(),
        bytes
    );
    println!(
        "THREAD_LOAD_COST {}",
        json!({"label":label,"recovery":recovery,"variant":variant,"input_bytes":bytes.len(),"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"warmups":5,"total_samples_ms":total,"load_samples_ms":load,"construct_samples_ms":construct,"settle_samples_ms":settle})
    );
    std::fs::remove_file(path).unwrap();
    if recovery {
        std::fs::remove_file(super::super::super::desktop_state_backup_path(path)).unwrap();
    }
}

#[gpui::test]
#[ignore = "manual optimized complete-file saved-thread restoration comparison"]
fn complete_thread_restore_load_cost(cx: &mut TestAppContext) {
    initialize(cx);
    let (variant, restore): (&str, ThreadRestorer) =
        match std::env::var("BRUHS_PERF_THREAD_RESTORE").as_deref() {
            Ok("original") => ("original", original_threads),
            Ok("owned") | Err(_) => ("owned", current_threads),
            other => panic!("invalid thread restore selection {other:?}"),
        };
    let root = tempfile::tempdir().unwrap();
    let selected_case = std::env::var("BRUHS_PERF_THREAD_CASE").ok();
    let mut measured = 0;
    for (label, documents, records, messages) in [
        ("settings-control", 0, 0, 0),
        ("knowledge-small", 256, 0, 0),
        ("knowledge-large", 4096, 0, 0),
        ("records-large", 0, 4096, 0),
        ("history-control", 0, 0, 8192),
        ("meeting-control", 0, 0, 0),
        ("mixed", 1024, 1024, 2048),
    ] {
        if selected_case
            .as_deref()
            .is_some_and(|selected| selected != label)
        {
            continue;
        }
        for recovery in [false, true] {
            measure(
                cx,
                &root.path().join(format!("{label}-{recovery}.json")),
                restore,
                label,
                recovery,
                saved_state(label, documents, records, messages),
                variant,
            );
            measured += 1;
        }
    }
    for label in [
        "thread-rich-small",
        "thread-rich-history",
        "thread-assistant-history",
        "thread-single-history",
        "thread-many-small",
    ] {
        if selected_case
            .as_deref()
            .is_some_and(|selected| selected != label)
        {
            continue;
        }
        for recovery in [false, true] {
            measure(
                cx,
                &root.path().join(format!("{label}-{recovery}.json")),
                restore,
                label,
                recovery,
                thread_state(label),
                variant,
            );
            measured += 1;
        }
    }
    assert!(measured > 0, "selected thread fixture must exist");
}
