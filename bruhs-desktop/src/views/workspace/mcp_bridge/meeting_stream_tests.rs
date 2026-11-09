use super::super::{
    desktop_state_backup_path, load_desktop_state_from_path, SaveStatus, Workspace,
};
use super::tests::fixture;
use bruhs_models::{Meeting, MeetingEvent, TranscriptSegment};
use gpui::{Entity, TestAppContext, VisualTestContext};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-live-transcript-{}", uuid::Uuid::new_v4())))
    }
    fn path(&self) -> std::path::PathBuf {
        self.0.join("state.json")
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn segment(text: &str, partial: bool) -> TranscriptSegment {
    TranscriptSegment {
        id: "live-fragment".into(),
        meeting_id: "live-meeting".into(),
        speaker_id: None,
        start_ms: 0,
        end_ms: 10,
        text: text.into(),
        confidence: 0.9,
        partial,
        speaker_confidence: None,
    }
}

async fn install(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, files: &Files) {
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(files.path());
        let mut meeting = Meeting::new("Owned streaming fixture", 1.0);
        meeting.id = "live-meeting".into();
        w.meetings_hub.update(cx, |hub, cx| {
            hub.install_meeting_test_snapshot(meeting, vec![], cx)
        });
    });
    cx.run_until_parked();
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
}

fn emit(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, record: TranscriptSegment) {
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, cx| {
            hub.apply_meeting_event(
                MeetingEvent::Segment {
                    meeting_id: "live-meeting".into(),
                    segment: record,
                },
                cx,
            );
        })
    });
    cx.run_until_parked();
}

#[gpui::test]
async fn native_stream_checkpoint_saves_during_continuous_input_and_restores_exact_received_words(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let (workspace, cx) = fixture(cx);
    install(&workspace, cx, &files).await;
    let before = std::fs::read(files.path()).unwrap();
    let mut saved_while_streaming = false;
    for index in 0..20 {
        emit(
            &workspace,
            cx,
            segment(&format!("Live words {index} 🗒"), true),
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        if index < 19 && std::fs::read(files.path()).unwrap() != before {
            saved_while_streaming = true;
        }
    }
    assert!(
        saved_while_streaming,
        "150ms saves must not wait for a 50ms stream to become idle"
    );
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.segments.clear();
            hub.restore_meetings(saved.meeting_cache);
            assert_eq!(
                hub.segments["live-meeting"],
                vec![segment("Live words 19 🗒", true)]
            );
            assert!(hub.live_meeting_id.is_none());
            assert!(hub.capture_status.recording().is_none());
        })
    });
    emit(&workspace, cx, segment("Final words 🗒", false));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_meetings(saved.meeting_cache);
            assert_eq!(
                hub.segments["live-meeting"],
                vec![segment("Final words 🗒", false)]
            );
        })
    });
}

#[gpui::test]
async fn native_stream_save_failure_keeps_received_speech_and_real_retry_acknowledges_latest_fragment(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let (workspace, cx) = fixture(cx);
    install(&workspace, cx, &files).await;
    let before = std::fs::read(files.path()).unwrap();
    let backup = desktop_state_backup_path(&files.path());
    if backup.is_file() {
        std::fs::remove_file(&backup).unwrap();
    }
    std::fs::create_dir(&backup).unwrap();
    emit(&workspace, cx, segment("Unsaved draft", true));
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    emit(&workspace, cx, segment("Received final speech", false));
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.meetings_hub.read(cx).segments["live-meeting"],
            vec![segment("Received final speech", false)]
        )
    });
    std::fs::remove_dir(&backup).unwrap();
    cx.run_until_parked();
    let bounds = cx.debug_bounds("desktop-retry-save").unwrap();
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_meetings(saved.meeting_cache);
            assert_eq!(
                hub.segments["live-meeting"],
                vec![segment("Received final speech", false)]
            );
        })
    });
}
