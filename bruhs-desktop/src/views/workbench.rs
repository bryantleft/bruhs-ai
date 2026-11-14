//! The local workbench domain UI is shared with web, without embedding the app shell.
use super::preview_panel::{PreviewPanelEvent, PreviewPanelView};
use gpui::{prelude::*, *};

pub struct WorkbenchPanel {
    preview: Entity<PreviewPanelView>,
    _events: Subscription,
}
impl WorkbenchPanel {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let preview = cx.new(|cx| {
            let mut panel = PreviewPanelView::new(window, cx);
            let base = std::env::var("BRUHS_SERVER_URL")
                .unwrap_or_else(|_| crate::server_client::DEFAULT_SERVER_URL.into());
            panel.go_to(
                &format!("{}/api/workbench/ui", base.trim_end_matches('/')),
                window,
                cx,
            );
            panel.set_shown(false);
            panel
        });
        let events = cx.subscribe(&preview, |_, _, event: &PreviewPanelEvent, cx| {
            let PreviewPanelEvent::OpenExternal(url) = event;
            cx.open_url(url);
        });
        Self {
            preview,
            _events: events,
        }
    }
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<'_, Self>) {
        self.preview.update(cx, |p, _| p.set_shown(shown));
    }
}
impl Render for WorkbenchPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().size_full().child(self.preview.clone())
    }
}
