//! Small copy-to-clipboard button: `copy` glyph, flips to a green `check`
//! for 1.5s after a click. Uses the async Clipboard API through `Reflect`
//! so no extra web-sys feature is needed.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::ui::Icon;

/// Write `text` to the system clipboard; silently no-ops when the API is
/// unavailable (insecure context, old browser).
pub fn copy_to_clipboard(text: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let navigator = window.navigator();
    let Ok(clipboard) = js_sys::Reflect::get(&navigator, &"clipboard".into()) else {
        return;
    };
    if clipboard.is_undefined() || clipboard.is_null() {
        return;
    }
    let Ok(write_text) = js_sys::Reflect::get(&clipboard, &"writeText".into()) else {
        return;
    };
    let Ok(write_text) = write_text.dyn_into::<js_sys::Function>() else {
        return;
    };
    let _ = write_text.call1(&clipboard, &wasm_bindgen::JsValue::from_str(text));
}

#[component]
pub fn CopyButton(
    /// Text placed on the clipboard.
    #[prop(into)]
    text: String,
    /// Accessible name / tooltip. Default "Copy".
    #[prop(optional, into)]
    label: String,
    /// Extra classes for the button element.
    #[prop(optional, into)]
    class: String,
) -> impl IntoView {
    let (copied, set_copied) = signal(false);
    // A second click restarts the "copied" moment instead of being cut
    // short by the first click's reset.
    let reset = super::ResetTimer::new();
    let text = StoredValue::new(text);
    let label = if label.is_empty() {
        "Copy".to_string()
    } else {
        label
    };
    let title = label.clone();
    let on_click = move |ev: leptos::ev::MouseEvent| {
        ev.stop_propagation();
        copy_to_clipboard(&text.get_value());
        set_copied.set(true);
        reset.after(1_500, move || {
            set_copied.try_set(false);
        });
    };
    view! {
        <button
            type="button"
            class=format!(
                "inline-flex size-6 shrink-0 items-center justify-center rounded-seed text-rambutan-500 transition-colors hover:bg-longan-800/60 hover:text-rambutan-100 {class}"
            )
            aria-label=label
            title=title
            on:click=on_click
        >
            {move || {
                if copied.get() {
                    view! { <Icon name="check" size=12 class="text-pandan-400" /> }
                } else {
                    view! { <Icon name="copy" size=12 /> }
                }
            }}
        </button>
    }
}
