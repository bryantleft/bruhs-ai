//! Floating menu primitive (bruhs "floating" tone: longan-700 + white/15
//! ring). A transparent full-screen backdrop closes it on outside click;
//! Escape closes it too.

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuAlign {
    #[default]
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuSide {
    #[default]
    Bottom,
    Top,
}

#[component]
pub fn Menu(
    /// Open state, owned by the caller so triggers elsewhere can toggle it.
    open: RwSignal<bool>,
    #[prop(optional)] align: MenuAlign,
    #[prop(optional)] side: MenuSide,
    /// Extra classes on the popup (e.g. `min-w-[16rem]`).
    #[prop(optional, into)]
    class: String,
    /// Accessible name for the menu.
    #[prop(optional, into)]
    label: String,
    /// The trigger control; the caller wires its `on:click` to `open`.
    #[prop(into)]
    trigger: ViewFn,
    children: ChildrenFn,
) -> impl IntoView {
    let pos = match (align, side) {
        (MenuAlign::Start, MenuSide::Bottom) => "left-0 top-full mt-1",
        (MenuAlign::End, MenuSide::Bottom) => "right-0 top-full mt-1",
        (MenuAlign::Start, MenuSide::Top) => "left-0 bottom-full mb-1",
        (MenuAlign::End, MenuSide::Top) => "right-0 bottom-full mb-1",
    };
    let popup_class = format!(
        "absolute z-50 min-w-[12rem] max-w-[22rem] rounded-grape bg-longan-700 inset-ring inset-ring-white/15 p-1 {pos} {class}"
    );

    // Escape closes while open. The listener is attached only for the
    // lifetime of an open menu (JS closures are not `Send`, so they cannot
    // ride along in `on_cleanup`).
    let esc_listener = StoredValue::new_local(None::<Closure<dyn Fn(web_sys::Event)>>);
    Effect::new(move |_| {
        if open.get() {
            if esc_listener.with_value(|l| l.is_some()) {
                return;
            }
            let closure = Closure::<dyn Fn(web_sys::Event)>::new(move |raw: web_sys::Event| {
                if let Ok(ev) = raw.dyn_into::<web_sys::KeyboardEvent>() {
                    if ev.key() == "Escape" {
                        let _ = open.try_set(false);
                    }
                }
            });
            if let Some(window) = web_sys::window() {
                let _ = window
                    .add_event_listener_with_callback("keydown", closure.as_ref().unchecked_ref());
            }
            esc_listener.set_value(Some(closure));
        } else if let Some(closure) = esc_listener.try_update_value(|l| l.take()).flatten() {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "keydown",
                    closure.as_ref().unchecked_ref(),
                );
            }
        }
    });

    view! {
        <div class="relative inline-flex">
            {trigger.run()}
            <Show when=move || open.get() fallback=|| ()>
                <div
                    class="fixed inset-0 z-40 cursor-default"
                    aria-hidden="true"
                    on:click=move |_| open.set(false)
                />
                <div class=popup_class.clone() role="menu" aria-label=label.clone()>
                    {children()}
                </div>
            </Show>
        </div>
    }
}

#[component]
pub fn MenuItem(
    #[prop(optional)] on_select: Option<Callback<()>>,
    #[prop(optional, into)] selected: Signal<bool>,
    /// Secondary line under the label.
    #[prop(optional, into)]
    description: String,
    #[prop(optional)] destructive: bool,
    #[prop(optional, into)] class: String,
    children: Children,
) -> impl IntoView {
    let tone = if destructive {
        "text-lychee-300 hover:bg-lychee-500/10"
    } else {
        "text-rambutan-200 hover:bg-longan-800/60 hover:text-rambutan-100"
    };
    let base = format!(
        "flex w-full flex-col items-start gap-0.5 rounded-seed px-2.5 py-1.5 text-left text-label-sm transition-colors cursor-pointer {tone} {class}"
    );
    let base_selected = format!("{base} bg-longan-800/60 text-rambutan-100");
    let has_description = !description.is_empty();
    view! {
        <button
            type="button"
            role="menuitem"
            aria-checked=move || selected.get().to_string()
            class=move || if selected.get() { base_selected.clone() } else { base.clone() }
            on:click=move |_| {
                if let Some(cb) = on_select {
                    cb.run(());
                }
            }
        >
            <span class="flex w-full items-center gap-2">{children()}</span>
            {has_description.then(|| view! {
                <span class="text-label-sm text-rambutan-400">{description.clone()}</span>
            })}
        </button>
    }
}

/// Hairline between menu groups.
#[component]
pub fn MenuSeparator() -> impl IntoView {
    view! { <div role="separator" class="my-1 h-px bg-white/10" /> }
}
