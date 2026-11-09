//! Toggle switch for settings rows (switches, not checkboxes):
//! persimmon track when on, longan tone-step when off, no shadows.

use leptos::prelude::*;

#[component]
pub fn Switch(
    #[prop(optional, into)] checked: Signal<bool>,
    /// Fired with the next state on toggle.
    #[prop(optional)]
    on_change: Option<Callback<bool>>,
    #[prop(optional, into)] disabled: Signal<bool>,
    /// Accessible name when no visible label is associated.
    #[prop(optional, into)]
    label: String,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let label_attr = (!label.is_empty()).then_some(label);
    let toggle = move |_| {
        if disabled.get_untracked() {
            return;
        }
        if let Some(cb) = on_change {
            cb.run(!checked.get_untracked());
        }
    };
    view! {
        <button
            type="button"
            role="switch"
            aria-checked=move || checked.get().to_string()
            aria-label=label_attr
            disabled=move || disabled.get()
            class=move || format!(
                "relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-orb transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50 {} {class}",
                if checked.get() {
                    "bg-persimmon-500"
                } else {
                    "bg-longan-700 inset-ring inset-ring-white/10"
                }
            )
            on:click=toggle
        >
            <span
                aria-hidden="true"
                class=move || format!(
                    "pointer-events-none block size-5 rounded-orb bg-rambutan-100 transition-transform {}",
                    if checked.get() { "translate-x-[1.375rem]" } else { "translate-x-0.5" }
                )
            ></span>
        </button>
    }
}
