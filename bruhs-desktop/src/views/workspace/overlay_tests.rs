//! One modal dialog at a time, and Escape's "was anything open" agrees with
//! what Escape closes.

use super::{date_picker_ui::DatePickerTarget, Overlay, TopLayer, Workspace};
use crate::{app_state::DesktopState, theme};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
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
    (workspace, cx)
}

#[gpui::test]
fn opening_a_dialog_replaces_the_one_showing(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_task_create(window, cx);
        workspace.toggle_task_create_menu(super::TaskCreateMenu::Status, cx);
        assert_eq!(
            workspace.overlay,
            Some(Overlay::TaskCreate {
                menu: Some(super::TaskCreateMenu::Status)
            })
        );
        workspace.open_settings(&super::OpenSettings, window, cx);
        // The task dialog and its menu went with it; nothing is stacked.
        assert_eq!(
            workspace.overlay,
            Some(Overlay::Settings {
                open_dropdown: None
            })
        );
        assert!(!workspace.task_create_visible());
        assert_eq!(workspace.task_create_menu_open(), None);
        workspace.close_transient_surfaces();
        assert_eq!(workspace.overlay, None);
        assert!(!workspace.overlay_open());
    });
}

#[gpui::test]
fn an_open_date_picker_counts_as_an_overlay(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.toggle_date_picker(DatePickerTarget::TaskDetailDue, window, cx);
        assert!(workspace.date_picker.is_some());
        // Escape closes it, so Escape must not treat the screen as clear.
        assert!(workspace.overlay_open());
        workspace.close_transient_surfaces();
        assert!(workspace.date_picker.is_none());
        assert!(!workspace.overlay_open());
    });
}

/// Web mounts the command bar and project picker over everything: ⌘K over
/// an open form leaves the form open behind it, and Escape closes both.
#[gpui::test]
fn the_command_bar_opens_over_a_form_and_escape_closes_both(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_task_create(window, cx);
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
        assert!(workspace.task_create_visible());
        assert_eq!(workspace.top_layer, Some(TopLayer::CommandBar));
        // ⌘K again closes only the bar.
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
        assert!(workspace.task_create_visible());
        assert_eq!(workspace.top_layer, None);
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
        workspace.close_overlays(&super::CloseOverlays, window, cx);
        assert_eq!(workspace.overlay, None);
        assert_eq!(workspace.top_layer, None);
        assert!(!workspace.overlay_open());
    });
}

#[gpui::test]
fn the_project_picker_opens_over_a_form_and_escape_closes_both(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_event_create(None, window, cx);
        workspace.open_project_picker(&super::OpenProjectPicker, window, cx);
        assert!(workspace.event_create_visible());
        assert!(workspace.project_picker_visible());
        assert!(workspace.overlay_open());
        workspace.close_overlays(&super::CloseOverlays, window, cx);
        assert!(!workspace.event_create_visible());
        assert!(!workspace.project_picker_visible());
    });
}

/// Both layers draw: the form under the bar.
#[gpui::test]
fn both_layers_render(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_task_create(window, cx);
        workspace.focus_command_bar(&super::FocusCommandBar, window, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-command-bar").is_some());
    assert!(cx.debug_bounds("desktop-task-create-dialog").is_some());
}
