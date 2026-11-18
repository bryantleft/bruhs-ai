//! A bruh's words as Markdown, drawn the way a thread chat draws a reply:
//! [`crate::markdown::render_untrusted`] into a `.prose-msg` block, with the
//! render pass for code, math and diagrams. Crew prose also carries
//! `.crew-prose`, and its links are handled here rather than by the thread
//! chat's link handler: a room or a bruh's chat has no thread, so a link
//! never opens a file against the active thread's project. A web or mail
//! link opens in a new tab; any other link stays in the page and does
//! nothing.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::render_pass::{has_math, schedule_render_pass};

/// Marks crew prose, which the thread chat's link handler leaves alone.
pub(crate) const CREW_PROSE: &str = "crew-prose";

/// The classes of a bubble that holds crew prose.
pub(super) const PROSE_BUBBLE: &str = "prose-msg crew-prose min-w-0 break-words rounded-grape bg-longan-900 px-3 py-2 text-body text-rambutan-200 inset-ring inset-ring-longan-700/50";

/// Where a crew link goes: a web or mail address opens in a new tab, and
/// `None` keeps everything else (a file path, a fragment) in the page.
pub(crate) fn crew_link(href: &str) -> Option<&str> {
    let safe = crate::markdown::safe_url(href)?;
    let lower = safe.trim().to_ascii_lowercase();
    (lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("//"))
    .then_some(safe)
}

/// Run the render pass whenever `texts` changes, and handle clicks in crew
/// prose while the page is up: links as [`crew_link`] says, and a
/// diagram's Open as board as the thread chat does.
pub(super) fn install_crew_prose(texts: impl Fn() -> Vec<String> + 'static) {
    Effect::new(move |_| {
        let texts = texts();
        let needs_math = texts.iter().any(|text| has_math(text));
        let needs_mermaid = texts.iter().any(|text| text.contains("```mermaid"));
        schedule_render_pass(needs_math, needs_mermaid);
    });
    let boards = expect_context::<crate::whiteboard::host::WhiteboardHost>();
    let listener = window_event_listener(leptos::ev::click, move |ev| {
        let Some(element) = ev
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
            .and_then(|node| match node.dyn_ref::<web_sys::Element>() {
                Some(element) => Some(element.clone()),
                None => node.parent_element(),
            })
        else {
            return;
        };
        if element
            .closest(&format!(".{CREW_PROSE}"))
            .ok()
            .flatten()
            .is_none()
        {
            return;
        }
        if let Some(button) = element
            .closest("button[data-open-whiteboard]")
            .ok()
            .flatten()
        {
            if let Some(source) = button.get_attribute("data-open-whiteboard") {
                ev.prevent_default();
                boards.open_mermaid(source);
            }
            return;
        }
        let Some(anchor) = element.closest("a").ok().flatten() else {
            return;
        };
        ev.prevent_default();
        let href = anchor.get_attribute("href").unwrap_or_default();
        if let Some(url) = crew_link(&href) {
            let _ = window().open_with_url_and_target_and_features(
                url,
                "_blank",
                "noopener,noreferrer",
            );
        }
    });
    on_cleanup(move || listener.remove());
}

/// Keep a transcript that follows its newest line at the bottom while its
/// rows grow after they draw (highlighted code, fonts), as the thread chat
/// does. `following` is whether the reader is at the bottom: the returned
/// handler updates it on each scroll, and Load earlier clears it.
pub(super) fn stick_to_bottom(
    scroller: NodeRef<leptos::html::Div>,
    following: StoredValue<bool>,
) -> impl Fn(leptos::ev::Event) + Copy + 'static {
    let observer = StoredValue::new_local(
        None::<(
            web_sys::ResizeObserver,
            wasm_bindgen::closure::Closure<dyn FnMut()>,
        )>,
    );
    Effect::new(move |_| {
        let Some(element) = scroller.get() else {
            return;
        };
        let Some(content) = element.first_element_child() else {
            return;
        };
        let callback = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
            if following.try_get_value() == Some(true) {
                if let Some(element) = scroller.get_untracked() {
                    element.set_scroll_top(element.scroll_height());
                }
            }
        }) as Box<dyn FnMut()>);
        if let Ok(resize) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            resize.observe(&content);
            observer.update_value(|value| {
                if let Some((old, _)) = value.replace((resize, callback)) {
                    old.disconnect();
                }
            });
        }
    });
    on_cleanup(move || {
        if let Some((resize, _callback)) = observer.try_update_value(Option::take).flatten() {
            resize.disconnect();
        }
    });
    move |_| {
        if let Some(element) = scroller.get_untracked() {
            let below = element.scroll_height() - element.client_height() - element.scroll_top();
            following.set_value(below <= 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::crew_link;

    #[test]
    fn web_and_mail_links_open_in_a_tab_and_the_rest_stay_put() {
        assert_eq!(
            crew_link("https://example.com/list"),
            Some("https://example.com/list")
        );
        assert_eq!(crew_link("HTTP://example.com"), Some("HTTP://example.com"));
        assert_eq!(
            crew_link("mailto:bo@example.com"),
            Some("mailto:bo@example.com")
        );
        assert_eq!(crew_link("//example.com/x"), Some("//example.com/x"));
        // A file path would open against a thread's project: not here.
        assert_eq!(crew_link("notes/plan.md"), None);
        assert_eq!(crew_link("/Users/me/plan.md"), None);
        assert_eq!(crew_link("file:///Users/me/plan.md"), None);
        assert_eq!(crew_link("#top"), None);
        assert_eq!(crew_link("javascript:alert(1)"), None);
    }
}
