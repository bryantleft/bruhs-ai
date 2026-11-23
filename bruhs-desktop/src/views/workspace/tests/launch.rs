//! The app as a launch builds it, for tests that restart it: a workspace
//! read from a state file and saving back to it, in a window of its own.
//! Its hub talks to a closed loopback port (9), never a bruhs-server
//! running on this machine.

use super::*;
use crate::views::workspace::RestoredRecords;
use std::path::Path;

pub(in crate::views::workspace) fn init(cx: &mut gpui::TestAppContext) {
    cx.executor().allow_parking();
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
}

/// The workspace read from `path` (a new one when there is no file yet),
/// saving back to it. Call [`init`] once per test first; launch a second
/// time from a clone of the test's `TestAppContext`.
pub(in crate::views::workspace) fn launch<'a>(
    cx: &'a mut gpui::TestAppContext,
    path: &Path,
) -> (Entity<Workspace>, &'a mut gpui::VisualTestContext) {
    let loaded = load_desktop_state_with_backup(path);
    let cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let captured = cell.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::from_loaded_state(
                loaded,
                window,
                cx,
                RestoredRecords::take_from_persisted,
                Workspace::restore_threads,
            )
        });
        *captured.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("workspace entity");
    workspace.update_in(cx, |workspace, _window, cx| {
        workspace.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                crate::server_client::ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9")
                    .expect("a loopback client"),
            );
        });
        workspace.persistence_test_path = Some(path.to_path_buf());
        cx.notify();
    });
    cx.simulate_resize(gpui::size(px(1280.0), px(800.0)));
    cx.run_until_parked();
    (workspace, cx)
}

/// The window stops saving, as the app does when it quits.
pub(in crate::views::workspace) fn quit(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
) {
    workspace.update_in(cx, |workspace, _window, _cx| {
        assert!(workspace.save_status.is_saved(), "quit after the last save");
        workspace.persistence_test_path = None;
    });
}

/// Waits for the last change to reach the file.
pub(in crate::views::workspace) async fn saved(
    workspace: &Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
) {
    cx.condition(workspace, |workspace, _| workspace.save_status.is_saved())
        .await;
    cx.run_until_parked();
}
