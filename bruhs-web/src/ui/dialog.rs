use leptos::html::Div;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, KeyboardEvent};

/// A modal dialog with a backdrop, role="dialog", and a focus trap.
///
/// Keyboard behavior (WCAG 2.4.3 Focus Order, 2.1.2 No Keyboard Trap):
/// - Tab from the last focusable element wraps to the first.
/// - Shift+Tab from the first wraps to the last.
/// - Escape is handled by the consumer via `on_close`, but a backup Escape
///   handler is attached to the dialog so it works even when focus has been
///   pulled to a child.
///
/// Dismissal restores the invoking control when it still exists, unless the
/// action has already moved focus to another destination.
#[component]
pub fn Dialog(
    /// When true, the dialog is rendered. When false, nothing is rendered.
    #[prop(into)]
    open: Signal<bool>,
    /// Called when the user clicks the backdrop or presses Escape.
    #[prop(optional)]
    on_close: Option<Callback<()>>,
    /// Optional max-width class for the card. Defaults to `max-w-xl`.
    #[prop(optional, into)]
    max_width: Option<String>,
    /// Optional padding-top distance from the top of the viewport.
    #[prop(optional, into)]
    padding_top: Option<String>,
    /// Accessible name for the dialog (announced by screen readers).
    #[prop(optional, into)]
    label: Option<String>,
    children: ChildrenFn,
) -> impl IntoView {
    let max_w = max_width.unwrap_or_else(|| "max-w-xl".to_string());
    let pt = padding_top.unwrap_or_else(|| "8rem".to_string());

    // Keep the warm modal scrim without filtering the entire workspace on every
    // keystroke. Full-screen backdrop blur exceeded the command paint budget.
    // Padding-top stays inline because it's a per-instance prop.
    let backdrop_class = "fixed inset-0 z-50 flex items-start justify-center bg-longan-950/70";
    let backdrop_style = format!("padding-top: {pt};");

    // Card = modal tone (bruhs §6): bg-longan-800 + inset-ring-white/20.
    // rounded-lychee (14px) is the bruhs panel/card radius. Lifting from
    // floating tone (longan-700/white-15) to modal tone (longan-800/white-20)
    // gives the dialog the strongest elevation step in the system.
    let card_class = format!(
        "w-full {max_w} rounded-lychee bg-longan-800 inset-ring inset-ring-white/20 overflow-hidden"
    );

    view! {
        <Show when=move || open.get() fallback=|| ()>
            <DialogSurface
                on_close=on_close
                label=label.clone()
                backdrop_class=backdrop_class
                backdrop_style=backdrop_style.clone()
                card_class=card_class.clone()
                content=children.clone()
            />
        </Show>
    }
}

/// Mounted once per opening: capture the origin before children autofocus and
/// attach cleanup to this opening, not the long-lived Dialog component.
#[component]
fn DialogSurface(
    on_close: Option<Callback<()>>,
    label: Option<String>,
    backdrop_class: &'static str,
    backdrop_style: String,
    card_class: String,
    content: ChildrenFn,
) -> impl IntoView {
    let card_ref: NodeRef<Div> = NodeRef::new();
    let origin = StoredValue::new_local(
        document()
            .active_element()
            .and_then(|element| element.dyn_into::<HtmlElement>().ok()),
    );
    on_cleanup(move || {
        let target = origin.get_value();
        let closing_card = card_ref.get_untracked().map(|card| (*card).clone());
        // Let unmount and destination autofocus finish first. No timer/frame
        // delay, and no attempt to focus an element removed by navigation.
        queue_microtask(move || {
            let Some(target) = target else { return };
            if !target.is_connected()
                || target.offset_parent().is_none()
                || target.matches(":disabled").unwrap_or(true)
            {
                return;
            }
            if let Some(active) = document().active_element() {
                let still_closing = closing_card
                    .as_ref()
                    .is_some_and(|card| card.contains(Some(&active)));
                if active.tag_name() != "BODY" && !still_closing {
                    return;
                }
            }
            // A newly opened modal owns focus; a nested modal may instead
            // return to its invoking control inside the remaining parent.
            if let Ok(dialogs) = document().query_selector_all("[role=dialog]") {
                if let Some(top) = dialogs.item(dialogs.length().saturating_sub(1)) {
                    if !top.contains(Some(&target)) {
                        return;
                    }
                }
            }
            let _ = target.focus();
        });
    });

    // When the dialog opens, focus the most useful element inside so keyboard
    // users don't have to tab from the page body into the modal. A child can
    // claim initial focus with `data-autofocus` (e.g. the selected option in
    // a picker); otherwise the first focusable wins.
    Effect::new(move |_| {
        if let Some(card) = card_ref.get() {
            let html: HtmlElement = (*card).clone();
            let element: &web_sys::Element = html.as_ref();
            let preferred = element
                .query_selector("[data-autofocus]")
                .ok()
                .flatten()
                .and_then(|el| el.dyn_into::<HtmlElement>().ok());
            if let Some(el) = preferred {
                let _ = el.focus();
            } else if let Some(first) = focusable_descendants(&html).first() {
                let _ = first.focus();
            } else {
                let _ = html.focus();
            }
        }
    });

    // Key handler for Tab cycling and Escape fallback. We attach to the
    // dialog card rather than window so there's no global listener to clean
    // up — if the dialog unmounts, the handler goes with it.
    let on_key = move |ev: KeyboardEvent| {
        let key = ev.key();
        if key == "Escape" {
            if let Some(cb) = on_close {
                ev.prevent_default();
                cb.run(());
            }
            return;
        }
        if key != "Tab" {
            return;
        }
        let Some(card) = card_ref.get() else { return };
        let html: HtmlElement = (*card).clone();
        let focusables = focusable_descendants(&html);
        if focusables.is_empty() {
            ev.prevent_default();
            return;
        }
        let first = &focusables[0];
        let last = &focusables[focusables.len() - 1];
        let active = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element())
            .and_then(|el| el.dyn_into::<HtmlElement>().ok());
        let Some(active) = active else { return };
        if ev.shift_key() {
            if active.is_same_node(Some(first)) {
                ev.prevent_default();
                let _ = last.focus();
            }
        } else if active.is_same_node(Some(last)) {
            ev.prevent_default();
            let _ = first.focus();
        }
    };

    view! {
            <div
                role="dialog"
                aria-modal="true"
                aria-label=label
                class=backdrop_class
                style=backdrop_style
                on:click=move |_| { if let Some(cb) = on_close { cb.run(()); } }
                on:keydown=on_key
            >
                <div
                    node_ref=card_ref
                    tabindex="-1"
                    class=card_class
                    on:click=move |ev| ev.stop_propagation()
                >
                    {content()}
                </div>
            </div>
    }
}

/// Collect every element inside `root` that participates in sequential focus.
/// This is a small, pragmatic version of what libraries like `focus-trap`
/// provide — covers buttons, links, inputs, textareas, and `tabindex >= 0`
/// elements, and filters out ones that are disabled or hidden.
fn focusable_descendants(root: &HtmlElement) -> Vec<HtmlElement> {
    const SELECTOR: &str = "a[href], button:not([disabled]), input:not([disabled]), \
         select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex=\"-1\"])";
    let element: &web_sys::Element = root.as_ref();
    let list = match element.query_selector_all(SELECTOR) {
        Ok(l) => l,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for i in 0..list.length() {
        let Some(node) = list.item(i) else { continue };
        if let Ok(el) = node.dyn_into::<HtmlElement>() {
            // Skip hidden elements — offsetParent is null for display:none
            // nodes and elements whose ancestors are display:none.
            if el.offset_parent().is_some() || el.tag_name().to_lowercase() == "body" {
                out.push(el);
            }
        }
    }
    out
}
