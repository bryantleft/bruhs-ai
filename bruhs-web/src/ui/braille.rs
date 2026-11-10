//! Dither pieces (Bruhs AGENTS.md section 19): the Bruh drawn as a halftone of
//! dots, and a level meter as one braille cell. Dot math lives in
//! `bruhs_models::logic::dither`, shared with the desktop app.

use bruhs_models::logic::dither::{bruh_dots, meter_column};
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

/// A static, dithered Bruh `size` CSS pixels square. Decorative: hidden from
/// assistive technology, and it never moves.
#[component]
pub fn BrailleBruh(#[prop(default = 96.0)] size: f64) -> impl IntoView {
    let canvas = NodeRef::<html::Canvas>::new();
    Effect::new(move |_| {
        if let Some(canvas) = canvas.get() {
            draw_bruh(&canvas, size);
        }
    });
    let style = format!("width:{size}px;height:{size}px");
    view! { <canvas node_ref=canvas aria-hidden="true" class="shrink-0" style=style></canvas> }
}

fn draw_bruh(canvas: &web_sys::HtmlCanvasElement, size: f64) {
    let scale = web_sys::window()
        .map_or(1.0, |w| w.device_pixel_ratio())
        .clamp(1.0, 2.0);
    let pixels = (size * scale).round().to_u32_saturating();
    canvas.set_width(pixels);
    canvas.set_height(pixels);
    let Some(ctx) = canvas
        .get_context("2d")
        .ok()
        .flatten()
        .and_then(|ctx| ctx.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
    else {
        return;
    };
    // Lay the dots out in CSS pixels and let the transform scale them.
    let _ = ctx.set_transform(scale, 0.0, 0.0, scale, 0.0, 0.0);
    let size_f32 = size.to_f32_lossy();
    let dots = bruh_dots(size_f32);
    // One path per color: the mouth blends face → mouth in five steps.
    let mut colors: Vec<u32> = dots
        .iter()
        .map(bruhs_models::logic::dither::Dot::rgb)
        .collect();
    colors.sort_unstable();
    colors.dedup();
    for rgb in colors {
        ctx.begin_path();
        for dot in dots.iter().filter(|dot| dot.rgb() == rgb) {
            let (x, y, r) = (f64::from(dot.x), f64::from(dot.y), f64::from(dot.radius));
            ctx.move_to(x + r, y);
            let _ = ctx.arc(x, y, r, 0.0, std::f64::consts::TAU);
        }
        ctx.set_fill_style_str(&format!("#{rgb:06x}"));
        ctx.fill();
    }
}

/// Microphone and system audio levels as one braille cell: mic in the left
/// column, system audio in the right (only while it's captured), filling from
/// the bottom. Each column is its own meter for screen readers.
#[component]
pub fn BrailleMeter(level: RwSignal<(f32, Option<f32>)>) -> impl IntoView {
    let column = |label: &'static str, lit_class: &'static str, value: Signal<f32>| {
        view! {
            <span
                role="meter"
                aria-label=label
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow=move || (value.get().clamp(0.0, 1.0) * 100.0).round()
                class="flex flex-col gap-[2px]"
            >
                {(0..4)
                    .map(|index| {
                        view! {
                            <i class=move || {
                                if meter_column(value.get())[index] {
                                    format!("block size-1 rounded-orb {lit_class}")
                                } else {
                                    "block size-1 rounded-orb bg-longan-700".to_string()
                                }
                            }></i>
                        }
                    })
                    .collect_view()}
            </span>
        }
    };
    let mic = Signal::derive(move || level.get().0);
    let has_system = Memo::new(move |_| level.get().1.is_some());
    view! {
        <span data-testid="braille-meter" class="flex shrink-0 gap-[3px]">
            {column("Microphone level", "bg-dragonfruit-500", mic)}
            <Show when=move || has_system.get()>
                {column("System audio level", "bg-pandan-500", Signal::derive(move || level.get().1.unwrap_or(0.0)))}
            </Show>
        </span>
    }
}
