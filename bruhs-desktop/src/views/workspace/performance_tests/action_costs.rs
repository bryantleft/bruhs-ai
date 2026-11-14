use super::*;
use bruhs_models::{KnowledgeDocument, Meeting, MeetingNote, TranscriptSegment};
use serde_json::{json, Map, Value};

#[gpui::test]
#[ignore = "diagnostic optimized computation timings, not input-to-paint"]
fn native_action_audit_cost(cx: &mut TestAppContext) {
    palette_projection_cost(cx);
    palette_input_cost(cx);
    mixed_snapshot_cost(cx);
}

fn populate_threads(
    workspace: &mut Workspace,
    count: usize,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<'_, Workspace>,
) {
    workspace.sidebar_visible = false;
    workspace.active_view = AppView::Today;
    workspace.active_thread_id = None;
    workspace.threads = (0..count)
        .map(|index| {
            let mut thread = Workspace::create_thread_with_title(
                String::new(),
                String::new(),
                workspace.current_model.clone(),
                workspace.permission_mode,
                &format!("Thread {index}"),
                window,
                cx,
            );
            thread.updated_at_ms =
                bruhs_models::EpochMs::new(i64::try_from(index % 17).unwrap()).unwrap();
            thread.chat.update(cx, |chat, _| {
                chat.messages = vec![crate::views::chat::Message::user(format!(
                    "{} bodyquartz",
                    "Archived context. ".repeat(200)
                ))];
            });
            thread
        })
        .collect();
}

fn palette_projection_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    for count in [100, 1_000, 5_000] {
        workspace.update_in(cx, |workspace, window, cx| {
            populate_threads(workspace, count, window, cx);
        });
        cx.run_until_parked();
        for query in ["thread", "bodyquartz", "absentquartz"] {
            type Search = fn(&Workspace, &str, &gpui::App) -> Vec<super::super::PaletteItem>;
            for (version, search) in [
                ("legacy", legacy_matching_thread_items as Search),
                ("current", Workspace::matching_thread_items as Search),
            ] {
                let samples = (0..31)
                    .map(|_| {
                        let start = Instant::now();
                        workspace.read_with(cx, |workspace, cx| {
                            let items = search(workspace, black_box(query), cx)
                                .into_iter()
                                .take(super::super::PALETTE_THREAD_LIMIT_MIXED)
                                .collect::<Vec<_>>();
                            assert_eq!(
                                items.len(),
                                if query == "absentquartz" {
                                    0
                                } else {
                                    super::super::PALETTE_THREAD_LIMIT_MIXED.min(count)
                                }
                            );
                            black_box(items);
                        });
                        start.elapsed()
                    })
                    .collect();
                print_samples(
                    &format!("palette thread projection / {count} threads / {query} / {version}"),
                    samples,
                );
            }
        }
    }
}

// The former production path, retained for same-binary diagnostic comparisons.
fn legacy_matching_thread_items(
    workspace: &Workspace,
    query: &str,
    cx: &gpui::App,
) -> Vec<super::super::PaletteItem> {
    let mut threads = workspace
        .threads
        .iter()
        .filter(|thread| {
            if query.is_empty() {
                return true;
            }
            let chat = thread.chat.read(cx);
            super::super::text_matches_query(&chat.thread_title, query)
                || super::super::text_matches_query(&thread.project_name, query)
                || chat
                    .messages
                    .iter()
                    .any(|message| super::super::text_matches_query(&message.content, query))
        })
        .collect::<Vec<_>>();
    threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at_ms));
    threads
        .into_iter()
        .map(|thread| workspace.thread_palette_item(thread, cx))
        .collect()
}

#[gpui::test]
fn bounded_thread_picks_preserve_recency_ties_field_matching_and_keyboard_activation(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let expected = workspace.update_in(cx, |workspace, window, cx| {
        populate_threads(workspace, 37, window, cx);
        for query in [
            "",
            "thread",
            "bodyquartz",
            "ARCHIVED BODYQUARTZ",
            "thread bodyquartz",
            "absentquartz",
        ] {
            let expected = legacy_matching_thread_items(workspace, query, cx)
                .into_iter()
                .take(super::super::PALETTE_THREAD_LIMIT_MIXED)
                .map(|item| item.selector())
                .collect::<Vec<_>>();
            let actual = workspace
                .matching_thread_items(query, cx)
                .into_iter()
                .map(|item| item.selector())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "query {query:?}");
        }
        let expected = [16, 33, 15, 32, 14, 31].map(|index| workspace.threads[index].id.clone());
        let actual = workspace
            .matching_thread_items("thread", cx)
            .into_iter()
            .map(|item| match item {
                super::super::PaletteItem::Thread { id, .. } => id,
                _ => panic!("thread pick"),
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
        workspace.focus_command_bar(&super::super::FocusCommandBar, window, cx);
        expected[0].clone()
    });
    cx.run_until_parked();
    cx.simulate_input("bodyquartz");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.active_thread_id.as_deref(),
            Some(expected.as_str())
        );
        assert!(!workspace.command_bar_visible());
    });
}

fn cache(meetings: usize, segments_each: usize) -> super::super::MeetingCache {
    let mut records = Map::new();
    let mut transcripts = Map::new();
    for index in 0..meetings {
        let mut meeting = Meeting::new(format!("Saved meeting {index}"), 1.0);
        meeting.id = format!("meeting-{index}");
        let id = meeting.id.clone();
        records.insert(id.clone(), serde_json::to_value(meeting).unwrap());
        let segments = (0..segments_each)
            .map(|segment| {
                let start = u32::try_from(segment).unwrap().saturating_mul(1_000);
                TranscriptSegment {
                    id: format!("segment-{index}-{segment}"),
                    meeting_id: id.clone(),
                    speaker_id: None,
                    start_ms: start,
                    end_ms: start + 1_000,
                    text: "Saved meeting content with complete searchable text. ".repeat(6),
                    confidence: 0.9,
                    partial: false,
                    speaker_confidence: None,
                }
            })
            .collect::<Vec<_>>();
        transcripts.insert(id, serde_json::to_value(segments).unwrap());
    }
    serde_json::from_value(json!({"records": records, "transcripts": transcripts})).unwrap()
}

fn palette_input_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.sidebar_visible = false;
        workspace.active_view = AppView::Today;
        workspace.meeting_notes = (0..10_000)
            .map(|index| {
                MeetingNote::new(
                    format!("Planning {index}"),
                    format!("notequartz {index} {}", "Local context. ".repeat(16)),
                    1,
                )
                .unwrap()
            })
            .collect();
        workspace.focus_command_bar(&super::super::FocusCommandBar, window, cx);
        workspace.universal_search("notequartz", cx);
    });
    cx.run_until_parked();
    for (label, prefix, input) in [
        ("full input", "", "notequartz"),
        ("final character", "notequart", "z"),
    ] {
        let mut samples = Vec::new();
        for sample in 0..23 {
            workspace.update_in(cx, |workspace, window, cx| {
                workspace
                    .command_input
                    .update(cx, |input, cx| input.set_value(prefix, window, cx));
            });
            cx.run_until_parked();
            let start = Instant::now();
            cx.simulate_input(input);
            cx.run_until_parked();
            if sample >= 3 {
                samples.push(start.elapsed());
            }
        }
        print_samples(
            &format!("palette input + test layout / 10,000 notes / {label}"),
            samples,
        );
    }
    let samples = (0..20)
        .map(|_| {
            let start = Instant::now();
            cx.simulate_keystrokes("down");
            cx.run_until_parked();
            start.elapsed()
        })
        .collect();
    print_samples("palette selection + test layout / 10,000 notes", samples);
}

#[test]
fn compact_state_save_reads_legacy_json_and_preserves_its_recovery_backup() {
    use super::super::{
        desktop_state_backup_path, load_desktop_state_from_path, load_desktop_state_with_backup,
        read_desktop_state_snapshot, save_desktop_state, write_desktop_state_atomically,
        AtomicOrdering, PersistedWorkspaceState, PersistenceCoordinator,
    };
    use std::sync::{Arc, Mutex};

    struct Files(std::path::PathBuf);
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let files =
        Files(std::env::temp_dir().join(format!("bruhs-compact-state-{}", uuid::Uuid::new_v4())));
    let path = files.0.join("state.json");
    let mut document = KnowledgeDocument::new("é notes", "  Exact\n\n🗒 \"quoted\" \\ text  ", 1);
    document.revise("Revised", "Revised content", vec!["tag".into()], 2);
    let mut state = PersistedWorkspaceState {
        current_model: Some("legacy".into()),
        knowledge_documents: vec![document],
        meeting_notes: vec![MeetingNote::new("Local".into(), "Exact note text".into(), 1).unwrap()],
        meeting_cache: cache(1, 2),
        ..Default::default()
    };
    let original = serde_json::to_value(&state).unwrap();
    // This fixture deliberately uses the former writer's pretty format.
    write_desktop_state_atomically(&path, &state, 1).unwrap();
    let original_bytes = std::fs::read(&path).unwrap();
    assert!(original_bytes.starts_with(b"{\n"));
    let (loaded, version) = read_desktop_state_snapshot(&path).unwrap();
    assert_eq!(serde_json::to_value(loaded.unwrap()).unwrap(), original);
    let coordinator = Arc::new(PersistenceCoordinator {
        write_lock: Mutex::new(version),
        ..Default::default()
    });
    coordinator
        .latest_generation
        .store(2, AtomicOrdering::Release);
    state.current_model = Some("compact".into());
    state.meeting_notes[0].body.push_str("\nupdated");
    let expected = serde_json::to_value(&state).unwrap();
    let expected_bytes = serde_json::to_vec(&state).unwrap();
    save_desktop_state(Some(path.clone()), state, coordinator, 2).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected_bytes);
    assert_eq!(
        std::fs::read(desktop_state_backup_path(&path)).unwrap(),
        original_bytes
    );
    assert_eq!(
        serde_json::to_value(load_desktop_state_from_path(&path).unwrap().unwrap()).unwrap(),
        expected
    );
    std::fs::write(&path, b"invalid JSON").unwrap();
    let (recovered, warning, _) = load_desktop_state_with_backup(&path);
    assert!(warning.is_some());
    assert_eq!(serde_json::to_value(recovered.unwrap()).unwrap(), original);
}

fn mixed_snapshot_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    for (label, documents, meetings, messages) in [
        ("meeting-heavy", 0, 100, 0),
        ("knowledge-heavy", 5_000, 0, 0),
        ("mixed", 1_000, 50, 5_000),
    ] {
        let meeting_cache = cache(meetings, 1_000);
        workspace.update(cx, |workspace, cx| {
            workspace.sidebar_visible = false;
            workspace.active_view = AppView::Search;
            workspace.knowledge_documents = (0..documents)
                .map(|index| {
                    let mut document = KnowledgeDocument::new(
                        format!("Document {index}"),
                        "Saved Markdown notes and complete document context. ".repeat(160),
                        1,
                    );
                    if label == "mixed" {
                        document.revise(
                            "Revised title",
                            format!("{} revised", document.body_markdown),
                            Vec::new(),
                            2,
                        );
                        document.revise(
                            "Final title",
                            format!("{} final", document.body_markdown),
                            Vec::new(),
                            3,
                        );
                    }
                    document
                })
                .collect();
            workspace.meeting_notes = if label == "mixed" {
                (0..1_000)
                    .map(|index| {
                        MeetingNote::new(
                            format!("Note {index}"),
                            "Local meeting note. ".repeat(50),
                            1,
                        )
                        .unwrap()
                    })
                    .collect()
            } else {
                Vec::new()
            };
            workspace
                .meetings_hub
                .update(cx, |hub, _| hub.restore_meetings(meeting_cache));
            let chat = workspace.active_thread().unwrap().chat.clone();
            chat.update(cx, |chat, _| {
                chat.messages = (0..messages)
                    .map(|_| crate::views::chat::Message::user("Saved chat content. ".repeat(200)))
                    .collect();
            });
        });
        cx.run_until_parked();
        let samples = (0..31)
            .map(|_| {
                let start = Instant::now();
                workspace.read_with(cx, |workspace, cx| black_box(workspace.snapshot(cx)));
                start.elapsed()
            })
            .collect();
        print_samples(&format!("workspace snapshot / {label}"), samples);
        let state = workspace.read_with(cx, |workspace, cx| workspace.snapshot(cx));
        for pretty in [true, false] {
            let mut bytes_len = 0;
            let samples = (0..11)
                .map(|_| {
                    let start = Instant::now();
                    let bytes = if pretty {
                        serde_json::to_vec_pretty(black_box(&state)).unwrap()
                    } else {
                        serde_json::to_vec(black_box(&state)).unwrap()
                    };
                    bytes_len = bytes.len();
                    black_box(bytes);
                    start.elapsed()
                })
                .collect();
            print_samples(
                &format!("snapshot JSON / {label} / pretty={pretty} / {bytes_len} bytes"),
                samples,
            );
        }
        let pretty: Value =
            serde_json::from_slice(&serde_json::to_vec_pretty(&state).unwrap()).unwrap();
        let compact: Value = serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap();
        assert_eq!(pretty, compact);
    }
}
