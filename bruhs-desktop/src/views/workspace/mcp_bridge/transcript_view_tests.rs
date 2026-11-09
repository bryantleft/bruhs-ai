use super::tests::fixture;
use bruhs_models::{AppView, Meeting, MeetingEvent, Speaker, TranscriptSegment};
use gpui::{px, size, Focusable, ListOffset, TestAppContext};

fn install(
    w: &mut super::super::Workspace,
    count: usize,
    cx: &mut gpui::Context<'_, super::super::Workspace>,
) {
    let mut meeting = Meeting::new("Owned viewport fixture", 1.0);
    meeting.id = "viewport-meeting".into();
    let mut speaker = Speaker::new("Me");
    speaker.id = "me".into();
    meeting.speakers.push(speaker);
    w.meetings_hub.update(cx, |hub, cx| {
        hub.install_meeting_test_snapshot(meeting, records(count), cx);
    });
}

fn records(count: usize) -> Vec<TranscriptSegment> {
    (0..count)
        .map(|index| TranscriptSegment {
            id: format!("row-{index}"),
            meeting_id: "viewport-meeting".into(),
            speaker_id: Some("me".into()),
            start_ms: u32::try_from(index).unwrap() * 10,
            end_ms: u32::try_from(index).unwrap() * 10 + 9,
            text: format!("Transcript row {index}: {}", "received words ".repeat(16)),
            confidence: 0.9,
            partial: index + 1 == count,
            speaker_confidence: None,
        })
        .collect()
}

#[gpui::test]
fn local_focus_panel_keeps_transcript_visible_at_narrow_widths(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, 30, cx);
        w.meetings_hub.update(cx, |hub, _| {
            hub.live_meeting_id = Some("viewport-meeting".into());
            hub.local_focus.rows.insert(
                "viewport-meeting".into(),
                (0..10_000)
                    .map(|index| bruhs_models::LocalFocusObservation {
                        id: index.to_string(),
                        meeting_id: "viewport-meeting".into(),
                        observed_at_ms: index + 1,
                        focus: bruhs_models::LocalFocus::Available {
                            app_name: "Synthetic app with a longer localized name".into(),
                            bundle_id: None,
                        },
                    })
                    .collect(),
            );
        });
        w.active_view = AppView::MeetingDetail {
            meeting_id: "viewport-meeting".into(),
        };
        cx.notify();
    });
    for width in [320.0, 390.0, 1280.0] {
        cx.simulate_resize(size(px(width), px(844.0)));
        cx.run_until_parked();
        let panel = cx
            .debug_bounds("desktop-local-focus")
            .expect("local context panel");
        let toggle = cx
            .debug_bounds("desktop-local-focus-toggle")
            .expect("explicit opt-in");
        let transcript = cx
            .debug_bounds("desktop-transcript-viewport")
            .expect("transcript viewport");
        assert!(panel.size.height <= px(201.0), "unbounded context panel");
        assert!(
            panel.origin.x >= px(0.0) && panel.right() <= px(width),
            "panel overflow"
        );
        assert!(
            toggle.size.height >= px(44.0) && toggle.right() <= px(width),
            "toggle clipped"
        );
        assert!(
            transcript.origin.y >= panel.bottom() && transcript.size.height > px(100.0),
            "context covers transcript"
        );
        assert!(!workspace.read_with(cx, |w, cx| w
            .meetings_hub
            .read(cx)
            .local_focus
            .enabled_for("viewport-meeting")));
    }
}

// Deliberately measures element construction only, not GPU paint or input latency.
#[cfg(not(debug_assertions))]
#[gpui::test]
fn transcript_element_construction_measurement(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, 90_001, cx);
        // Keep the original pre-virtualization benchmark's unknown-speaker fixture.
        w.meetings_hub.update(cx, |hub, _| {
            hub.meetings[0].speakers.clear();
            for record in hub.segments.get_mut("viewport-meeting").unwrap() {
                record.speaker_id = None;
            }
        });
        let mut samples = Vec::new();
        for _ in 0..20 {
            let start = std::time::Instant::now();
            let element = w.render_meeting_detail("viewport-meeting", cx);
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
            std::hint::black_box(element);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "TRANSCRIPT_ELEMENT_BUILD 90001 rows p50={:.3}ms p95={:.3}ms",
            samples[10], samples[18]
        );
    });
}

#[cfg(not(debug_assertions))]
#[gpui::test]
fn transcript_headless_frame_measurement(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(size(px(1280.0), px(900.0)));
    workspace.update_in(cx, |w, _, cx| {
        install(w, 90_001, cx);
        w.active_view = AppView::MeetingDetail {
            meeting_id: "viewport-meeting".into(),
        };
        cx.notify();
    });
    cx.run_until_parked();
    let mut samples = Vec::new();
    for index in 0..105 {
        let start = std::time::Instant::now();
        workspace.update_in(cx, |w, _, cx| {
            w.transcript_rendered_rows.set(0);
            w.meetings_hub.update(cx, |hub, cx| {
                let mut tail = hub.segments["viewport-meeting"].last().unwrap().clone();
                tail.text = format!("Partial update {index}");
                hub.apply_meeting_event(
                    MeetingEvent::Segment {
                        meeting_id: "viewport-meeting".into(),
                        segment: tail,
                    },
                    cx,
                );
            });
        });
        cx.run_until_parked();
        if index >= 5 {
            samples.push(start.elapsed().as_secs_f64() * 1_000.0);
        }
    }
    samples.sort_by(f64::total_cmp);
    eprintln!(
        "TRANSCRIPT_HEADLESS_FRAME 90001 rows p50={:.3}ms p95={:.3}ms p99={:.3}ms",
        samples[50], samples[95], samples[99]
    );
    assert!(
        samples[95] < 16.0,
        "headless warmed stream/frame p95 exceeds 16ms"
    );
}

#[gpui::test]
fn native_transcript_large_history_renders_only_viewport_and_reaches_footer(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(size(px(1280.0), px(900.0)));
    workspace.update_in(cx, |w, _, cx| {
        install(w, 90_001, cx);
        w.active_view = AppView::MeetingDetail {
            meeting_id: "viewport-meeting".into(),
        };
        w.transcript_rendered_rows.set(0);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-transcript-row-0").is_some());
    let page = cx.debug_bounds("desktop-meeting-detail").unwrap();
    let first = cx.debug_bounds("desktop-transcript-row-0").unwrap();
    let expected_x = page.origin.x + ((page.size.width - px(768.0)) / 2.0).max(px(0.0));
    assert!(
        (first.origin.x - expected_x).abs() < px(1.0),
        "transcript must remain centered: {first:?} within {page:?}"
    );
    workspace.update_in(cx, |w, _, cx| {
        let count = w.transcript_rendered_rows.replace(0);
        assert!(
            count > 0 && count < 200,
            "built {count} rows, expected only viewport"
        );
        w.meeting_transcript_view
            .borrow()
            .as_ref()
            .unwrap()
            .list
            .scroll_to_end();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-meeting-chat").is_some(),
        "footer reachable without rendering preceding history"
    );
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.transcript_rendered_rows.get() < 200);
        assert_eq!(
            w.meetings_hub.read(cx).segments["viewport-meeting"].len(),
            90_001
        );
    });
}

#[gpui::test]
fn native_transcript_stream_update_preserves_middle_anchor(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, 10_001, cx);
        w.active_view = AppView::MeetingDetail {
            meeting_id: "viewport-meeting".into(),
        };
        cx.notify();
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        w.meeting_transcript_view
            .borrow()
            .as_ref()
            .unwrap()
            .list
            .scroll_to(ListOffset {
                item_ix: 5_000,
                offset_in_item: px(5.0),
            });
        cx.notify();
    });
    cx.run_until_parked();
    let before = workspace.update_in(cx, |w, _, _| {
        w.meeting_transcript_view
            .borrow()
            .as_ref()
            .unwrap()
            .list
            .logical_scroll_top()
    });
    workspace.update_in(cx, |w, _, cx| {
        w.transcript_rendered_rows.set(0);
        w.meetings_hub.update(cx, |hub, cx| {
            let mut tail = hub.segments["viewport-meeting"].last().unwrap().clone();
            tail.text = "A newly received partial sentence".into();
            hub.apply_meeting_event(
                MeetingEvent::Segment {
                    meeting_id: "viewport-meeting".into(),
                    segment: tail,
                },
                cx,
            );
        });
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, _| {
        let after = w
            .meeting_transcript_view
            .borrow()
            .as_ref()
            .unwrap()
            .list
            .logical_scroll_top();
        assert_eq!(
            (before.item_ix, before.offset_in_item),
            (after.item_ix, after.offset_in_item)
        );
        assert!(w.transcript_rendered_rows.get() < 200);
    });
}

#[gpui::test]
fn native_transcript_keyboard_reaches_both_ends_without_mounting_history(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, 10_001, cx);
        w.active_view = AppView::MeetingDetail {
            meeting_id: "viewport-meeting".into(),
        };
        cx.notify();
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, window, cx| {
        w.transcript_section_focus(false).unwrap().focus(window, cx);
        w.transcript_rendered_rows.set(0);
    });
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-meeting-chat").is_some());
    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-transcript-row-0").is_some());
    workspace.update_in(cx, |w, _, _| {
        assert!(w.transcript_rendered_rows.get() < 200)
    });
}

#[gpui::test]
fn native_transcript_editor_stays_focused_offscreen_and_keeps_draft(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, 10_001, cx);
        w.active_view = AppView::MeetingDetail {
            meeting_id: "viewport-meeting".into(),
        };
        cx.notify();
    });
    cx.run_until_parked();
    let chip = cx
        .debug_bounds("desktop-speaker-chip-0")
        .expect("first speaker chip");
    cx.simulate_click(chip.center(), gpui::Modifiers::default());
    cx.simulate_keystrokes("cmd-a d r a f t");
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.speaker_rename_input.read(cx).value().as_str(),
            "draft",
            "typed keys must reach the inline editor"
        );
    });
    workspace.update_in(cx, |w, window, cx| {
        w.speaker_rename_input.update(cx, |input, cx| {
            input.set_value("Draft speaker 🗒", window, cx)
        });
        w.meeting_transcript_view
            .borrow()
            .as_ref()
            .unwrap()
            .list
            .scroll_to(ListOffset {
                item_ix: 5_000,
                offset_in_item: px(0.0),
            });
        w.transcript_rendered_rows.set(0);
        cx.notify();
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("home");
    workspace.update_in(cx, |w, window, cx| {
        assert_eq!(
            w.speaker_rename_input.read(cx).value().as_str(),
            "Draft speaker 🗒"
        );
        assert!(w
            .speaker_rename_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window));
        assert_eq!(
            w.meeting_transcript_view
                .borrow()
                .as_ref()
                .unwrap()
                .list
                .logical_scroll_top()
                .item_ix,
            5_000
        );
        assert!(
            w.transcript_rendered_rows.get() < 200,
            "pin only the editor, not the intervening history"
        );
    });
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, cx| {
            let meeting = hub.meeting("viewport-meeting").unwrap().clone();
            let mut records = hub.segments["viewport-meeting"].clone();
            let mut inserted = records[0].clone();
            inserted.id = "a-before".into();
            records.push(inserted);
            hub.install_meeting_test_snapshot(meeting, records, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-speaker-editor-1").is_some(),
        "editor follows its segment ID after snapshot insertion"
    );
    workspace.update_in(cx, |w, window, cx| {
        assert_eq!(
            w.speaker_rename_input.read(cx).value().as_str(),
            "Draft speaker 🗒"
        );
        assert!(w
            .speaker_rename_input
            .read(cx)
            .focus_handle(cx)
            .is_focused(window));
    });
    cx.simulate_keystrokes("escape");
    workspace.update_in(cx, |w, _, _| assert!(w.editing_speaker.is_none()));
}

#[gpui::test]
fn native_live_transcript_follows_tail_without_stealing_review_scroll(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, 10_001, cx);
        w.meetings_hub.update(cx, |hub, cx| {
            hub.live_meeting_id = Some("viewport-meeting".into());
            cx.notify();
        });
        w.transcript_rendered_rows.set(0);
    });
    cx.run_until_parked();
    assert!(cx
        .debug_bounds("desktop-live-transcript-row-10000")
        .is_some());
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.transcript_rendered_rows.get() < 200);
        let view = w.live_transcript_view.borrow();
        let list = &view.as_ref().unwrap().list;
        assert!(list.is_following_tail());
        list.scroll_to(ListOffset {
            item_ix: 500,
            offset_in_item: px(0.0),
        });
        cx.notify();
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, cx| {
            let mut next = hub.segments["viewport-meeting"].last().unwrap().clone();
            next.id = "appended-row".into();
            next.text = "New speech while reviewing history".into();
            hub.apply_meeting_event(
                MeetingEvent::Segment {
                    meeting_id: "viewport-meeting".into(),
                    segment: next,
                },
                cx,
            );
        });
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, _| {
        let view = w.live_transcript_view.borrow();
        let list = &view.as_ref().unwrap().list;
        assert!(!list.is_following_tail());
        assert_eq!(list.logical_scroll_top().item_ix, 500);
        assert_eq!(list.item_count(), 10_002);
    });
}
