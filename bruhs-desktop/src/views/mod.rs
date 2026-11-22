pub(crate) mod api_client_panel;
pub mod braille;
pub(crate) mod browser_preferences;
mod acp_models;
pub mod chat;
mod model_browser;
pub(crate) mod language;
pub(crate) mod copy_feedback;
pub mod crew_face;
pub(crate) mod database_panel;
pub mod diff_panel;
pub mod dropdown;
pub mod meeting_dock;
pub mod meetings_hub;
pub mod native_blur;
pub mod page_header;
pub mod preview_panel;
pub(crate) mod reactive_automations;
pub(crate) mod service_connections;
pub mod sidebar_order;
mod task_registry_subscription;
pub mod tasks_panel;
pub mod terminal_panel;
pub mod terminal_view;
pub mod web_button;
pub(crate) mod whiteboard;
pub mod workbench;
pub mod workspace;

pub(crate) mod theme_regions;
/// `selector` as the `&'static str` a control's id must be, kept once for
/// the life of the app: the set grows only with the rows that ask (vault
/// entries, app accounts, mailboxes and bruhs), and asking again returns
/// the same one. The bruh page's Vault and Apps switches and Settings,
/// Apps' grant switches share it.
pub(crate) fn kept_selector(selector: String) -> &'static str {
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock, PoisonError};
    static SELECTORS: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut selectors = SELECTORS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(kept) = selectors.get(selector.as_str()) {
        return kept;
    }
    let kept: &'static str = Box::leak(selector.into_boxed_str());
    selectors.insert(kept);
    kept
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_selector_is_kept_once() {
        let first = super::kept_selector("desktop-test-kept-a".into());
        assert_eq!(first, "desktop-test-kept-a");
        assert!(std::ptr::eq(
            first,
            super::kept_selector("desktop-test-kept-a".into())
        ));
        assert!(!std::ptr::eq(
            first,
            super::kept_selector("desktop-test-kept-b".into())
        ));
    }
}
