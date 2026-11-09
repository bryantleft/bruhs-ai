use super::super::{
    desktop_state_backup_path, load_desktop_state_from_path, AppView, DesktopState, SaveStatus,
    Workspace,
};
use super::tests::fixture;
use crate::{server_client::ServerClient, views::meetings_hub::ActionServer};
use gpui::{Entity, TestAppContext, VisualTestContext};

struct Files(std::path::PathBuf);
impl Files {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("bruhs-meeting-action-{}", uuid::Uuid::new_v4())))
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
}

async fn install(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    files: &Files,
) -> ActionServer {
    let server = workspace.update_in(cx, |w, _, cx| {
        let server = ActionServer::new(&cx.global::<DesktopState>().runtime);
        let meeting = server.state.lock().expect("fixture state").meeting.clone();
        server.state.lock().expect("fixture state").intent_file = Some(files.path());
        w.persistence_test_path = Some(files.path());
        w.meetings_hub.update(cx, |hub, cx| {
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.install_meeting_test_snapshot(meeting, vec![], cx);
            hub.prefs.personality.extract_observations = false;
        });
        w.active_view = AppView::MeetingDetail {
            meeting_id: "action-meeting".into(),
        };
        cx.notify();
        server
    });
    cx.run_until_parked();
    cx.condition(workspace, |w, _| w.save_status.is_saved())
        .await;
    server
}

async fn finished(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.condition(workspace, |w, cx| {
        w.save_status.is_saved()
            && w.meetings_hub
                .read(cx)
                .meeting("action-meeting")
                .is_some_and(|meeting| meeting.summary.is_some())
            && !w
                .meetings_hub
                .read(cx)
                .meeting_action_blocked("action-meeting")
    })
    .await;
}

#[gpui::test]
async fn native_meeting_action_failed_intent_save_sends_nothing_then_real_retry_saves_once(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let (workspace, cx) = fixture(cx);
    let server = install(&workspace, cx, &files).await;
    let before = std::fs::read(files.path()).unwrap();
    let backup = desktop_state_backup_path(&files.path());
    if backup.is_file() {
        std::fs::remove_file(&backup).unwrap();
    }
    std::fs::create_dir(&backup).unwrap();
    click(cx, "desktop-meeting-resummarize");
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(server.state.lock().unwrap().posts, 0);
    assert_eq!(std::fs::read(files.path()).unwrap(), before);
    workspace.update_in(cx, |w, _, cx| {
        let status = w
            .meetings_hub
            .read(cx)
            .meeting_action_status("action-meeting")
            .unwrap();
        assert!(status.busy && status.message.contains("not sent"));
    });
    std::fs::remove_dir(&backup).unwrap();
    click(cx, "desktop-retry-save");
    finished(&workspace, cx).await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(saved.meeting_actions).unwrap(),
        serde_json::json!({})
    );
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_meetings(saved.meeting_cache);
            assert_eq!(
                hub.meeting("action-meeting")
                    .unwrap()
                    .summary
                    .as_ref()
                    .unwrap()
                    .tldr,
                "Generated from fixture transcript 🗒"
            );
        })
    });
    let state = server.state.lock().unwrap();
    assert!(state.observed_intent);
    assert_eq!((state.posts, state.patches), (1, 0));
    drop(state);
}

#[gpui::test]
async fn native_meeting_action_result_save_failure_restores_get_only_recovery_through_real_controls(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let (workspace, cx) = fixture(cx);
    let server = install(&workspace, cx, &files).await;
    server.state.lock().unwrap().fail_result_save = true;
    click(cx, "desktop-meeting-resummarize");
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    let saved = load_desktop_state_from_path(&files.path())
        .unwrap()
        .unwrap();
    assert!(serde_json::to_value(&saved.meeting_actions).unwrap()["action-meeting"].is_object());
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            assert!(hub.meeting("action-meeting").unwrap().summary.is_some());
            assert!(hub.meeting_action_blocked("action-meeting"));
            // Exercise the same cache/checkpoint restore boundary as startup, without replay.
            hub.restore_meetings(saved.meeting_cache);
            hub.restore_action_checkpoints(saved.meeting_actions);
            assert!(hub.meeting("action-meeting").unwrap().summary.is_none());
        })
    });
    std::fs::remove_dir(desktop_state_backup_path(&files.path())).unwrap();
    workspace.update_in(cx, |_, _, cx| cx.notify());
    click(cx, "desktop-meeting-load-latest");
    finished(&workspace, cx).await;
    assert_eq!(
        serde_json::to_value(
            load_desktop_state_from_path(&files.path())
                .unwrap()
                .unwrap()
                .meeting_actions
        )
        .unwrap(),
        serde_json::json!({})
    );
    let state = server.state.lock().unwrap();
    assert!(state.observed_intent);
    assert_eq!((state.posts, state.patches, state.gets), (1, 0, 2));
    drop(state);
}

#[gpui::test]
async fn native_meeting_action_keyboard_activation_and_navigation_keep_one_owned_request(
    cx: &mut TestAppContext,
) {
    let files = Files::new();
    let (workspace, cx) = fixture(cx);
    let server = install(&workspace, cx, &files).await;
    server.state.lock().unwrap().delay_ms = 200;
    cx.run_until_parked();
    let bounds = cx.debug_bounds("desktop-meeting-resummarize").unwrap();
    assert!(bounds.size.height >= gpui::px(44.0));
    // Focus without completing a click; Enter must be the activation.
    cx.simulate_mouse_down(
        bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        gpui::point(gpui::px(1.0), gpui::px(1.0)),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    workspace.update_in(cx, |w, _, cx| {
        assert!(!w
            .meetings_hub
            .read(cx)
            .meeting_action_blocked("action-meeting"))
    });
    cx.simulate_keystrokes("enter");
    workspace.update_in(cx, |w, _, cx| {
        assert!(w
            .meetings_hub
            .read(cx)
            .meeting_action_blocked("action-meeting"))
    });
    cx.simulate_keystrokes("enter");
    click(cx, "desktop-meeting-back");
    workspace.update_in(cx, |w, _, _| {
        assert!(matches!(w.active_view, AppView::Meetings))
    });
    finished(&workspace, cx).await;
    let state = server.state.lock().unwrap();
    assert_eq!((state.posts, state.patches), (1, 0));
    assert!(state.observed_intent);
    drop(state);
}
