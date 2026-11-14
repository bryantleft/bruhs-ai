//! Real fresh-thread command/action paths with explicit fake auth processes.
use super::{AppView, ChatView, NewThread, Workspace};
use crate::{app_state::DesktopState, theme};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use serde_json::json;
use std::{
    cell::RefCell, os::unix::fs::PermissionsExt as _, path::Path, rc::Rc, sync::mpsc::Receiver,
    time::Duration,
};

fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        ChatView::clear_auth_probe_for_test(cx);
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("isolated desktop state"));
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, visual) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow_mut().take().expect("workspace");
    visual.run_until_parked();
    (workspace, visual)
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn program(directory: &Path) -> String {
    let path = directory.join("fake-claude");
    std::fs::write(
        directory.join("response"),
        r#"{"loggedIn":true,"email":"fixture@example.invalid"}"#,
    )
    .expect("fixture response");
    release(directory);
    let body = format!(
        "#!/bin/sh\n\
         [ \"$1 $2\" = 'auth status' ] || exit 99\n\
         printf '.\\n' >> {calls}\n\
         while [ ! -f {release} ]; do /bin/sleep 0.01; done\n\
         /bin/cat {response}\n",
        calls = quote(&directory.join("calls")),
        release = quote(&directory.join("release")),
        response = quote(&directory.join("response")),
    );
    std::fs::write(&path, body).expect("fake auth executable");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("fixture executable permissions");
    path.to_str().expect("UTF-8 fixture path").to_string()
}

fn release(directory: &Path) {
    std::fs::write(directory.join("release"), []).expect("release fake status query");
}

fn count(directory: &Path) -> usize {
    std::fs::read_to_string(directory.join("calls"))
        .expect("auth query counter")
        .lines()
        .count()
}

fn finish(completed: &Receiver<()>, expected: usize, visual: &mut VisualTestContext) {
    for _ in 0..expected {
        completed
            .recv_timeout(Duration::from_secs(30))
            .expect("auth timeout plus equal scheduling margin");
    }
    visual.run_until_parked();
}

fn close(workspace: Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|window, _| window.remove_window());
    drop(workspace);
    visual.cx.update(|_| {});
    visual.run_until_parked();
    visual.cx.update(ChatView::clear_auth_probe_for_test);
}

#[gpui::test]
fn fresh_thread_actions_auth_process_counts(cx: &mut TestAppContext) {
    for action in ["palette", "dispatch"] {
        for model in ["claude-sonnet-4-6", "gpt-5.4"] {
            let (workspace, visual) = fixture(cx);
            let directory = tempfile::tempdir().unwrap();
            let completed = visual
                .cx
                .update(|cx| ChatView::install_auth_probe_for_test(cx, program(directory.path())));
            let before = workspace.read_with(visual, |workspace, _| workspace.threads.len());
            workspace.update_in(visual, |workspace, window, cx| {
                workspace.current_model = model.into();
                workspace.workspace_focus_handle.focus(window, cx);
                if action == "palette" {
                    workspace.execute_command_action("new-thread", window, cx)
                } else {
                    window.dispatch_action(Box::new(NewThread), cx)
                }
            });
            visual.run_until_parked();
            let expected = 1;
            finish(&completed, expected, visual);
            assert_eq!(count(directory.path()), expected);
            workspace.read_with(visual, |workspace, cx| {
                assert_eq!(workspace.threads.len(), before + 1);
                let thread = workspace.threads.last().unwrap();
                let chat = thread.chat.read(cx);
                assert_eq!(
                    workspace.active_thread_id.as_deref(),
                    Some(thread.id.as_str())
                );
                assert_eq!(workspace.active_view, AppView::Chat);
                assert_eq!(thread.model_id, model);
                assert_eq!(chat.model_id, model);
                assert!(chat.messages.is_empty());
                assert!(chat.prompt_queue.is_empty());
                assert!(chat.session_id.is_none());
                assert!(!chat.auth_banner_visible_for_test());
                let snapshot = workspace.snapshot(cx);
                let mut value = serde_json::to_value(snapshot.threads.last().unwrap()).unwrap();
                value["id"] = json!("created-thread");
                value["updated_at_ms"] = json!(0);
                println!(
                    "FRESH_THREAD_AUTH_RESULT {}",
                    json!({
                        "action": action,
                        "model": model,
                        "processes": expected,
                        "created_thread": value,
                    })
                );
            });
            close(workspace, visual);
        }
    }
}

#[gpui::test]
fn provider_changes_keep_prefetched_account_state_and_still_check_fresh(cx: &mut TestAppContext) {
    let (workspace, visual) = fixture(cx);
    let directory = tempfile::tempdir().unwrap();
    let completed = visual
        .cx
        .update(|cx| ChatView::install_auth_probe_for_test(cx, program(directory.path())));
    workspace.update_in(visual, |workspace, window, cx| {
        workspace.current_model = "gpt-5.4".into();
        workspace.execute_command_action("new-thread", window, cx);
    });
    finish(&completed, 1, visual);
    let chat = workspace.read_with(visual, |workspace, _| {
        workspace.threads.last().unwrap().chat.clone()
    });
    std::fs::remove_file(directory.path().join("release")).unwrap();
    std::fs::write(directory.path().join("response"), r#"{"loggedIn":false}"#).unwrap();
    chat.update(visual, |chat, cx| chat.set_model("claude-sonnet-4-6", cx));
    visual.run_until_parked();
    assert!(
        !chat.read_with(visual, |chat, _| chat.auth_banner_visible_for_test()),
        "keep the prefetched signed-in state until the fresh check answers"
    );
    release(directory.path());
    finish(&completed, 1, visual);
    assert_eq!(count(directory.path()), 2);
    assert!(
        chat.read_with(visual, |chat, _| chat.auth_banner_visible_for_test()),
        "a fresh signed-out result must replace the prefetched account"
    );
    drop(chat);
    close(workspace, visual);
}
