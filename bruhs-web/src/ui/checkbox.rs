//! Checkbox on bruhs tones: a button with `role="checkbox"`, so no OS
//! control ever shows. Persimmon when checked, input tone otherwise. Same
//! props as before, so `<label>` wrappers and `id`/`for` pairs keep working
//! (buttons are labelable elements).

use leptos::prelude::*;

use crate::ui::Icon;

#[component]
pub fn Checkbox(
    /// Current checked state.
    #[prop(optional, into)]
    checked: Signal<bool>,
    /// Fired with the new checked state on toggle.
    #[prop(optional)]
    on_change: Option<Callback<bool>>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] class: String,
    /// Optional element id, for explicit `<label for=…>` association.
    #[prop(optional, into)]
    id: Option<String>,
    /// Accessible name when no visible label is associated.
    #[prop(optional, into)]
    label: String,
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
            role="checkbox"
            id=id
            aria-checked=move || checked.get().to_string()
            aria-label=label_attr
            disabled=move || disabled.get()
            class=move || format!(
                "inline-flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-seed transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-persimmon-400 disabled:cursor-not-allowed disabled:opacity-50 {} {class}",
                if checked.get() {
                    "bg-persimmon-500 text-longan-950"
                } else {
                    "bg-white/5 text-transparent inset-ring inset-ring-white/15 hover:bg-white/10"
                }
            )
            on:click=toggle
        >
            <Icon name="check" size=14 class="stroke-[3]" />
        </button>
    }
}
