//! Workspace UI fixture shared by portable view tests and Unix IPC tests.
use super::super::{AppView, AutomationState, DesktopState, Workspace};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

pub(super) fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    // Some callers cross real Tokio IO; they cannot use only the deterministic
    // executor's virtual clock. The fixture itself does not open an IPC socket.
    cx.executor().allow_parking();
    cx.update(|cx| {
        bruhs_ui::init(cx);
        crate::theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let cell = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("workspace entity");
    workspace.update_in(cx, |workspace, _, cx| {
        workspace.work_items.clear();
        workspace.calendar_events.clear();
        workspace.captures.clear();
        workspace.knowledge_documents.clear();
        workspace.inbox_conversations.clear();
        workspace.automation_state = AutomationState::default();
        workspace.active_view = AppView::Automations;
        cx.notify();
    });
    // Agent workflows gives each waiting workflow a card with its full
    // payload (PARITY-05), so its Start and the history's Run sit low on the
    // page; a control must be on screen to be clicked.
    cx.simulate_resize(gpui::size(gpui::px(1920.0), gpui::px(3200.0)));
    cx.run_until_parked();
    (workspace, cx)
}

/// [`fixture`] in a window tall enough for the workflow controls under the
/// Automations card. That card runs past 1080px since it took the web's
/// order (PM-48), the Local suggestions card adds its own height (PM-64),
/// and a control must be on screen to be clicked.
#[cfg(unix)]
pub(super) fn workflow_fixture(
    cx: &mut TestAppContext,
) -> (Entity<Workspace>, &mut VisualTestContext) {
    let (workspace, cx) = fixture(cx);
    // Tall enough for every Agent workflows card, review cards included.
    cx.simulate_resize(gpui::size(gpui::px(1920.0), gpui::px(3200.0)));
    cx.run_until_parked();
    (workspace, cx)
}
