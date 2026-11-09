//! The project picker reads the disk off the UI thread.

use super::Workspace;
use crate::{app_state::DesktopState, theme};
use gpui::{AppContext, TestAppContext};
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
fn listing_a_directory_does_not_block_the_ui_thread(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().expect("workspace");
    cx.run_until_parked();
    let home = std::fs::canonicalize(std::env::var_os("HOME").expect("HOME in tests"))
        .expect("home resolves");

    workspace.update(cx, |workspace, cx| {
        workspace.project_picker_current_path = "unchanged".into();
        workspace.load_project_directory(Some(home.to_string_lossy().into_owned()), cx);
        // Nothing was read synchronously.
        assert_eq!(workspace.project_picker_current_path, "unchanged");
        assert!(workspace.project_picker_task.is_some());
    });
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.project_picker_current_path,
            home.to_string_lossy()
        );
        assert!(workspace.project_picker_error.is_none());
    });
}
