//! Range slider on bruhs tones: a longan track with a persimmon fill under
//! a transparent native range input (kept for keyboard and pointer
//! semantics; `.bruhs-range` in index.html hides its OS chrome and draws
//! the thumb). Emits the parsed `f64` value on input.

use leptos::prelude::*;

/// Fill width as a CSS length: the thumb's centre, so the fill meets it.
/// The thumb is 1rem wide, so 0% sits 0.5rem in and 100% sits 0.5rem short.
pub fn fill_width(value: f64, min: f64, max: f64) -> String {
    let span = max - min;
    let pct = if span > 0.0 {
        ((value - min) / span).clamp(0.0, 1.0)
    } else {
        0.0
    };
    format!("calc(0.5rem + (100% - 1rem) * {pct:.4})")
}

#[component]
pub fn Slider(
    #[prop(optional, into)] value: Signal<f64>,
    #[prop(optional)] on_input: Option<Callback<f64>>,
    #[prop(into)] min: f64,
    #[prop(into)] max: f64,
    #[prop(optional, into)] step: Option<f64>,
    #[prop(optional, into)] disabled: Signal<bool>,
    #[prop(optional, into)] class: String,
    /// Accessible name for the range input.
    #[prop(optional, into)]
    label: String,
) -> impl IntoView {
    let step = step.unwrap_or(1.0);
    let label_attr = (!label.is_empty()).then_some(label);
    let on_input_handler = move |ev: web_sys::Event| {
        if let Some(cb) = on_input {
            use wasm_bindgen::JsCast;
            if let Some(input) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            {
                if let Ok(v) = input.value().parse::<f64>() {
                    cb.run(v);
                }
            }
        }
    };

    view! {
        <div class=format!("relative flex h-5 w-full items-center {class}")>
            <div class="absolute inset-x-0 h-1.5 rounded-orb bg-longan-700 inset-ring inset-ring-white/5" aria-hidden="true"></div>
            <div
                class=move || format!(
                    "absolute left-0 h-1.5 rounded-orb {}",
                    if disabled.get() { "bg-rambutan-600" } else { "bg-persimmon-500" }
                )
                style=move || format!("width:{}", fill_width(value.get(), min, max))
                aria-hidden="true"
            ></div>
            <input
                type="range"
                min=min.to_string()
                max=max.to_string()
                step=step.to_string()
                aria-label=label_attr
                class="bruhs-range relative z-10 w-full"
                disabled=move || disabled.get()
                prop:value=move || value.get().to_string()
                on:input=on_input_handler
            />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_tracks_the_thumb_centre() {
        assert_eq!(
            fill_width(2.0, 2.0, 20.0),
            "calc(0.5rem + (100% - 1rem) * 0.0000)"
        );
        assert_eq!(
            fill_width(20.0, 2.0, 20.0),
            "calc(0.5rem + (100% - 1rem) * 1.0000)"
        );
        assert_eq!(
            fill_width(11.0, 2.0, 20.0),
            "calc(0.5rem + (100% - 1rem) * 0.5000)"
        );
        assert_eq!(
            fill_width(5.0, 5.0, 5.0),
            "calc(0.5rem + (100% - 1rem) * 0.0000)"
        );
    }
}
