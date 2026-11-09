use super::*;
use bruhs_models::MeetingNote;
mod manual;

fn local_workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                crate::server_client::ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                    .unwrap(),
            );
        })
    });
    (workspace, cx)
}

#[gpui::test]
fn local_meeting_note_should_join_universal_search(cx: &mut TestAppContext) {
    let (workspace, cx) = local_workspace(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.meeting_notes =
            vec![
                MeetingNote::new("Local planning".into(), "Unique notequartz body".into(), 1)
                    .unwrap(),
            ];
        let hits = w.universal_search("notequartz", cx).1;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.id, w.meeting_notes[0].id);
        assert_eq!(hits[0].document.kind, bruhs_models::SearchKind::MeetingNote);
    });
}

#[gpui::test]
fn palette_keyboard_opens_exact_note_beyond_first_page_without_opening_same_id_recording(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = local_workspace(cx);
    let original = workspace.update_in(cx, |w, window, cx| {
        w.meeting_notes = (0..101)
            .map(|index| {
                MeetingNote::new(format!("Planning {index}"), format!("Body {index}"), 1).unwrap()
            })
            .collect();
        let note = &mut w.meeting_notes[100];
        note.body = "  Unique notequartz body\n\nExact final line 🗒  ".into();
        let original = note.clone();
        w.meetings_hub.update(cx, |hub, _| {
            let mut recording = bruhs_models::Meeting::new("Different recording", 1.);
            recording.id = original.id.clone();
            // A restored empty transcript keeps this local navigation fixture
            // independent of the background recording hydration path.
            hub.restore_meetings(
                serde_json::from_value(serde_json::json!({
                    "records": {(original.id.clone()): recording},
                    "transcripts": {(original.id.clone()): []}
                }))
                .unwrap(),
            );
        });
        w.focus_command_bar(&FocusCommandBar, window, cx);
        original
    });
    cx.run_until_parked();
    cx.simulate_input("notequartz");
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.filtered_palette_items(cx).len(), 1)
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let content = cx
        .debug_bounds("desktop-local-meeting-note-content")
        .unwrap();
    assert!(content.size.width > gpui::px(0.) && content.size.height > gpui::px(0.));
    workspace.update_in(cx, |w, window, _| {
        assert_eq!(
            w.active_view,
            AppView::MeetingNoteDetail {
                note_id: original.id.clone()
            }
        );
        assert!(w.search_record_focus.is_focused(window));
        assert!(!w.command_bar_visible());
        assert_eq!(
            MeetingNote::find_unique(&w.meeting_notes, &original.id).unwrap(),
            &original
        );
    });
    let back = cx.debug_bounds("desktop-back-to-meeting-notes").unwrap();
    cx.simulate_click(back.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, _| assert_eq!(w.active_view, AppView::Meetings));
}

#[gpui::test]
fn local_note_route_shows_missing_or_ambiguous_identity_without_mutating_records(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = local_workspace(cx);
    for duplicate in [false, true] {
        let original = workspace.update_in(cx, |w, window, cx| {
            let mut note = MeetingNote::new("Original".into(), "Exact body".into(), 1).unwrap();
            note.id = "note-id".into();
            w.meeting_notes = if duplicate {
                vec![note.clone(), note]
            } else {
                vec![]
            };
            w.open_local_meeting_note("note-id".into(), window, cx);
            w.meeting_notes.clone()
        });
        cx.run_until_parked();
        assert!(cx
            .debug_bounds("desktop-local-meeting-note-error")
            .is_some());
        assert!(cx
            .debug_bounds("desktop-local-meeting-note-content")
            .is_none());
        workspace.update_in(cx, |w, _, _| assert_eq!(w.meeting_notes, original));
    }
}

#[gpui::test]
fn note_search_invalidates_after_saved_edit_and_removal(cx: &mut TestAppContext) {
    let (workspace, cx) = local_workspace(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.meeting_notes =
            vec![MeetingNote::new("Planning".into(), "Old notequartz".into(), 1).unwrap()];
        assert_eq!(w.universal_search("old notequartz", cx).1.len(), 1);
        w.meeting_notes[0].body = "New notequartz".into();
        w.meeting_notes[0].updated_at = 2;
        w.persist_and_notify(cx);
        assert!(w.universal_search("old notequartz", cx).1.is_empty());
        assert_eq!(w.universal_search("new notequartz", cx).1.len(), 1);
        w.meeting_notes.clear();
        w.persist_and_notify(cx);
        assert!(w.universal_search("notequartz", cx).1.is_empty());
    });
}

#[gpui::test]
fn local_meeting_note_search_timing(cx: &mut TestAppContext) {
    use std::time::Instant;
    let (workspace, cx) = local_workspace(cx);
    let indexed = workspace.update_in(cx, |w, window, cx| {
        w.meeting_notes = (0..10_000)
            .map(|index| {
                MeetingNote::new(
                    format!("Planning {index}"),
                    format!("notequartz {index} {}", "Local context. ".repeat(16)),
                    1,
                )
                .unwrap()
            })
            .collect();
        w.focus_command_bar(&FocusCommandBar, window, cx);
        w.universal_search("notequartz", cx).0
    });
    cx.run_until_parked();
    let samples = if cfg!(debug_assertions) { 3 } else { 20 };
    let mut times = Vec::new();
    for sample in 0..samples + 3 {
        workspace.update_in(cx, |w, window, cx| {
            w.command_input
                .update(cx, |input, cx| input.set_value("", window, cx))
        });
        cx.run_until_parked();
        let start = Instant::now();
        cx.simulate_input("notequartz");
        cx.run_until_parked();
        let elapsed = start.elapsed().as_secs_f64() * 1_000.;
        if sample >= 3 {
            times.push(elapsed);
        }
        workspace.update_in(cx, |w, _, cx| {
            assert_eq!(
                w.filtered_palette_items(cx).len(),
                super::super::PALETTE_RECORD_LIMIT
            )
        });
    }
    times.sort_by(f64::total_cmp);
    eprintln!("meeting note palette: notes=10000 indexed={indexed} samples={samples}; warm headless input + layout p50={:.3}ms p95={:.3}ms p99={:.3}ms; excludes fixture/index construction, hardware/GPU paint, transport and disk completion", times[times.len()/2], times[(times.len()*95/100).min(times.len()-1)], times[(times.len()*99/100).min(times.len()-1)]);
}
