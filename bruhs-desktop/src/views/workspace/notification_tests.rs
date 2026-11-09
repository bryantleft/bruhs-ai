use super::Workspace;
use crate::{app_state::DesktopState, theme, views::meetings_hub::HubNotice};
use bruhs_models::AppView;
use gpui::{px, AppContext, Entity, Modifiers, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
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
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.active_view = AppView::Search;
        cx.notify();
    });
    (workspace, cx)
}

fn show_notice(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, message: String) {
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.meetings_hub.update(cx, |hub, cx| {
            hub.notice = Some(HubNotice {
                message,
                warning: true,
            });
            cx.notify();
        });
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn native_notifications_reserve_bounded_space_without_covering_search(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    for width in [320.0, 390.0, 1280.0] {
        cx.simulate_resize(gpui::size(px(width), px(844.0)));
        cx.run_until_parked();
        let before = cx.debug_bounds("desktop-search-page").expect("search page");
        show_notice(&workspace, cx, "Synthetic capture warning. ".repeat(100));
        let search = cx.debug_bounds("desktop-search-page").expect("search page");
        let rail = cx
            .debug_bounds("desktop-notification-rail")
            .expect("notice rail");
        let notice = cx.debug_bounds("desktop-hub-notice").expect("notice");
        assert_eq!(before.origin.y, search.origin.y, "notice moved the header");
        assert!(
            search.bottom() <= rail.origin.y,
            "notice covers page: {search:?} {rail:?}"
        );
        assert!(
            rail.size.height <= px(844.0 * 0.3 + 1.0),
            "unbounded notice rail: {rail:?}"
        );
        assert!(rail.bottom() <= px(844.0));
        assert!(notice.origin.x >= px(0.0) && notice.right() <= px(width));
        workspace.update_in(cx, |workspace, _, cx| {
            workspace
                .meetings_hub
                .update(cx, |hub, cx| hub.dismiss_notice(cx));
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("desktop-notification-rail").is_none());
    }
}

#[gpui::test]
fn native_notification_dismiss_supports_pointer_and_keyboard_and_restores_focus(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    for activation in ["pointer", "enter", "space"] {
        show_notice(&workspace, cx, "Synthetic capture warning".into());
        let dismiss = cx
            .debug_bounds("desktop-hub-notice-dismiss")
            .expect("dismiss control");
        assert!(dismiss.size.width >= px(44.0) && dismiss.size.height >= px(44.0));
        if activation == "pointer" {
            cx.simulate_click(dismiss.center(), Modifiers::default());
        } else {
            // Focus without completing a click, so only the key can dismiss.
            cx.simulate_mouse_down(
                dismiss.center(),
                gpui::MouseButton::Left,
                Modifiers::default(),
            );
            cx.simulate_mouse_up(
                gpui::point(px(1.0), px(1.0)),
                gpui::MouseButton::Left,
                Modifiers::default(),
            );
            workspace.update_in(cx, |workspace, _, cx| {
                assert!(workspace.meetings_hub.read(cx).notice.is_some());
            });
            cx.simulate_keystrokes(activation);
        }
        cx.run_until_parked();
        workspace.update_in(cx, |workspace, window, cx| {
            assert!(
                workspace.meetings_hub.read(cx).notice.is_none(),
                "{activation}"
            );
            assert!(
                workspace.workspace_focus_handle.is_focused(window),
                "{activation}"
            );
        });
        assert!(cx.debug_bounds("desktop-hub-notice").is_none());
    }
}
