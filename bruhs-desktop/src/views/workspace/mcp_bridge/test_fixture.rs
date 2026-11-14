//! Workspace UI fixture shared by portable view tests and Unix IPC tests.
use super::super::{AppView, AutomationState, DesktopState, Workspace};
use gpui::{AppContext, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

pub(super) fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    // Some callers cross real Tokio IO; they cannot use only the deterministic
    // executor's virtual clock. The fixture itself does not open an IPC socket.
    cx.executor().allow_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let cell = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        gpui_component::Root::new(workspace, window, cx)
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
    (workspace, cx)
}
