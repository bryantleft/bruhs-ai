use leptos::prelude::*;

#[component]
pub fn Input(
    #[prop(optional, into)] id: Option<String>,
    #[prop(optional, into)] value: Option<Signal<String>>,
    #[prop(optional)] on_input: Option<Callback<String>>,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional, into)] aria_label: Option<String>,
    #[prop(optional, into)] role: Option<String>,
    #[prop(optional, into)] aria_controls: Option<String>,
    #[prop(optional, into)] aria_activedescendant: Option<Signal<Option<String>>>,
    #[prop(optional, into)] aria_expanded: Option<Signal<bool>>,
    #[prop(optional)] on_keydown: Option<Callback<web_sys::KeyboardEvent>>,
    #[prop(optional, into)] input_type: Option<String>,
    #[prop(optional, into)] class: String,
    #[prop(optional)] autofocus: bool,
    #[prop(optional, into)] disabled: Signal<bool>,
) -> impl IntoView {
    let final_class = format!(
        "block w-full rounded-grape bg-white/5 px-3 py-2 text-body text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400 disabled:opacity-60 disabled:cursor-not-allowed {class}"
    );
    let t = input_type.unwrap_or_else(|| "text".to_string());

    // HTML `autofocus` only applies during initial document parse, so it
    // does nothing for SPA view-mounts — focus explicitly via NodeRef.
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();
    if autofocus {
        Effect::new(move |_| {
            if let Some(el) = input_ref.get() {
                let _ = el.focus();
            }
        });
    }

    let on_input_handler = move |ev: web_sys::Event| {
        if let Some(cb) = on_input {
            use wasm_bindgen::JsCast;
            if let Some(input) = ev
                .target()
                .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                cb.run(input.value());
            }
        }
    };

    view! {
        <input
            id=id
            node_ref=input_ref
            type=t
            class=final_class
            placeholder=placeholder
            aria-label=aria_label
            role=role
            aria-controls=aria_controls
            aria-activedescendant=move || aria_activedescendant.and_then(|value| value.get())
            aria-expanded=move || aria_expanded.map(|value| value.get().to_string())
            autofocus=autofocus
            disabled=move || disabled.get()
            prop:value=move || value.map(|v| v.get()).unwrap_or_default()
            on:input=on_input_handler
            on:keydown=move |ev| { if let Some(callback) = on_keydown { callback.run(ev); } }
        />
    }
}
