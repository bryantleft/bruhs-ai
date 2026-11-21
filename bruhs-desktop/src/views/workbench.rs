//! The local workbench domain UI is shared with web, without embedding the app shell.
use super::preview_panel::{PreviewPanelEvent, PreviewPanelView};
use bruhs_models::logic::workbench_draft::dictation_draft_text;
use gpui::{prelude::*, *};

/// Verification's "Use draft": dictation for the active thread's composer
/// (web: the Workbench frame's `bruhs-dictation-draft` message).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkbenchEvent {
    DictationDraft(String),
}

pub struct WorkbenchPanel {
    preview: Entity<PreviewPanelView>,
    /// bruhs-server, whose own pages alone may offer a draft.
    server_base: String,
    _events: Subscription,
    _page_messages: Task<()>,
}
impl EventEmitter<WorkbenchEvent> for WorkbenchPanel {}
impl WorkbenchPanel {
    /// Debug captures only: run `script` in the Workbench page.
    #[cfg(debug_assertions)]
    pub fn smoke_evaluate(&self, script: &str, cx: &App) -> Result<(), String> {
        self.preview.read(cx).smoke_evaluate(script)
    }

    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let server_base = std::env::var("BRUHS_SERVER_URL")
            .unwrap_or_else(|_| crate::server_client::DEFAULT_SERVER_URL.into());
        let (sender, page_messages) = async_channel::unbounded();
        let preview = cx.new(|cx| {
            let mut panel = PreviewPanelView::new(window, cx);
            panel.use_application_session();
            // The page relays Verification's draft through `window.ipc`.
            panel.set_page_messages(sender);
            // Web shows the Workbench page without browser chrome.
            panel.set_url_row(false);
            panel.go_to(
                &format!("{}/api/workbench/ui", server_base.trim_end_matches('/')),
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
        let _page_messages = cx.spawn(async move |this, cx| {
            while let Ok((frame, body)) = page_messages.recv().await {
                if this
                    .update(cx, |this, cx| this.receive_page_message(&frame, &body, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            preview,
            server_base,
            _events: events,
            _page_messages,
        }
    }
    /// A page's `window.ipc` message: a dictation draft from one of
    /// bruhs-server's own pages goes to the composer; anything else is
    /// ignored.
    pub(crate) fn receive_page_message(
        &mut self,
        frame: &str,
        body: &str,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(text) = dictation_draft_text(frame, &self.server_base, body) {
            cx.emit(WorkbenchEvent::DictationDraft(text));
        }
    }
    #[cfg(test)]
    pub(crate) fn server_base(&self) -> &str {
        &self.server_base
    }
    #[cfg(test)]
    pub(crate) fn url_row_shown(&self, cx: &App) -> bool {
        self.preview.read(cx).url_row_shown()
    }
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<'_, Self>) {
        self.preview.update(cx, |p, _| p.set_shown(shown));
    }
    pub fn open_handoff(
        &mut self,
        id: &bruhs_models::ids::ThreadId,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let base = std::env::var("BRUHS_SERVER_URL")
            .unwrap_or_else(|_| crate::server_client::DEFAULT_SERVER_URL.into());
        self.preview.update(cx, |preview, cx| {
            preview.go_to(
                &format!(
                    "{}/api/workbench/ui#handoff={id}",
                    base.trim_end_matches('/')
                ),
                window,
                cx,
            );
        });
    }
    #[cfg(test)]
    pub(crate) fn current_url(&self, cx: &App) -> Option<String> {
        self.preview.read(cx).url().map(str::to_owned)
    }
}
impl Render for WorkbenchPanel {
    fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
        div().size_full().child(self.preview.clone())
    }
}
