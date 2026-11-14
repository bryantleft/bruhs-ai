//! A single task-domain view is shared with the web and paired phone clients.
use super::preview_panel::{PreviewPanelEvent, PreviewPanelView};
use gpui::{prelude::*, *};

pub struct CoordinationPanel {
    preview: Entity<PreviewPanelView>,
    _events: Subscription,
}

impl CoordinationPanel {
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let preview = cx.new(|cx| {
            let mut view = PreviewPanelView::new(window, cx);
            let base = std::env::var("BRUHS_SERVER_URL")
                .unwrap_or_else(|_| crate::server_client::DEFAULT_SERVER_URL.into());
            view.go_to(
                &format!("{}/api/coordination/ui", base.trim_end_matches('/')),
                window,
                cx,
            );
            view
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
        self.preview.update(cx, |view, _| view.set_shown(shown));
    }
}

impl Render for CoordinationPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        self.preview.update(cx, |view, _| view.set_shown(true));
        div().size_full().child(self.preview.clone())
    }
}
