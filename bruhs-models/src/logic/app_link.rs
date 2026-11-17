//! `bruhs://` links the desktop app opens from the OS: a thread, or any view
//! by its web route, so `bruhs://open/inbox/<id>` opens what `#/inbox/<id>`
//! opens on the web.

use crate::AppView;

/// A thread's link, which ⌘⇧C copies.
pub const THREAD_LINK_PREFIX: &str = "bruhs://thread/";
/// A view's link is this followed by its route.
pub const VIEW_LINK_PREFIX: &str = "bruhs://open";

/// What a `bruhs://` link opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppLink {
    Thread(String),
    View(AppView),
}

/// The link to the thread `id`.
pub fn thread_link(id: &str) -> String {
    format!("{THREAD_LINK_PREFIX}{id}")
}

/// The link to `view`.
pub fn view_link(view: &AppView) -> String {
    format!("{VIEW_LINK_PREFIX}{}", view.route())
}

/// What `link` opens, or `None` for a link this build does not know.
pub fn parse_app_link(link: &str) -> Option<AppLink> {
    let link = link.trim();
    if let Some(rest) = link.strip_prefix(THREAD_LINK_PREFIX) {
        let id = rest.trim_end_matches('/');
        return (!id.is_empty()).then(|| AppLink::Thread(id.to_string()));
    }
    let route = link
        .strip_prefix(VIEW_LINK_PREFIX)
        .filter(|route| route.starts_with('/'))?;
    AppView::from_route(route).map(AppLink::View)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_threads_and_views_by_their_web_routes() {
        assert_eq!(
            parse_app_link(&thread_link("t-1")),
            Some(AppLink::Thread("t-1".into()))
        );
        assert_eq!(
            parse_app_link(" bruhs://thread/t-1/ "),
            Some(AppLink::Thread("t-1".into()))
        );
        let view = AppView::ConversationDetail {
            conversation_id: "c-1".into(),
        };
        assert_eq!(view_link(&view), "bruhs://open/inbox/c-1");
        assert_eq!(parse_app_link(&view_link(&view)), Some(AppLink::View(view)));
        assert_eq!(
            parse_app_link("bruhs://open/workbench"),
            Some(AppLink::View(AppView::Workbench))
        );
        for unknown in [
            "bruhs://thread/",
            "bruhs://open",
            "bruhs://openinbox",
            "bruhs://open/nowhere",
            "https://bruhs.ai/#/inbox",
        ] {
            assert_eq!(parse_app_link(unknown), None, "{unknown}");
        }
    }
}
