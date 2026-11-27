//! Numeric stepper primitive matching the web `Stepper` control.
//!
//! Renders decrease / editable numeric input / optional suffix / increase in
//! one pill with semantic ring styling and Up/Down keyboard stepping.

use crate::icons;
use crate::input::{Input, InputState};
use crate::press::on_press;
use crate::slider::snap;
use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{
    div, relative, svg, AnyElement, App, Entity, FontWeight, KeyDownEvent, Role, SharedString,
    StyleRefinement, Window,
};
use std::rc::Rc;

/// Overall control height matching the web `h-8` stepper.
pub const STEPPER_HEIGHT_PX: f32 = theme::CONTROL_PX;
/// Width of the decrease and increase side buttons (`w-8`).
pub const STEPPER_SIDE_WIDTH_PX: f32 = theme::CONTROL_PX;
/// Width of the editable numeric field (`w-12`).
pub const STEPPER_VALUE_WIDTH_PX: f32 = 48.0;
const STEPPER_ICON_PX: f32 = theme::ICON_PX;
const STEPPER_DISABLED_OPACITY: f32 = 0.6;
const STEPPER_SIDE_DISABLED_OPACITY: f32 = 0.4;
const STEPPER_RADIUS_PX: f32 = 8.0;

/// Callback invoked with the next clamped/snapped value when stepping via
/// minus/plus buttons or Up/Down keys.
pub type StepHandler = Rc<dyn Fn(f64, &mut Window, &mut App)>;

fn focus_ring(style: StyleRefinement) -> StyleRefinement {
    style.border_color(theme::persimmon_400())
}

fn side_button(
    id: SharedString,
    icon_path: &'static str,
    aria_label: &'static str,
    disabled: bool,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let selector = id.clone();
    let group = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .group(group.clone())
        .w(px(STEPPER_SIDE_WIDTH_PX))
        .h_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .border_1()
        .border_color(gpui::transparent_black())
        .role(Role::Button)
        .aria_label(aria_label)
        .when(disabled, |el| {
            el.opacity(STEPPER_SIDE_DISABLED_OPACITY)
                .cursor_not_allowed()
        })
        .when(!disabled, |el| {
            on_press(
                el.cursor_pointer()
                    .tab_index(0)
                    .focus_visible(focus_ring)
                    .hover(|el| el.bg(theme::control_surface())),
                on_activate,
            )
        })
        .child(
            svg()
                .path(icon_path)
                .size(px(STEPPER_ICON_PX))
                .text_color(theme::rambutan_400())
                .when(!disabled, |el| {
                    el.group_hover(group, |el| el.text_color(theme::rambutan_100()))
                }),
        )
        .into_any_element()
}

/// Numeric stepper with decrease/increase buttons, editable input, and optional unit suffix.
#[derive(IntoElement)]
pub struct Stepper {
    id: SharedString,
    aria_label: SharedString,
    input: Entity<InputState>,
    value: f64,
    min: f64,
    max: f64,
    step: f64,
    suffix: Option<SharedString>,
    disabled: bool,
    on_step: StepHandler,
}

impl Stepper {
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the numeric range spec and input state required by callers"
    )]
    pub fn new(
        id: impl Into<SharedString>,
        aria_label: impl Into<SharedString>,
        input: Entity<InputState>,
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        on_step: StepHandler,
    ) -> Self {
        Self {
            id: id.into(),
            aria_label: aria_label.into(),
            input,
            value,
            min,
            max,
            step,
            suffix: None,
            disabled: false,
            on_step,
        }
    }

    pub fn suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        let suffix = suffix.into();
        if !suffix.is_empty() {
            self.suffix = Some(suffix);
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl RenderOnce for Stepper {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            aria_label,
            input,
            value,
            min,
            max,
            step,
            suffix,
            disabled,
            on_step,
        } = self;

        let at_min = value <= min;
        let at_max = value >= max;
        let selector = id.clone();
        let on_dec = on_step.clone();
        let on_inc = on_step.clone();

        div()
            .id(id.clone())
            .debug_selector(move || selector.to_string())
            .h(px(STEPPER_HEIGHT_PX))
            .flex()
            .items_stretch()
            .overflow_hidden()
            .rounded(px(STEPPER_RADIUS_PX))
            .bg(theme::control_surface())
            .border_1()
            .border_color(theme::ring_card())
            .when(disabled, |el| el.opacity(STEPPER_DISABLED_OPACITY))
            .child(side_button(
                format!("{id}-decrease").into(),
                icons::MINUS,
                "Decrease",
                disabled || at_min,
                move |window, cx| {
                    on_dec(snap(value - step, Some(min), Some(max), step), window, cx);
                },
            ))
            .child(
                div()
                    .id(SharedString::from(format!("{id}-value")))
                    .w(px(STEPPER_VALUE_WIDTH_PX))
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_family(theme::CODE_FONT_FAMILY)
                    .text_size(px(14.0))
                    .text_color(theme::rambutan_100())
                    .when(disabled, |el| el.cursor_not_allowed())
                    .when(!disabled, |el| {
                        el.on_key_down(move |event: &KeyDownEvent, window, cx| {
                            let dir = match event.keystroke.key.as_str() {
                                "up" => 1.0,
                                "down" => -1.0,
                                _ => return,
                            };
                            window.prevent_default();
                            cx.stop_propagation();
                            on_step(
                                snap(value + dir * step, Some(min), Some(max), step),
                                window,
                                cx,
                            );
                        })
                    })
                    .child(
                        Input::new(&input)
                            .appearance(false)
                            .px_0()
                            .disabled(disabled)
                            .text_center()
                            .w_full()
                            .aria_label(aria_label),
                    ),
            )
            .when_some(suffix, |el, text| {
                el.child(
                    div().flex().items_center().pr_1().child(
                        div()
                            .font_family(theme::UI_SMALL_FONT_FAMILY)
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .line_height(relative(1.5))
                            .text_color(theme::rambutan_500())
                            .child(text),
                    ),
                )
            })
            .child(side_button(
                format!("{id}-increase").into(),
                icons::PLUS,
                "Increase",
                disabled || at_max,
                move |window, cx| {
                    on_inc(snap(value + step, Some(min), Some(max), step), window, cx);
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slider::format_value;
    use gpui::{Context, TestAppContext};

    struct StepperHarness {
        value: f64,
        disabled: bool,
        input: Entity<InputState>,
    }

    impl gpui::Render for StepperHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let entity = cx.entity().downgrade();
            Stepper::new(
                "test-stepper",
                "Delay seconds",
                self.input.clone(),
                self.value,
                0.0,
                10.0,
                5.0,
                Rc::new(move |next, window, cx| {
                    let _ = entity.update(cx, |this, cx| {
                        this.value = next;
                        this.input.update(cx, |input, cx| {
                            input.set_value(format_value(next, 5.0), window, cx);
                        });
                        cx.notify();
                    });
                }),
            )
            .suffix("s")
            .disabled(self.disabled)
        }
    }

    #[gpui::test]
    fn stepper_steps_clamps_and_respects_disabled(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value("5"));
            StepperHarness {
                value: 5.0,
                disabled: false,
                input,
            }
        });
        cx.run_until_parked();

        let inc = cx
            .debug_bounds("test-stepper-increase")
            .expect("increase button");
        let dec = cx
            .debug_bounds("test-stepper-decrease")
            .expect("decrease button");

        cx.simulate_click(inc.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 10.0);
        // At max, clicking increase again stays clamped at 10.
        cx.simulate_click(inc.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 10.0);

        cx.simulate_click(dec.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 5.0);
        cx.simulate_click(dec.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 0.0);
        // At min, clicking decrease again stays clamped at 0.
        cx.simulate_click(dec.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 0.0);

        // Disabled stepper ignores clicks.
        view.update(cx, |h, cx| {
            h.value = 5.0;
            h.disabled = true;
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_click(inc.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.value), 5.0);
    }
}
