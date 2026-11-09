//! Numeric stepper: minus / value / plus on one input-toned pill, with
//! ↑/↓ nudging and typed values clamped to the step grid. Replaces native
//! number spinners in settings rows.

use bruhs_models::num::ToIntSaturating;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::ui::Icon;

/// Snap `v` to the step grid (from `min` when given) and clamp to bounds.
pub fn snap(v: f64, min: Option<f64>, max: Option<f64>, step: f64) -> f64 {
    let step = if step > 0.0 { step } else { 1.0 };
    let base = min.unwrap_or(0.0);
    let mut out = base + ((v - base) / step).round() * step;
    if let Some(lo) = min {
        out = out.max(lo);
    }
    if let Some(hi) = max {
        out = out.min(hi);
    }
    // Avoid -0 and float noise for integer grids.
    if step.fract() == 0.0 {
        out = out.round();
    }
    if out == 0.0 {
        0.0
    } else {
        out
    }
}

/// Integer grids print as integers; fractional grids keep two decimals.
pub fn format_value(v: f64, step: f64) -> String {
    if step.fract() == 0.0 {
        format!("{}", v.round().to_i64_saturating())
    } else {
        format!("{v:.2}")
    }
}

#[component]
pub fn Stepper(
    #[prop(into)] value: Signal<f64>,
    #[prop(optional)] on_change: Option<Callback<f64>>,
    #[prop(optional, into)] min: Option<f64>,
    #[prop(optional, into)] max: Option<f64>,
    #[prop(optional, into)] step: Option<f64>,
    #[prop(optional, into)] disabled: Signal<bool>,
    /// Unit shown after the number, e.g. "s".
    #[prop(optional, into)]
    suffix: String,
    /// Accessible name for the text field.
    #[prop(optional, into)]
    label: String,
    #[prop(optional, into)] class: String,
) -> impl IntoView {
    let step = step.unwrap_or(1.0);
    let label_attr = (!label.is_empty()).then_some(label);
    let emit = move |v: f64| {
        let v = snap(v, min, max, step);
        if let Some(cb) = on_change {
            cb.run(v);
        }
    };
    let nudge = move |dir: f64| {
        if disabled.get_untracked() {
            return;
        }
        emit(value.get_untracked() + dir * step);
    };
    let at_min = Signal::derive(move || min.is_some_and(|lo| value.get() <= lo));
    let at_max = Signal::derive(move || max.is_some_and(|hi| value.get() >= hi));
    let on_commit = move |ev: web_sys::Event| {
        if let Some(input) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        {
            match input.value().trim().parse::<f64>() {
                Ok(v) => emit(v),
                // Reject garbage: put the current value back.
                Err(_) => input.set_value(&format_value(value.get_untracked(), step)),
            }
        }
    };
    let on_key = move |ev: leptos::ev::KeyboardEvent| match ev.key().as_str() {
        "ArrowUp" => {
            ev.prevent_default();
            nudge(1.0);
        }
        "ArrowDown" => {
            ev.prevent_default();
            nudge(-1.0);
        }
        _ => {}
    };
    let side_btn = "flex w-8 shrink-0 items-center justify-center text-rambutan-400 transition-colors hover:bg-white/5 hover:text-rambutan-100 disabled:cursor-not-allowed disabled:opacity-40";

    view! {
        <div class=move || format!(
            "inline-flex h-8 items-stretch overflow-hidden rounded-grape bg-white/5 inset-ring inset-ring-white/10 focus-within:outline-2 focus-within:-outline-offset-1 focus-within:outline-persimmon-400 {} {class}",
            if disabled.get() { "opacity-60" } else { "" }
        )>
            <button
                type="button"
                aria-label="Decrease"
                class=side_btn
                disabled=move || disabled.get() || at_min.get()
                on:click=move |_| nudge(-1.0)
            >
                <Icon name="minus" size=12 />
            </button>
            <input
                type="text"
                inputmode="decimal"
                aria-label=label_attr
                class="w-12 min-w-0 bg-transparent text-center text-body-sm tabular-nums text-rambutan-100 focus:outline-none disabled:cursor-not-allowed"
                disabled=move || disabled.get()
                prop:value=move || format_value(value.get(), step)
                on:change=on_commit
                on:keydown=on_key
            />
            {(!suffix.is_empty()).then(|| view! {
                <span class="flex items-center pr-1 text-label-sm text-rambutan-500">{suffix.clone()}</span>
            })}
            <button
                type="button"
                aria-label="Increase"
                class=side_btn
                disabled=move || disabled.get() || at_max.get()
                on:click=move |_| nudge(1.0)
            >
                <Icon name="plus" size=12 />
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_rounds_to_grid_and_clamps() {
        assert_eq!(snap(7.4, Some(0.0), None, 1.0), 7.0);
        assert_eq!(snap(7.6, Some(0.0), None, 1.0), 8.0);
        assert_eq!(snap(-3.0, Some(0.0), None, 1.0), 0.0);
        assert_eq!(snap(99.0, Some(2.0), Some(20.0), 1.0), 20.0);
        assert_eq!(snap(0.73, Some(0.5), Some(0.95), 0.05), 0.75);
        assert_eq!(snap(5.0, None, None, 0.0), 5.0);
    }

    #[test]
    fn format_follows_step_grid() {
        assert_eq!(format_value(60.0, 1.0), "60");
        assert_eq!(format_value(0.75, 0.05), "0.75");
    }
}
