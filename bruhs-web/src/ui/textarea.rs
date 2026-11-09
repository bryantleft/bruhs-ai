use leptos::prelude::*;

#[component]
pub fn Textarea(
    #[prop(optional)] id: Option<String>,
    #[prop(optional, into)] aria_label: Option<String>,
    #[prop(optional, into)] value: Option<Signal<String>>,
    #[prop(optional)] on_input: Option<Callback<String>>,
    #[prop(optional)] on_keydown: Option<Callback<web_sys::KeyboardEvent>>,
    #[prop(optional, into)] placeholder: String,
    #[prop(optional)] rows: Option<u32>,
    #[prop(optional, into)] class: String,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional)] autofocus: bool,
) -> impl IntoView {
    let final_class = format!(
        "resize-none block w-full rounded-grape bg-white/5 px-3 py-2 text-body text-rambutan-100 placeholder:text-rambutan-500 inset-ring inset-ring-white/10 focus-visible:outline-2 -outline-offset-1 focus-visible:outline-persimmon-400 disabled:opacity-60 disabled:cursor-not-allowed {class}"
    );

    let r = rows.unwrap_or(3);

    let on_input_handler = move |ev: web_sys::Event| {
        if let Some(cb) = on_input {
            use wasm_bindgen::JsCast;
            if let Some(ta) = ev
                .target()
                .and_then(|target| target.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
            {
                cb.run(ta.value());
            }
        }
    };

    let on_keydown_handler = move |ev: web_sys::KeyboardEvent| {
        if let Some(cb) = on_keydown {
            cb.run(ev);
        }
    };

    view! {
        <textarea
            id=id
            aria-label=aria_label
            class=final_class
            placeholder=placeholder
            rows=r
            disabled=move || disabled.get()
            autofocus=autofocus
            prop:value=move || value.map(|v| v.get()).unwrap_or_default()
            on:input=on_input_handler
            on:keydown=on_keydown_handler
        />
    }
}
