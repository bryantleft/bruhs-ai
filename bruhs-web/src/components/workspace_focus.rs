//! Keyboard navigation between visible workspace regions.

use std::collections::HashMap;

use leptos::prelude::*;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{Document, Element, FocusOptions, HtmlElement, KeyboardEvent};

const AREA: &str = "[data-workspace-focus]";
const BOUNDARY: &str = "[role=dialog], [role=alertdialog], [aria-modal=true], dialog[open]";
const HIDDEN: &str = "[hidden], [inert], [aria-hidden=true]";

#[derive(Clone, Copy)]
pub(super) struct WorkspaceFocus {
    history: StoredValue<HashMap<String, HtmlElement>, LocalStorage>,
}

impl WorkspaceFocus {
    pub(super) fn new() -> Self {
        let this = Self {
            history: StoredValue::new_local(HashMap::new()),
        };
        let listener = window_event_listener(leptos::ev::focusin, move |event| {
            let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlElement>().ok())
            else {
                return;
            };
            if target.closest(BOUNDARY).ok().flatten().is_some()
                || target
                    .closest("[data-thread-menu]")
                    .ok()
                    .flatten()
                    .is_some()
            {
                return;
            }
            if let Some(key) =
                focus_area(&target).and_then(|area| area.get_attribute("data-workspace-focus"))
            {
                this.history.update_value(|history| {
                    history.insert(key, target);
                });
            }
        });
        on_cleanup(move || listener.remove());
        this
    }

    pub(super) fn step(self, backwards: bool, settings: bool) -> bool {
        let document = document();
        if document.query_selector_all(BOUNDARY).is_ok_and(|dialogs| {
            (0..dialogs.length())
                .filter_map(|i| dialogs.item(i))
                .filter_map(|node| node.dyn_into::<Element>().ok())
                .any(|dialog| visible(&document, &dialog))
        }) {
            return false;
        }

        let order: &[&str] = if settings {
            &["settings-nav", "settings"]
        } else {
            &[
                "sidebar",
                "conversation",
                "composer",
                "details",
                "panel",
                "terminal",
            ]
        };
        let areas: Vec<_> = order
            .iter()
            .filter_map(|&name| {
                let find = |name| {
                    document
                        .query_selector(&format!("[data-workspace-focus=\"{name}\"]"))
                        .ok()
                        .flatten()
                        .filter(|area| visible(&document, area))
                };
                find(name)
                    .or_else(|| (name == "conversation").then(|| find("content")).flatten())
                    .map(|area| (name, area))
            })
            .collect();
        if areas.is_empty() {
            return false;
        }

        let current = document.active_element().and_then(|el| focus_area(&el));
        let index = current
            .as_ref()
            .and_then(|current| areas.iter().position(|(_, area)| area == current));
        let next = match (index, backwards) {
            (Some(index), false) => (index + 1) % areas.len(),
            (Some(index), true) => (index + areas.len() - 1) % areas.len(),
            (None, false) => 0,
            (None, true) => areas.len() - 1,
        };
        let (name, area) = &areas[next];
        let key = area
            .get_attribute("data-workspace-focus")
            .unwrap_or_default();
        let previous = self
            .history
            .with_value(|history| history.get(&key).cloned());
        let usable = |target: &HtmlElement| {
            focus_area(target).as_ref() == Some(area)
                && visible(&document, target)
                && !target.matches(":disabled").unwrap_or(true)
        };
        let preferred = match *name {
            "sidebar" => &[
                "[aria-current=page]",
                "[aria-label='Search threads and projects']",
            ][..],
            "composer" => &["#chat-input"][..],
            "settings-nav" => &["[aria-current=true]"][..],
            "terminal" => &[".xterm-helper-textarea"][..],
            "panel" | "details" => &["button:not(:disabled), a[href], input:not(:disabled)"][..],
            _ => &[],
        };
        let options = FocusOptions::new();
        options.set_prevent_scroll(true);
        for target in previous
            .filter(|target| usable(target))
            .into_iter()
            .chain(
                preferred
                    .iter()
                    .filter_map(|selector| {
                        area.query_selector(selector)
                            .ok()
                            .flatten()?
                            .dyn_into::<HtmlElement>()
                            .ok()
                    })
                    .filter(|target| usable(target)),
            )
            .chain(area.clone().dyn_into::<HtmlElement>().ok())
        {
            if target.focus_with_options(&options).is_ok()
                && document.active_element().as_ref() == Some(target.as_ref())
            {
                return true;
            }
        }
        false
    }
}

fn focus_area(element: &Element) -> Option<Element> {
    element.closest(AREA).ok().flatten()
}

fn visible(document: &Document, element: &Element) -> bool {
    if !element.is_connected() || element.closest(HIDDEN).ok().flatten().is_some() {
        return false;
    }
    let bounds = element.get_bounding_client_rect();
    bounds.width() > 0.0
        && bounds.height() > 0.0
        && document
            .default_view()
            .and_then(|window| window.get_computed_style(element).ok().flatten())
            .and_then(|style| style.get_property_value("visibility").ok())
            .is_some_and(|value| value != "hidden" && value != "collapse")
}

/// Capture these shortcuts before the focused editor or terminal consumes them.
pub(super) fn listen_to_workspace_keys(handler: impl Fn(KeyboardEvent) + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::<dyn FnMut(KeyboardEvent)>::new(handler);
    if window
        .add_event_listener_with_callback_and_bool(
            "keydown",
            callback.as_ref().unchecked_ref(),
            true,
        )
        .is_err()
    {
        return;
    }
    let listener = StoredValue::new_local(Some((window, callback)));
    on_cleanup(move || {
        listener.try_update_value(|listener| {
            if let Some((window, callback)) = listener.take() {
                let _ = window.remove_event_listener_with_callback_and_bool(
                    "keydown",
                    callback.as_ref().unchecked_ref(),
                    true,
                );
            }
        });
    });
}
