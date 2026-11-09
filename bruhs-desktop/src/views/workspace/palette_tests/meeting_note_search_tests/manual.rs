use super::*;
use crate::views::workspace::{load_desktop_state_from_path, SaveStatus};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-manual-note-{}", uuid::Uuid::new_v4())))
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

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("Missing {selector}"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn set_content(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, title: &str, body: &str) {
    workspace.update_in(cx, |w, window, cx| {
        let editor = w.note_editor.as_ref().unwrap();
        editor
            .title
            .update(cx, |input, cx| input.set_value(title, window, cx));
        editor
            .body
            .update(cx, |input, cx| input.set_value(body, window, cx));
    });
}

#[gpui::test]
async fn manual_note_create_type_save_and_edit_survive_real_file_readback(cx: &mut TestAppContext) {
    let (workspace, cx) = local_workspace(cx);
    let files = Files::new();
    workspace.update_in(cx, |w, _, cx| {
        w.persistence_test_path = Some(files.path());
        w.active_view = AppView::Meetings;
        w.meeting_notes.clear();
        cx.notify();
    });
    click(cx, "desktop-new-meeting-note");
    click(cx, "desktop-save-meeting-note");
    workspace.update_in(cx, |w, _, _| assert!(w.meeting_notes.is_empty()));
    workspace.update_in(cx, |w, window, cx| {
        w.note_editor
            .as_ref()
            .unwrap()
            .title
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.simulate_input("Manual planning");
    workspace.update_in(cx, |w, window, cx| {
        w.note_editor
            .as_ref()
            .unwrap()
            .body
            .update(cx, |input, cx| input.focus(window, cx));
    });
    cx.simulate_input("exact notequartz");
    click(cx, "desktop-save-meeting-note");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let first = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap()
        .meeting_notes[0]
        .clone();
    assert_eq!(first.title, "Manual planning");
    assert_eq!(first.body, "exact notequartz");
    set_content(&workspace, cx, "Edited planning", "  Exact\n\n🗒  ");
    click(cx, "desktop-save-meeting-note");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap()
        .meeting_notes;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, first.id);
    assert_eq!(saved[0].created_at, first.created_at);
    assert_eq!(saved[0].body, "  Exact\n\n🗒  ");
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.universal_search("notequartz", cx).1.is_empty());
        assert_eq!(
            w.universal_search("edited planning", cx).1[0].document.id,
            first.id
        );
    });
}

#[gpui::test]
fn manual_note_draft_survives_navigation_and_refuses_stale_original(cx: &mut TestAppContext) {
    let (workspace, cx) = local_workspace(cx);
    workspace.update_in(cx, |w, window, cx| {
        let note = MeetingNote::new("Original".into(), "Body".into(), 1).unwrap();
        w.meeting_notes = vec![note.clone()];
        w.open_local_meeting_note(note.id.clone(), window, cx);
        w.begin_note_edit(Some(note), window, cx);
    });
    set_content(&workspace, cx, "My unsaved draft", "Private draft");
    workspace.update_in(cx, |w, window, cx| {
        w.active_view = AppView::Tasks;
        w.active_view = AppView::Meetings;
        w.begin_note_edit(None, window, cx);
        assert_eq!(
            w.note_editor
                .as_ref()
                .unwrap()
                .title
                .read(cx)
                .value()
                .as_str(),
            "My unsaved draft"
        );
        w.meeting_notes[0].body = "Changed elsewhere".into();
        w.save_note_edit(cx);
        assert_eq!(w.meeting_notes[0].body, "Changed elsewhere");
        assert_eq!(
            w.note_editor
                .as_ref()
                .unwrap()
                .body
                .read(cx)
                .value()
                .as_str(),
            "Private draft"
        );
    });
    click(cx, "desktop-close-note-editor");
    workspace.update_in(cx, |w, _, _| assert!(w.note_editor.is_none()));
}

#[gpui::test]
async fn manual_note_failed_save_retains_original_file_and_retry_saves_same_identity(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = local_workspace(cx);
    let files = Files::new();
    workspace.update_in(cx, |w, window, cx| {
        w.persistence_test_path = Some(files.path());
        w.active_view = AppView::Meetings;
        w.meeting_notes.clear();
        w.begin_note_edit(None, window, cx);
    });
    set_content(&workspace, cx, "First", "Before failure");
    workspace.update_in(cx, |w, _, cx| w.save_note_edit(cx));
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let before = std::fs::read(files.path()).unwrap();
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(files.path().join("blocked.json"))
    });
    set_content(&workspace, cx, "Second", "Retained after failure");
    workspace.update_in(cx, |w, _, cx| w.save_note_edit(cx));
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    let id = workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.meeting_notes.len(), 1);
        assert_eq!(
            w.note_editor
                .as_ref()
                .unwrap()
                .body
                .read(cx)
                .value()
                .as_str(),
            "Retained after failure"
        );
        w.persistence_test_path = Some(files.path());
        w.meeting_notes[0].id.clone()
    });
    click(cx, "desktop-retry-save");
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(saved.meeting_notes[0].id, id);
    assert_eq!(saved.meeting_notes[0].body, "Retained after failure");
}

#[gpui::test]
fn linked_note_creation_navigation_and_missing_recording_preserve_note(cx: &mut TestAppContext) {
    // Opening the recording uses the real Tokio read adapter (loopback port 9).
    cx.executor().allow_parking();
    let (workspace, cx) = local_workspace(cx);
    workspace.update_in(cx, |w, window, cx| {
        let mut meeting = bruhs_models::Meeting::new("Recording", 1.);
        meeting.id = "recording-1".into();
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_meetings(
                serde_json::from_value(serde_json::json!({
                    "records": {"recording-1":meeting}, "transcripts": {"recording-1":[]}
                }))
                .unwrap(),
            )
        });
        w.begin_linked_note("recording-1", window, cx);
    });
    set_content(&workspace, cx, "Linked note", "Independent text");
    let note = workspace.update_in(cx, |w, window, cx| {
        w.save_note_edit(cx);
        let note = w.meeting_notes[0].clone();
        assert_eq!(note.recording_id.as_deref(), Some("recording-1"));
        w.note_editor = None;
        w.open_local_meeting_note(note.id.clone(), window, cx);
        note
    });
    click(cx, "desktop-open-linked-recording");
    workspace.update_in(cx, |w, window, cx| {
        assert_eq!(
            w.active_view,
            AppView::MeetingDetail {
                meeting_id: "recording-1".into()
            }
        );
        w.meetings_hub.update(cx, |hub, _| hub.meetings.clear());
        w.open_local_meeting_note(note.id.clone(), window, cx);
    });
    click(cx, "desktop-open-linked-recording");
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(
            w.active_view,
            AppView::MeetingNoteDetail {
                note_id: note.id.clone()
            }
        );
        assert_eq!(w.meeting_notes[0], note);
    });
}

#[gpui::test]
fn manual_note_command_opens_editor_from_palette_keyboard(cx: &mut TestAppContext) {
    let (workspace, cx) = local_workspace(cx);
    workspace.update_in(cx, |w, window, cx| {
        w.meeting_notes.clear();
        w.focus_command_bar(&FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_input("new meeting note");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-meeting-note-editor").is_some());
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.active_view, AppView::Meetings);
        assert!(w.note_editor.is_some());
        assert!(!w.command_bar_visible());
        assert!(w.meeting_notes.is_empty());
    });
    cx.simulate_input("Retain my title");
    workspace.update_in(cx, |w, window, cx| {
        w.focus_command_bar(&FocusCommandBar, window, cx)
    });
    cx.run_until_parked();
    cx.simulate_input("new meeting note");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(
            w.note_editor
                .as_ref()
                .unwrap()
                .title
                .read(cx)
                .value()
                .as_str(),
            "Retain my title"
        );
        assert!(w.meeting_notes.is_empty());
    });
}
