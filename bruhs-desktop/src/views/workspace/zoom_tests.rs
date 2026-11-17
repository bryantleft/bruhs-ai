//! PM-57: the desktop zooms like a browser zooms the web app. ⌘+ / ⌘− / ⌘0
//! step through Chrome's levels, the level survives a restart, design lengths
//! (and the rem size) scale with it, and a zoomed window switches layouts at
//! the web's widths in CSS pixels.

use super::{
    AppView, DiskVersion, PersistedWorkspaceState, ResetZoom, RestoredRecords, Workspace, ZoomIn,
    ZoomOut,
};
use crate::{app_state::DesktopState, theme};
use gpui::{size, AppContext, Entity, TestAppContext, VisualTestContext};
use gpui_component::zoom::{window_px, zoom};
use std::{cell::RefCell, rc::Rc};

fn open(
    cx: &mut TestAppContext,
    state: Option<PersistedWorkspaceState>,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let entity = Rc::new(RefCell::new(None));
    let captured = entity.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| {
            Workspace::from_loaded_state(
                (state, None, DiskVersion::Missing),
                window,
                cx,
                RestoredRecords::take_from_persisted,
                Workspace::restore_threads,
            )
        });
        *captured.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
    });
    let workspace = entity.borrow().clone().expect("workspace");
    cx.run_until_parked();
    (workspace, cx)
}

fn saved_zoom(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<f32> {
    workspace.read_with(cx, |workspace, cx| workspace.snapshot(cx).zoom)
}

/// ⌘+ / ⌘− / ⌘0 step through Chrome's levels and save the level, leaving
/// 100% out of the state file.
#[gpui::test]
fn zoom_actions_step_and_save_like_browser_zoom(cx: &mut TestAppContext) {
    let (workspace, cx) = open(cx, None);
    cx.simulate_resize(size(window_px(1280.0), window_px(800.0)));
    // Actions go to the focused workspace, as the shortcuts and the View
    // menu do in the app.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.workspace_focus_handle.focus(window, cx);
    });
    assert_eq!(zoom(), 1.0);
    assert_eq!(saved_zoom(&workspace, cx), None);

    cx.dispatch_action(ZoomIn);
    cx.run_until_parked();
    assert_eq!(zoom(), 1.1);
    assert_eq!(saved_zoom(&workspace, cx), Some(1.1));
    cx.dispatch_action(ZoomIn);
    assert_eq!(zoom(), 1.25);

    cx.dispatch_action(ZoomOut);
    cx.dispatch_action(ZoomOut);
    cx.dispatch_action(ZoomOut);
    assert_eq!(zoom(), 0.9);

    cx.dispatch_action(ResetZoom);
    cx.run_until_parked();
    assert_eq!(zoom(), 1.0);
    assert_eq!(saved_zoom(&workspace, cx), None);
}

/// At 200% a 1280x800 window shows the page the web shows at 200% browser
/// zoom: design lengths are twice as long, rem-sized ones too, and the
/// layout is the web's 640px one, so the sidebar stays docked (web `sm`) and
/// the page fits. At 250% the page is 512 CSS pixels wide and the sidebar
/// becomes a drawer. Back at 100%, every length is what it was.
#[gpui::test]
fn two_hundred_percent_reflows_at_the_web_widths(cx: &mut TestAppContext) {
    let (workspace, cx) = open(cx, None);
    cx.simulate_resize(size(window_px(1280.0), window_px(800.0)));
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.active_view = AppView::Today;
        workspace.sidebar_visible = true;
        cx.notify();
    });
    cx.run_until_parked();
    let sidebar = cx
        .debug_bounds("desktop-workspace-sidebar")
        .expect("docked sidebar");
    let rem = cx.update(|window, _| window.rem_size());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.set_zoom(2.0, window, cx)
    });
    cx.run_until_parked();
    let zoomed = cx
        .debug_bounds("desktop-workspace-sidebar")
        .expect("docked at 640 CSS pixels");
    assert!(
        (f32::from(zoomed.size.width) - 2.0 * f32::from(sidebar.size.width)).abs() < 1.0,
        "{sidebar:?} -> {zoomed:?}"
    );
    cx.update(|window, _| assert_eq!(window.rem_size(), rem * 2.0));
    let title = cx
        .debug_bounds("desktop-page-title")
        .expect("Today's title");
    assert!(
        title.size.width >= window_px(160.0) && title.right() <= window_px(1280.0),
        "{title:?}"
    );
    let plan = cx.debug_bounds("desktop-today-plan").expect("Plan my day");
    assert!(plan.right() <= window_px(1280.0), "{plan:?}");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.set_zoom(2.5, window, cx)
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("desktop-workspace-sidebar").is_none(),
        "a drawer below 640 CSS pixels"
    );

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.set_zoom(1.0, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("desktop-workspace-sidebar")
            .map(|bounds| bounds.size.width),
        Some(sidebar.size.width)
    );
    cx.update(|window, _| assert_eq!(window.rem_size(), rem));
}

/// The zoom survives a restart; a level outside the steps does not.
#[gpui::test]
fn a_restart_keeps_the_zoom(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState =
        serde_json::from_value(serde_json::json!({ "zoom": 1.5 })).expect("state");
    let (_workspace, _cx) = open(cx, Some(state));
    assert_eq!(zoom(), 1.5);
}

#[gpui::test]
fn a_saved_zoom_outside_the_steps_opens_at_100(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState =
        serde_json::from_value(serde_json::json!({ "zoom": 9.0 })).expect("state");
    let (_workspace, _cx) = open(cx, Some(state));
    assert_eq!(zoom(), 1.0);
}

/// The macOS traffic lights stay in window points, so the room left for them
/// does not grow with the page.
#[cfg(target_os = "macos")]
#[gpui::test]
fn the_traffic_light_room_stays_the_same_on_screen(cx: &mut TestAppContext) {
    let (workspace, cx) = open(cx, None);
    let before = workspace.read_with(cx, |workspace, _| workspace.traffic_light_inset());
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.set_zoom(2.0, window, cx)
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.traffic_light_inset(), before);
    });
}
