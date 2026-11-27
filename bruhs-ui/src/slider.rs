//! Range slider primitive with keyboard, pointer, and AccessKit semantics.
//!
//! Draws the Bruhs range track (`longan-700` rail with semantic ring,
//! `persimmon-500` fill up to the thumb center, and `rambutan-100` thumb) and
//! exposes the shared snapping and coordinate math used across desktop views.

use crate::theme;
use crate::zoom::px;
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use gpui::prelude::*;
use gpui::{
    canvas, div, relative, rgba, App, Bounds, FocusHandle, KeyDownEvent, MouseButton,
    MouseDownEvent, Pixels, Role, SharedString, StyleRefinement, Window,
};
use std::cell::Cell;
use std::rc::Rc;

/// Interactive row height matching the web `h-5` range control.
pub const SLIDER_HEIGHT_PX: f32 = 20.0;
/// Track height matching the web `h-1.5` rail.
pub const SLIDER_TRACK_PX: f32 = 6.0;
/// Thumb diameter matching the web `1rem` range thumb.
pub const SLIDER_THUMB_PX: f32 = 16.0;

/// Callback invoked when the user changes a [`Slider`] value.
pub type SliderChangeHandler = Rc<dyn Fn(f64, &mut Window, &mut App)>;
/// Callback invoked on left mouse-down over the slider track with the pointer x position.
pub type SliderDragStartHandler = Rc<dyn Fn(Pixels, &mut Window, &mut App)>;

/// Snap `v` to the step grid anchored at `min` (or `0.0`) and clamp to `[min, max]`.
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
    if step.fract() == 0.0 {
        out = out.round();
    }
    if out == 0.0 {
        0.0
    } else {
        out
    }
}

/// Format a snapped slider or stepper value: integer grids print without decimals.
pub fn format_value(v: f64, step: f64) -> String {
    if step.fract() == 0.0 {
        format!("{}", v.round().to_i64_saturating())
    } else {
        format!("{v:.2}")
    }
}

/// Fractional position (`0.0..=1.0`) of `value` within `[min, max]`.
pub fn slider_fraction(value: f64, min: f64, max: f64) -> f64 {
    let span = max - min;
    if span > 0.0 {
        ((value - min) / span).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Snapped value under a pointer `x` px into a track `width` px wide.
///
/// The thumb travels the inner `width - SLIDER_THUMB_PX` span, matching a
/// native range input.
pub fn slider_value_at(x: f32, width: f32, min: f64, max: f64, step: f64) -> f64 {
    let inner = (width - SLIDER_THUMB_PX).max(1.0);
    let fraction = f64::from(((x - SLIDER_THUMB_PX / 2.0) / inner).clamp(0.0, 1.0));
    snap(min + fraction * (max - min), Some(min), Some(max), step)
}

fn focus_ring(style: StyleRefinement) -> StyleRefinement {
    style.border_color(theme::persimmon_400())
}

/// Accessible single-thumb range slider.
#[derive(IntoElement)]
pub struct Slider {
    id: SharedString,
    aria_label: SharedString,
    debug_selector: Option<SharedString>,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    large_step: Option<f64>,
    disabled: bool,
    focus_handle: Option<FocusHandle>,
    bounds: Option<Rc<Cell<Option<Bounds<Pixels>>>>>,
    on_change: Option<SliderChangeHandler>,
    on_drag_start: Option<SliderDragStartHandler>,
}

impl Slider {
    pub fn new(
        id: impl Into<SharedString>,
        aria_label: impl Into<SharedString>,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
    ) -> Self {
        let id = id.into();
        Self {
            debug_selector: Some(id.clone()),
            id,
            aria_label: aria_label.into(),
            value,
            min,
            max,
            step,
            large_step: None,
            disabled: false,
            focus_handle: None,
            bounds: None,
            on_change: None,
            on_drag_start: None,
        }
    }

    pub fn debug_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.debug_selector = Some(selector.into());
        self
    }

    pub fn large_step(mut self, step: f64) -> Self {
        self.large_step = Some(step);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Supply an explicit focus handle instead of an internal keyed handle.
    pub fn focus_handle(mut self, handle: FocusHandle) -> Self {
        self.focus_handle = Some(handle);
        self
    }

    /// Share the rendered track bounds cell with a caller that tracks window-level drags.
    pub fn bounds(mut self, cell: Rc<Cell<Option<Bounds<Pixels>>>>) -> Self {
        self.bounds = Some(cell);
        self
    }

    pub fn on_change(mut self, handler: SliderChangeHandler) -> Self {
        self.on_change = Some(handler);
        self
    }

    /// Override left mouse-down handling for callers that maintain cross-window drag state.
    pub fn on_drag_start(mut self, handler: SliderDragStartHandler) -> Self {
        self.on_drag_start = Some(handler);
        self
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            aria_label,
            debug_selector,
            value,
            min,
            max,
            step,
            large_step,
            disabled,
            focus_handle,
            bounds,
            on_change,
            on_drag_start,
        } = self;

        let handle = focus_handle.unwrap_or_else(|| {
            window
                .use_keyed_state(
                    SharedString::from(format!("bruhs-slider-focus-{id}")),
                    cx,
                    |_, cx| cx.focus_handle().tab_stop(true),
                )
                .read(cx)
                .clone()
        });
        let focus_on_down = handle.clone();
        let bounds_cell = bounds.unwrap_or_else(|| Rc::new(Cell::new(None)));
        let track_bounds_for_paint = bounds_cell.clone();
        let fraction = slider_fraction(value, min, max).to_f32_lossy();
        let track_top = (SLIDER_HEIGHT_PX - SLIDER_TRACK_PX) / 2.0 - 1.0;
        let thumb_top = (SLIDER_HEIGHT_PX - SLIDER_THUMB_PX) / 2.0 - 1.0;
        let half_thumb = SLIDER_THUMB_PX / 2.0;

        let mut el = div()
            .id(id)
            .when_some(debug_selector, |el, selector| {
                el.debug_selector(move || selector.to_string())
            })
            .relative()
            .w_full()
            .h(px(SLIDER_HEIGHT_PX))
            .border_1()
            .border_color(gpui::transparent_black())
            .role(Role::Slider)
            .aria_label(aria_label)
            .aria_numeric_value(value)
            .aria_min_numeric_value(min)
            .aria_max_numeric_value(max)
            .aria_numeric_value_step(step)
            .when(disabled, |el| el.opacity(0.5).cursor_not_allowed())
            .when(!disabled, |el| {
                el.cursor_pointer()
                    .tab_index(0)
                    .track_focus(&handle)
                    .focus_visible(focus_ring)
            })
            .child(
                canvas(
                    move |bounds, _window, _cx| track_bounds_for_paint.set(Some(bounds)),
                    |_bounds, _state, _window, _cx| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(track_top))
                    .h(px(SLIDER_TRACK_PX))
                    .rounded_full()
                    .bg(theme::longan_700())
                    .border_1()
                    .border_color(theme::ring_card()),
            )
            .child(
                // Fill runs from the left edge of the track to the thumb center:
                // a fixed half-thumb cap plus a share of the inner travel.
                div()
                    .absolute()
                    .left(px(half_thumb))
                    .right(px(half_thumb))
                    .top(px(track_top))
                    .h(px(SLIDER_TRACK_PX))
                    .child(
                        div()
                            .absolute()
                            .left(px(-half_thumb))
                            .top_0()
                            .w(px(half_thumb))
                            .h_full()
                            .rounded_tl(px(SLIDER_TRACK_PX))
                            .rounded_bl(px(SLIDER_TRACK_PX))
                            .bg(theme::persimmon_500()),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .w(relative(fraction))
                            .h_full()
                            .bg(theme::persimmon_500()),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .left(px(half_thumb))
                    .right(px(half_thumb))
                    .top(px(thumb_top))
                    .h(px(SLIDER_THUMB_PX))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(relative(fraction))
                            .ml(px(-half_thumb))
                            .size(px(SLIDER_THUMB_PX))
                            .rounded_full()
                            .bg(theme::rambutan_100())
                            .border_1()
                            .border_color(rgba(0x14121099)),
                    ),
            );

        if !disabled {
            if let Some(drag_start) = on_drag_start {
                el = el.on_mouse_down(
                    MouseButton::Left,
                    move |event: &MouseDownEvent, window, cx| {
                        focus_on_down.focus(window, cx);
                        cx.stop_propagation();
                        drag_start(event.position.x, window, cx);
                    },
                );
            } else if let Some(change) = on_change.clone() {
                el = el.on_mouse_down(
                    MouseButton::Left,
                    move |event: &MouseDownEvent, window, cx| {
                        focus_on_down.focus(window, cx);
                        cx.stop_propagation();
                        if let Some(bounds) = bounds_cell.get() {
                            let next = slider_value_at(
                                f32::from(event.position.x - bounds.left()),
                                f32::from(bounds.size.width),
                                min,
                                max,
                                step,
                            );
                            change(next, window, cx);
                        }
                    },
                );
            }

            if let Some(change) = on_change {
                el = el.on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let delta = if event.keystroke.modifiers.shift {
                        large_step.unwrap_or(step)
                    } else {
                        step
                    };
                    let next = match event.keystroke.key.as_str() {
                        "left" | "down" => value - delta,
                        "right" | "up" => value + delta,
                        "home" => min,
                        "end" => max,
                        _ => return,
                    };
                    window.prevent_default();
                    cx.stop_propagation();
                    change(snap(next, Some(min), Some(max), step), window, cx);
                });
            }
        }

        el
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, TestAppContext};

    #[test]
    fn slider_math_snaps_clamps_and_formats() {
        assert_eq!(snap(12.4, Some(0.0), Some(100.0), 5.0), 10.0);
        assert_eq!(snap(12.6, Some(0.0), Some(100.0), 5.0), 15.0);
        assert_eq!(snap(-10.0, Some(0.0), Some(100.0), 1.0), 0.0);
        assert_eq!(snap(150.0, Some(0.0), Some(100.0), 1.0), 100.0);
        assert_eq!(format_value(42.0, 1.0), "42");
        assert_eq!(format_value(0.75, 0.05), "0.75");
        assert!((slider_fraction(50.0, 0.0, 100.0) - 0.5).abs() < 1e-6);
        assert_eq!(slider_value_at(8.0, 216.0, 0.0, 100.0, 1.0), 0.0);
        assert_eq!(slider_value_at(108.0, 216.0, 0.0, 100.0, 1.0), 50.0);
        assert_eq!(slider_value_at(208.0, 216.0, 0.0, 100.0, 1.0), 100.0);
    }

    struct SliderHarness {
        value: f64,
        disabled: bool,
        focus: FocusHandle,
    }

    impl gpui::Render for SliderHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let entity = cx.entity().downgrade();
            div().w(px(216.0)).child(
                Slider::new("test-slider", "Test slider", self.value, 0.0, 100.0, 5.0)
                    .large_step(20.0)
                    .disabled(self.disabled)
                    .focus_handle(self.focus.clone())
                    .on_change(Rc::new(move |next, _, cx| {
                        let _ = entity.update(cx, |this, cx| {
                            this.value = next;
                            cx.notify();
                        });
                    })),
            )
        }
    }

    #[gpui::test]
    fn slider_responds_to_pointer_and_keyboard_and_ignores_when_disabled(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| SliderHarness {
            value: 50.0,
            disabled: false,
            focus: cx.focus_handle().tab_stop(true),
        });
        cx.run_until_parked();

        // Focus and nudge via keyboard.
        view.update_in(cx, |h, window, cx| h.focus.focus(window, cx));
        cx.simulate_keystrokes("right");
        assert_eq!(view.read_with(cx, |h, _| h.value), 55.0);
        cx.simulate_keystrokes("shift-right");
        assert_eq!(view.read_with(cx, |h, _| h.value), 75.0);
        cx.simulate_keystrokes("home");
        assert_eq!(view.read_with(cx, |h, _| h.value), 0.0);
        cx.simulate_keystrokes("end");
        assert_eq!(view.read_with(cx, |h, _| h.value), 100.0);

        // Pointer click at track center sets 50.
        let bounds = cx.debug_bounds("test-slider").expect("slider bounds");
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 50.0);

        // Disabled slider ignores pointer clicks.
        view.update(cx, |h, cx| {
            h.disabled = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_click(
            gpui::point(bounds.left() + px(8.0), bounds.center().y),
            gpui::Modifiers::default(),
        );
        assert_eq!(view.read_with(cx, |h, _| h.value), 50.0);
    }
}
