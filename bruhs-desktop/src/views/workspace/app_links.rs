//! `bruhs://` links opened from the OS: a thread, or a view by its web route
//! (`bruhs_models::logic::app_link`), so a link reaches the same place on
//! desktop that its route reaches on the web.

use super::*;
use bruhs_models::logic::app_link::{parse_app_link, AppLink};

pub(super) const APP_LINK_UNKNOWN: &str = "Bruhs cannot open that link.";
pub(super) const APP_LINK_THREAD_MISSING: &str = "That thread is not on this device.";

impl Workspace {
    pub fn open_app_link(&mut self, link: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        match parse_app_link(link) {
            Some(AppLink::Thread(id)) if self.threads.iter().any(|thread| thread.id == id) => {
                self.select_thread_by_id(&id, cx);
            }
            Some(AppLink::Thread(_)) => self.app_link_notice(APP_LINK_THREAD_MISSING, cx),
            Some(AppLink::View(view)) => self.open_routed_view(view, window, cx),
            None => self.app_link_notice(APP_LINK_UNKNOWN, cx),
        }
    }

    fn app_link_notice(&mut self, message: &'static str, cx: &mut Context<'_, Self>) {
        self.meetings_hub
            .update(cx, |hub, cx| hub.show_notice(message, true, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AppView, Workspace};
    use super::{APP_LINK_THREAD_MISSING, APP_LINK_UNKNOWN};
    use crate::{app_state::DesktopState, theme};
    use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
    use std::{cell::RefCell, rc::Rc};

    fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            theme::apply_bruhs_theme(cx);
            cx.set_global(DesktopState::new().expect("desktop state"));
        });
        let entity = Rc::new(RefCell::new(None));
        let captured = entity.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let workspace = cx.new(|cx| Workspace::new(window, cx));
            *captured.borrow_mut() = Some(workspace.clone());
            gpui_component::Root::new(workspace, window, cx)
        });
        let workspace = entity.borrow().clone().expect("workspace");
        (workspace, cx)
    }

    fn notice(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
        workspace.read_with(cx, |workspace, cx| {
            workspace
                .meetings_hub
                .read(cx)
                .notice
                .as_ref()
                .map(|notice| notice.message.clone())
        })
    }

    #[gpui::test]
    fn links_open_views_and_threads_like_their_web_routes(cx: &mut TestAppContext) {
        let (workspace, cx) = workspace(cx);
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_app_link("bruhs://open/inbox", window, cx);
            assert_eq!(workspace.active_view, AppView::Inbox);
            workspace.open_app_link("bruhs://open/workbench", window, cx);
            assert_eq!(workspace.active_view, AppView::Workbench);
            let id = workspace.threads[0].id.clone();
            workspace.open_app_link(&format!("bruhs://thread/{id}"), window, cx);
            assert_eq!(workspace.active_view, AppView::Chat);
            assert_eq!(workspace.active_thread_id.as_deref(), Some(id.as_str()));
            workspace.open_app_link("bruhs://thread/not-here", window, cx);
        });
        assert_eq!(
            notice(&workspace, cx).as_deref(),
            Some(APP_LINK_THREAD_MISSING)
        );
        workspace.update_in(cx, |workspace, window, cx| {
            workspace.open_app_link("bruhs://open/nowhere", window, cx);
        });
        assert_eq!(notice(&workspace, cx).as_deref(), Some(APP_LINK_UNKNOWN));
    }
}
