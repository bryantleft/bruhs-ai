//! PM-57: the desktop zooms like a browser zooms the web app. ⌘+ / ⌘− / ⌘0
//! step through Chrome's levels, the level survives a restart, and a zoomed
//! window lays its pages out at the web's widths for the smaller viewport.

use super::{
    AppView, DiskVersion, PersistedWorkspaceState, ResetZoom, RestoredRecords, Workspace, ZoomIn,
    ZoomOut,
};
use crate::{app_state::DesktopState, theme};
use gpui::{px, size, AppContext, Entity, TestAppContext, VisualTestContext};
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

fn window_zoom(cx: &mut VisualTestContext) -> f32 {
    cx.update(|window, _| window.zoom())
}

fn saved_zoom(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<f32> {
    workspace.read_with(cx, |workspace, cx| workspace.snapshot(cx).zoom)
}

/// ⌘+ / ⌘− / ⌘0 step through Chrome's levels and save the level, leaving
/// 100% out of the state file.
#[gpui::test]
fn zoom_actions_step_and_save_like_browser_zoom(cx: &mut TestAppContext) {
    let (workspace, cx) = open(cx, None);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    // Actions go to the focused workspace, as the shortcuts and the View
    // menu do in the app.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.workspace_focus_handle.focus(window, cx);
    });
    assert_eq!(window_zoom(cx), 1.0);
    assert_eq!(saved_zoom(&workspace, cx), None);

    cx.dispatch_action(ZoomIn);
    cx.run_until_parked();
    assert_eq!(window_zoom(cx), 1.1);
    assert_eq!(saved_zoom(&workspace, cx), Some(1.1));
    cx.dispatch_action(ZoomIn);
    assert_eq!(window_zoom(cx), 1.25);

    cx.dispatch_action(ZoomOut);
    cx.dispatch_action(ZoomOut);
    cx.dispatch_action(ZoomOut);
    assert_eq!(window_zoom(cx), 0.9);

    cx.dispatch_action(ResetZoom);
    cx.run_until_parked();
    assert_eq!(window_zoom(cx), 1.0);
    assert_eq!(saved_zoom(&workspace, cx), None);
}

/// At 200% a 1280x800 window lays out a 640x400 page, as the web does at
/// 200% browser zoom: the sidebar stays docked (web `sm`) and the page
/// headers wrap instead of clipping. At 250% the page is 512 wide and the
/// sidebar becomes a drawer.
#[gpui::test]
fn two_hundred_percent_reflows_at_the_web_widths(cx: &mut TestAppContext) {
    let (workspace, cx) = open(cx, None);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_view = AppView::Today;
        workspace.sidebar_visible = true;
        workspace.set_zoom(2.0, window, cx);
    });
    cx.run_until_parked();
    cx.update(|window, _| assert_eq!(window.viewport_size(), size(px(640.0), px(400.0))));
    assert!(
        cx.debug_bounds("desktop-workspace-sidebar").is_some(),
        "docked at 640"
    );
    let title = cx
        .debug_bounds("desktop-page-title")
        .expect("Today's title");
    assert!(
        title.size.width >= px(80.0) && title.right() <= px(640.0),
        "{title:?}"
    );
    let plan = cx.debug_bounds("desktop-today-plan").expect("Plan my day");
    assert!(plan.right() <= px(640.0), "{plan:?}");

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.set_zoom(2.5, window, cx)
    });
    cx.run_until_parked();
    cx.update(|window, _| assert_eq!(window.viewport_size().width, px(512.0)));
    assert!(
        cx.debug_bounds("desktop-workspace-sidebar").is_none(),
        "a drawer below 640"
    );
}

/// The zoom survives a restart; a level outside the steps does not.
#[gpui::test]
fn a_restart_keeps_the_zoom(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState =
        serde_json::from_value(serde_json::json!({ "zoom": 1.5 })).expect("state");
    let (_workspace, cx) = open(cx, Some(state));
    assert_eq!(window_zoom(cx), 1.5);
}

#[gpui::test]
fn a_saved_zoom_outside_the_steps_opens_at_100(cx: &mut TestAppContext) {
    let state: PersistedWorkspaceState =
        serde_json::from_value(serde_json::json!({ "zoom": 9.0 })).expect("state");
    let (_workspace, cx) = open(cx, Some(state));
    assert_eq!(window_zoom(cx), 1.0);
}

/// The macOS traffic lights stay in window points, so the room left for
/// them in the content shrinks as the content zooms.
#[cfg(target_os = "macos")]
#[gpui::test]
fn the_traffic_light_room_stays_the_same_on_screen(cx: &mut TestAppContext) {
    let (workspace, cx) = open(cx, None);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.set_zoom(2.0, window, cx)
    });
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.traffic_light_inset_px(), 44.0);
    });
}
