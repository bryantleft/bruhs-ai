//! Full temporary-file loading through the real workspace constructor and test frame settling.
use super::super::{
    load_desktop_state_with_backup, PermissionMode, PersistedThread, PersistedWorkspaceState,
    RestoredRecords, SaveStatus,
};
use super::*;
use bruhs_models::{CaptureKind, CaptureRecord, KnowledgeDocument, MeetingNote, SourceClip};
use serde_json::{json, Value};
use sha2::{Digest as _, Sha256};
use std::path::Path;

mod cleanup;
mod decoder;
mod project_path_reference;
mod reference;
mod sidebar;
mod thread_reference;
mod threads;

type RestoreRecords = fn(Option<&mut PersistedWorkspaceState>) -> RestoredRecords;

fn original(state: Option<&mut PersistedWorkspaceState>) -> RestoredRecords {
    RestoredRecords::original_from_persisted(state.as_deref())
}

fn current(state: Option<&mut PersistedWorkspaceState>) -> RestoredRecords {
    RestoredRecords::take_from_persisted(state)
}

fn initialize(cx: &mut TestAppContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
}

fn saved_thread(index: usize, messages: usize) -> PersistedThread {
    serde_json::from_value(json!({
        "id": format!("thread-{index}"), "title": format!("Saved thread {index}"),
        "project_name": "", "project_path": "/synthetic/workspace-load-fixture",
        "model_id": "claude-sonnet-4-6", "permission_mode": "default",
        "updated_at_ms": 1_600_000_000_000_i64,
        "messages": (0..messages).map(|message| json!({
            "role":"user", "content":format!("Message {index}/{message}: {}",
                "Saved café, 👩🏽‍💻, quotes \" and newlines\n".repeat(16)),
        })).collect::<Vec<_>>(),
    }))
    .unwrap()
}

fn saved_state(
    label: &str,
    documents: usize,
    records: usize,
    messages: usize,
) -> PersistedWorkspaceState {
    let mut state = PersistedWorkspaceState {
        current_model: Some("claude-sonnet-4-6".into()),
        permission_mode: Some(PermissionMode::Default),
        threads: (0..messages.div_ceil(128).max(1))
            .map(|index| saved_thread(index, messages.saturating_sub(index * 128).min(128)))
            .collect(),
        editor_id: Some("zed".into()),
        title_instructions: Some("Keep exact saved title instructions. café 👩🏽‍💻".into()),
        calendar_anchor: Some("2026-05-12".into()),
        selected_work_item_id: Some("task-0".into()),
        update_checks: Some(false),
        ..Default::default()
    };
    for (index, rule) in state
        .scheduling_preferences
        .availability
        .iter_mut()
        .enumerate()
    {
        rule.id = format!("00000000-0000-4000-8000-{index:012}");
    }
    for index in 0..documents {
        let mut document = KnowledgeDocument::new(
            format!("Document {index}"),
            "Saved Markdown, café, 👩🏽‍💻, quotes \" and newlines\n".repeat(80),
            1,
        );
        document.id = format!("document-{index}");
        document.revise(
            "Revised",
            format!("{} revision", document.body_markdown),
            vec!["saved".into()],
            2,
        );
        // Version ids are random on creation; fixtures must be identical in fresh processes.
        for (revision, version) in document.versions.iter_mut().enumerate() {
            version.id = format!("version-{index}-{revision}");
        }
        state.knowledge_documents.push(document);
    }
    for index in 0..records {
        state.projects.push(super::super::PersistedProject {
            id: format!("project-{index}"),
            name: format!("Project {index}"),
        });
        state.work_items.push(super::super::PersistedWorkItem {
            id: format!("task-{index}"),
            project_id: Some(format!("project-{index}")),
            title: format!("Task {index}"),
            description: Some("Complete task details. café 👩🏽‍💻\n".repeat(24)),
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::Medium,
            due_date: Some("2026-05-12".into()),
            planning: Default::default(),
            updated_at: bruhs_models::EpochMs::new(1).unwrap(),
        });
        state
            .calendar_events
            .push(super::super::PersistedCalendarEvent {
                id: format!("event-{index}"),
                title: format!("Event {index}"),
                start_at: "2026-05-12T09:00:00".into(),
                end_at: "2026-05-12T10:00:00".into(),
                all_day: index % 2 == 0,
                color: Some("pandan".into()),
                notes: Some("Complete calendar notes. ".repeat(16)),
                ..Default::default()
            });
        let mut capture = CaptureRecord::new(
            CaptureKind::Note,
            &format!("Capture {index}\n{}", "Saved capture body. ".repeat(16)),
            1,
        )
        .unwrap();
        capture.id = format!("capture-{index}");
        state.captures.push(capture);
        let mut note = MeetingNote::new(
            format!("Note {index}"),
            "Saved meeting notes. ".repeat(24),
            1,
        )
        .unwrap();
        note.id = format!("note-{index}");
        state.meeting_notes.push(note);
        let mut clip = SourceClip::new(
            format!("Clip {index}"),
            "https://example.invalid/source",
            "Saved excerpt. ".repeat(24),
            1,
        );
        clip.id = format!("clip-{index}");
        state.source_clips.push(clip);
        let mut relationship = bruhs_models::RelationshipProfile::new(format!("person-{index}"), 1);
        relationship.notes = "Private relationship context. ".repeat(24);
        state.relationships.push(relationship);
        let mut transaction = bruhs_models::FinanceTransaction::manual(
            format!("Merchant {index}"),
            "Saved category",
            i64::try_from(index).unwrap(),
            1,
        );
        transaction.id = format!("transaction-{index}");
        transaction.note = Some("Complete saved receipt context. ".repeat(8));
        state
            .life_context
            .finance
            .transactions
            .push(std::sync::Arc::new(transaction));
        state
            .recent_projects
            .push(super::super::PersistedProjectLocation {
                path: format!("/synthetic/{index}"),
                name: format!("Project {index}"),
            });
    }
    state.inbox_conversations = super::inbox_records(records.min(256));
    // Inbox factories allocate ids independently; normalize every generated id for cross-process checks.
    for (index, conversation) in state.inbox_conversations.iter_mut().enumerate() {
        for (message, item) in conversation.messages.iter_mut().enumerate() {
            item.id = format!("inbox-{index}-{message}");
        }
    }
    if label == "meeting-control" || label == "mixed" {
        let mut meeting = bruhs_models::Meeting::new("Saved meeting", 1.0);
        meeting.id = "meeting-0".into();
        meeting.status = bruhs_models::MeetingStatus::Ended;
        meeting.ended_at_ms = Some(2.0);
        state.meeting_cache=serde_json::from_value(json!({
            "records":{"meeting-0":meeting},
            "transcripts":{"meeting-0":(0..8192).map(|index| json!({
                "id":format!("segment-{index}"),"meeting_id":"meeting-0","start_ms":index*1000,"end_ms":index*1000+1000,
                "text":"Complete saved speech. café 👩🏽‍💻\n".repeat(8),"confidence":0.9,"partial":false,"speaker_id":null,
            })).collect::<Vec<_>>()},
        })).unwrap();
    }
    assert!(state.automation_state.jobs.is_empty());
    assert!(state
        .threads
        .iter()
        .all(|thread| thread.prompt_queue.is_empty() && thread.branch.is_none()));
    state
}

/// The frozen restorer seeds the focus suggestion in its old wording, even
/// without tasks; since PM-39 the current one offers the web's shared
/// suggestion once an unfinished task exists. Both snapshots set their
/// suggestions aside so the rest compares exactly; the current ones return.
fn set_aside_focus_suggestions(expected: &mut Observation, actual: &mut Observation) -> Value {
    expected.snapshot["automation_state"]["suggestions"].take();
    actual.snapshot["automation_state"]["suggestions"].take()
}

struct Observation {
    total_ms: f64,
    read_decode_hash_ms: f64,
    construct_ms: f64,
    settle_ms: f64,
    snapshot: Value,
    recovered: bool,
}

fn observe(cx: &mut TestAppContext, path: &Path, restore: RestoreRecords) -> Observation {
    observe_with_threads(cx, path, restore, threads::current_threads)
}

#[inline(never)]
fn observe_with_threads(
    cx: &mut TestAppContext,
    path: &Path,
    restore: RestoreRecords,
    restore_threads: super::super::ThreadRestorer,
) -> Observation {
    observe_with_loader(
        cx,
        path,
        restore,
        restore_threads,
        load_desktop_state_with_backup,
    )
}

type LoadState = fn(
    &Path,
) -> (
    Option<PersistedWorkspaceState>,
    Option<String>,
    super::super::DiskVersion,
);

#[inline(never)]
fn observe_with_loader(
    cx: &mut TestAppContext,
    path: &Path,
    restore: RestoreRecords,
    restore_threads: super::super::ThreadRestorer,
    load: LoadState,
) -> Observation {
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (root, visual) = cx.add_window_view(|window, cx| {
        let started = Instant::now();
        let loaded = load(path);
        let expected_version = loaded.2.clone();
        let read_decode_hash_ms = started.elapsed().as_secs_f64() * 1000.0;
        let workspace =
            cx.new(|cx| Workspace::from_loaded_state(loaded, window, cx, restore, restore_threads));
        let root = bruhs_ui::Root::new(workspace.clone(), window, cx);
        let constructed = started.elapsed().as_secs_f64() * 1000.0;
        *capture.borrow_mut() = Some((
            workspace,
            started,
            read_decode_hash_ms,
            constructed,
            expected_version,
        ));
        root
    });
    // add_window_view runs the test executor until parked, including the initial headless frame.
    let (workspace, started, read_decode_hash_ms, constructed, expected_version) =
        captured.borrow_mut().take().unwrap();
    let total_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert!(visual.debug_bounds("desktop-composer").is_some());
    let (snapshot, recovered) = workspace.read_with(visual, |workspace, cx| {
        assert_eq!(
            *workspace.persistence.write_lock.lock().unwrap(),
            expected_version
        );
        assert_eq!(
            workspace.persist_requests, 0,
            "startup must not save or run a workflow"
        );
        assert!(workspace
            .threads
            .iter()
            .all(|thread| !thread.chat.read(cx).is_streaming));
        assert!(workspace.threads.iter().all(|thread| {
            let chat = thread.chat.read(cx);
            chat.prompt_queue.is_empty() || chat.queue_held
        }));
        (
            serde_json::to_value(workspace.snapshot(cx)).unwrap(),
            matches!(workspace.save_status, SaveStatus::RecoveryRequired(_)),
        )
    });
    cleanup::close_fixture(root, workspace, visual);
    Observation {
        total_ms,
        read_decode_hash_ms,
        construct_ms: constructed - read_decode_hash_ms,
        settle_ms: total_ms - constructed,
        snapshot,
        recovered,
    }
}

fn write_fixture(path: &Path, state: &PersistedWorkspaceState, recovery: bool) -> Vec<u8> {
    let bytes = serde_json::to_vec(state).unwrap();
    if recovery {
        std::fs::write(path, b"{ invalid primary").unwrap();
        std::fs::write(super::super::desktop_state_backup_path(path), &bytes).unwrap();
    } else {
        std::fs::write(path, &bytes).unwrap();
    }
    bytes
}

fn normalize_fresh_rule_ids(snapshot: &mut Value) {
    for (index, rule) in snapshot["scheduling_preferences"]["availability"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        uuid::Uuid::parse_str(rule["id"].as_str().unwrap()).unwrap();
        rule["id"] = json!(format!("00000000-0000-4000-8000-{index:012}"));
    }
}

#[test]
fn workspace_load_fixture_inputs_are_reproducible() {
    let first = serde_json::to_vec(&saved_state("mixed", 3, 3, 129)).unwrap();
    let second = serde_json::to_vec(&saved_state("mixed", 3, 3, 129)).unwrap();
    assert_eq!(Sha256::digest(&first), Sha256::digest(&second));
}

#[gpui::test]
fn complete_workspace_load_preserves_original_state_and_backup_recovery(cx: &mut TestAppContext) {
    initialize(cx);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("workspace.json");
    let mut state = saved_state("mixed", 3, 3, 5);
    let mut invalid = state.calendar_events[0].clone();
    invalid.id = "invalid-date".into();
    invalid.end_at = "invalid".into();
    state.calendar_events.push(invalid);
    for recovery in [false, true] {
        let bytes = write_fixture(&path, &state, recovery);
        let mut expected = observe(cx, &path, original);
        let mut actual = observe(cx, &path, current);
        let suggestions = set_aside_focus_suggestions(&mut expected, &mut actual);
        assert_eq!(actual.snapshot, expected.snapshot);
        assert_eq!(actual.recovered, recovery);
        assert_eq!(
            actual.snapshot["calendar_events"].as_array().unwrap().len(),
            3
        );
        let input: Value = serde_json::from_slice(&bytes).unwrap();
        for field in [
            "projects",
            "work_items",
            "captures",
            "knowledge_documents",
            "source_clips",
            "inbox_conversations",
            "meeting_notes",
            "relationships",
            "recent_projects",
            "life_context",
            "scheduling_preferences",
            "title_instructions",
        ] {
            assert_eq!(actual.snapshot[field], input[field], "{field}");
        }
        assert_eq!(suggestions.as_array().unwrap().len(), 1);
        assert_eq!(
            suggestions[0]["evidence"],
            "3 unfinished tasks and 3 calendar entries are stored locally."
        );
    }
    std::fs::remove_file(&path).unwrap();
    let mut expected_missing = observe(cx, &path, original);
    let mut missing = observe(cx, &path, current);
    let suggestions = set_aside_focus_suggestions(&mut expected_missing, &mut missing);
    assert_eq!(
        suggestions,
        json!([]),
        "no unfinished task, so no focus suggestion yet"
    );
    for observation in [&mut expected_missing, &mut missing] {
        assert_eq!(observation.snapshot["threads"].as_array().unwrap().len(), 1);
        assert_eq!(observation.snapshot["threads"][0]["messages"], json!([]));
        assert_eq!(
            observation.snapshot["active_thread_id"],
            observation.snapshot["threads"][0]["id"]
        );
        // Each fresh workspace intentionally creates its own thread id and activity timestamp.
        observation.snapshot["active_thread_id"] = json!("fresh-thread");
        observation.snapshot["threads"][0]["id"] = json!("fresh-thread");
        observation.snapshot["threads"][0]["updated_at_ms"] = json!(0);
        normalize_fresh_rule_ids(&mut observation.snapshot);
    }
    assert_eq!(missing.snapshot, expected_missing.snapshot);
    assert!(
        !missing.recovered,
        "missing primary retains existing fresh-state behavior even with backup"
    );
    assert_eq!(missing.snapshot["knowledge_documents"], json!([]));
}

#[gpui::test]
#[ignore = "manual optimized full temporary-file workspace construction and headless initial frame timing"]
fn complete_workspace_load_cost(cx: &mut TestAppContext) {
    initialize(cx);
    let (variant, restore): (&str, RestoreRecords) =
        match std::env::var("BRUHS_PERF_WORKSPACE_RESTORE").as_deref() {
            Ok("original") => ("original", original),
            Ok("current") | Err(_) => ("current", current),
            other => panic!("invalid restore selection: {other:?}"),
        };
    let root = tempfile::tempdir().unwrap();
    for (label, documents, records, messages) in [
        ("settings-control", 0, 0, 0),
        ("knowledge-small", 256, 0, 0),
        ("knowledge-large", 4096, 0, 0),
        ("records-large", 0, 4096, 0),
        ("history-control", 0, 0, 8192),
        ("meeting-control", 0, 0, 0),
        ("mixed", 1024, 1024, 2048),
    ] {
        let state = saved_state(label, documents, records, messages);
        for recovery in [false, true] {
            let path = root.path().join(format!("{label}-{recovery}.json"));
            let bytes = write_fixture(&path, &state, recovery);
            let mut expected = observe(cx, &path, original);
            let mut total = Vec::new();
            let mut load = Vec::new();
            let mut construct = Vec::new();
            let mut settle = Vec::new();
            for iteration in 0..35 {
                let mut observation = observe(cx, &path, restore);
                set_aside_focus_suggestions(&mut expected, &mut observation);
                assert_eq!(
                    observation.snapshot, expected.snapshot,
                    "{label}, recovery={recovery}, iteration={iteration}"
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
                    super::super::desktop_state_backup_path(&path)
                } else {
                    path.clone()
                })
                .unwrap(),
                bytes
            );
            println!(
                "WORKSPACE_LOAD_COST {}",
                json!({"label":label,"recovery":recovery,"variant":variant,"input_bytes":bytes.len(),"input_sha256":format!("{:x}",Sha256::digest(&bytes)),"warmups":5,"total_samples_ms":total,"load_samples_ms":load,"construct_samples_ms":construct,"settle_samples_ms":settle})
            );
            std::fs::remove_file(&path).unwrap();
            if recovery {
                std::fs::remove_file(super::super::desktop_state_backup_path(&path)).unwrap();
            }
        }
    }
}

#[test]
fn workspace_record_transfer_reuses_allocations_and_keeps_other_saved_fields() {
    let mut state = saved_state("contract", 3, 3, 5);
    state.active_project = Some(super::super::PersistedProjectLocation {
        path: "/synthetic/active-project".into(),
        name: "Saved active project".into(),
    });
    state.automation_state = RestoredRecords::original_from_persisted(None).automation_state;
    state.automation_state.suggestions[0].evidence = "Keep the user's existing suggestion".into();
    state.automation_state.suggestions[0].dismissed = true;
    state.scheduling_preferences.updated_at_ms = 72;
    let suggestions = serde_json::to_value(&state.automation_state.suggestions).unwrap();
    let mut expected_remaining = serde_json::to_value(&state).unwrap();
    let defaults = serde_json::to_value(PersistedWorkspaceState::default()).unwrap();
    for field in [
        "projects",
        "work_items",
        "calendar_events",
        "captures",
        "knowledge_documents",
        "source_clips",
        "inbox_conversations",
        "meeting_notes",
        "relationships",
        "scheduling_preferences",
        "life_context",
        "automation_state",
        "active_project",
        "recent_projects",
    ] {
        expected_remaining[field] = defaults[field].clone();
    }
    let documents = state.knowledge_documents.as_ptr();
    let body = state.knowledge_documents[0].body_markdown.as_ptr();
    let task_title = state.work_items[0].title.as_ptr();
    let capture = state.captures.as_ptr();
    let notes = state.meeting_notes.as_ptr();
    let clips = state.source_clips.as_ptr();
    let conversations = state.inbox_conversations.as_ptr();
    let relationships = state.relationships.as_ptr();
    let finance = state.life_context.finance.transactions.as_ptr();
    let active_path = state.active_project.as_ref().unwrap().path.as_ptr();
    let recent_path = state.recent_projects[0].path.as_ptr();
    let records = RestoredRecords::take_from_persisted(Some(&mut state));
    assert_eq!(records.knowledge_documents.as_ptr(), documents);
    assert_eq!(records.knowledge_documents[0].body_markdown.as_ptr(), body);
    assert_eq!(records.work_items[0].title.as_ptr(), task_title);
    assert_eq!(records.captures.as_ptr(), capture);
    assert_eq!(records.meeting_notes.as_ptr(), notes);
    assert_eq!(records.source_clips.as_ptr(), clips);
    assert_eq!(records.inbox_conversations.as_ptr(), conversations);
    assert_eq!(records.relationships.as_ptr(), relationships);
    assert_eq!(records.life_context.finance.transactions.as_ptr(), finance);
    assert_eq!(
        records.active_project.as_ref().unwrap().path.as_ptr(),
        active_path
    );
    assert_eq!(records.recent_projects[0].path.as_ptr(), recent_path);
    assert_eq!(
        serde_json::to_value(&records.automation_state.suggestions).unwrap(),
        suggestions
    );
    assert_eq!(records.scheduling_preferences.updated_at_ms, 72);
    assert_ne!(
        state.scheduling_preferences.availability[0].id,
        records.scheduling_preferences.availability[0].id
    );
    let mut remaining = serde_json::to_value(&state).unwrap();
    normalize_fresh_rule_ids(&mut remaining);
    normalize_fresh_rule_ids(&mut expected_remaining);
    for (field, expected) in expected_remaining.as_object().unwrap() {
        assert_eq!(&remaining[field], expected, "remaining field {field}");
    }
}
