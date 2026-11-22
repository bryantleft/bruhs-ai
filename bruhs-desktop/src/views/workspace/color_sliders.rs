//! Shared pointer and keyboard controls for editing an RGB color.
use super::*;
use gpui::{linear_color_stop, linear_gradient, relative, rgb, Bounds, Pixels, Point};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy)]
pub(super) struct Hsv {
    pub(super) hue: f32,
    saturation: f32,
    pub(super) value: f32,
}

impl Hsv {
    pub(super) fn from_rgb(color: u32, previous_hue: f32) -> Self {
        let color = rgb(color);
        let value = color.r.max(color.g).max(color.b);
        let chroma = value - color.r.min(color.g).min(color.b);
        Self {
            hue: if chroma == 0. {
                previous_hue
            } else {
                gpui::Hsla::from(color).h
            },
            saturation: if value == 0. { 0. } else { chroma / value },
            value,
        }
    }

    pub(super) fn rgb(self) -> u32 {
        let lightness = self.value * (1. - self.saturation / 2.);
        let saturation = if lightness == 0. || lightness == 1. {
            0.
        } else {
            (self.value - lightness) / lightness.min(1. - lightness)
        };
        let c = gpui::hsla(self.hue % 1., saturation.clamp(0., 1.), lightness, 1.).to_rgb();
        let channel = |v: f32| (v * 255.).round() as u32;
        channel(c.r) << 16 | channel(c.g) << 8 | channel(c.b)
    }
}

pub(super) struct Sliders {
    pub(super) hsv: Hsv,
    pub(super) dragging_hue: Option<bool>,
    pub(super) saturation_focus: FocusHandle,
    pub(super) brightness_focus: FocusHandle,
    pub(super) hue_focus: FocusHandle,
}

impl Sliders {
    pub(super) fn new(color: u32, cx: &App) -> Self {
        Self {
            hsv: Hsv::from_rgb(color, 0.),
            dragging_hue: None,
            saturation_focus: cx.focus_handle(),
            brightness_focus: cx.focus_handle(),
            hue_focus: cx.focus_handle(),
        }
    }
}

pub(super) trait SliderHost: Sized + 'static {
    fn sliders(&mut self) -> Option<&mut Sliders>;
    fn apply_slider_color(&mut self, w: &mut Window, cx: &mut Context<'_, Self>);
}
pub(super) fn color_surface<T: SliderHost>(
    picker: &Sliders,
    hue: bool,
    cx: &Context<'_, T>,
) -> Div {
    let hsv = picker.hsv;
    let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
    let measured = bounds.clone();
    let apply = move |ws: &mut T, pos: Point<Pixels>, w: &mut Window, cx: &mut Context<'_, T>| {
        let Some(sliders) = ws.sliders() else {
            return;
        };
        let b = bounds.get();
        if b.size.width <= px(0.) || b.size.height <= px(0.) {
            return;
        }
        let x = ((pos.x - b.left()) / b.size.width).clamp(0., 1.);
        if hue {
            sliders.hsv.hue = x;
        } else {
            sliders.hsv.saturation = x;
            sliders.hsv.value = 1. - ((pos.y - b.top()) / b.size.height).clamp(0., 1.);
        }
        ws.apply_slider_color(w, cx);
        cx.notify();
    };
    let press = apply.clone();
    let moving = cx.listener(move |ws, event: &MouseMoveEvent, w, cx| {
        if ws.sliders().is_some_and(|s| s.dragging_hue == Some(hue)) {
            if event.pressed_button == Some(MouseButton::Left) {
                apply(ws, event.position, w, cx);
            } else if let Some(sliders) = ws.sliders() {
                sliders.dragging_hue = None;
            }
        }
    });
    let release = cx.listener(|ws, event: &MouseUpEvent, _, _| {
        if event.button == MouseButton::Left {
            if let Some(sliders) = ws.sliders() {
                sliders.dragging_hue = None;
            }
        }
    });
    let gradients = if hue {
        let mut strip = div().flex().size_full();
        for index in 0..6 {
            let stop = |n: u8| gpui::hsla(f32::from(n % 6) / 6., 1., 0.5, 1.);
            strip = strip.child(div().flex_1().h_full().bg(linear_gradient(
                90.,
                linear_color_stop(stop(index), 0.),
                linear_color_stop(stop(index + 1), 1.),
            )));
        }
        strip
    } else {
        div()
            .size_full()
            .bg(linear_gradient(
                90.,
                linear_color_stop(gpui::white(), 0.),
                linear_color_stop(gpui::hsla(hsv.hue % 1., 1., 0.5, 1.), 1.),
            ))
            .child(div().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(gpui::black().opacity(0.), 0.),
                linear_color_stop(gpui::black(), 1.),
            )))
    };
    let id = if hue {
        "color-picker-hue"
    } else {
        "color-picker-field"
    };
    let x = if hue { hsv.hue } else { hsv.saturation };
    let y = if hue { 0.5 } else { 1. - hsv.value };
    div()
        .relative()
        .w_full()
        .h(px(if hue { 16. } else { 136. }))
        .rounded_md()
        .overflow_hidden()
        .on_children_prepainted(move |children, _, _| {
            if let Some(b) = children.first() {
                measured.set(*b);
            }
        })
        .child(
            div()
                .id(id)
                .debug_selector(move || id.into())
                .size_full()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |ws, e: &MouseDownEvent, w, cx| {
                        if let Some(sliders) = ws.sliders() {
                            sliders.dragging_hue = Some(hue);
                            let focus = if hue {
                                &sliders.hue_focus
                            } else {
                                &sliders.saturation_focus
                            };
                            focus.focus(w, cx);
                        }
                        press(ws, e.position, w, cx);
                    }),
                )
                .child(gradients)
                .when(hue, |el| {
                    el.child(keyboard_slider(
                        Channel::Hue,
                        hsv.hue,
                        &picker.hue_focus,
                        cx,
                    ))
                })
                .when(!hue, |el| {
                    el.child(keyboard_slider(
                        Channel::Saturation,
                        hsv.saturation,
                        &picker.saturation_focus,
                        cx,
                    ))
                    .child(keyboard_slider(
                        Channel::Brightness,
                        hsv.value,
                        &picker.brightness_focus,
                        cx,
                    ))
                }),
        )
        .child(
            gpui::canvas(
                |_, _, _| (),
                move |_, _, w, _| {
                    w.on_mouse_event(move |event: &MouseMoveEvent, phase, w, cx| {
                        if phase.bubble() {
                            moving(event, w, cx);
                        }
                    });
                    w.on_mouse_event(move |event: &MouseUpEvent, phase, w, cx| {
                        if phase.bubble() {
                            release(event, w, cx);
                        }
                    });
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .left(relative(x))
                .top(relative(y))
                .ml(px(-5.))
                .mt(px(-5.))
                .size(px(10.))
                .rounded_full()
                .border_2()
                .border_color(gpui::white())
                .shadow_sm(),
        )
}

#[derive(Clone, Copy)]
enum Channel {
    Hue,
    Saturation,
    Brightness,
}

fn keyboard_slider<T: SliderHost>(
    channel: Channel,
    value: f32,
    focus: &FocusHandle,
    cx: &Context<'_, T>,
) -> impl IntoElement {
    let (label, unit, max, step, large_step) = match channel {
        Channel::Hue => ("Color hue", "°", 360., 1., 10.),
        Channel::Saturation => ("Color saturation", "%", 100., 2., 10.),
        Channel::Brightness => ("Color brightness", "%", 100., 2., 10.),
    };
    div()
        .id(label)
        .track_focus(focus)
        .absolute()
        .inset_0()
        .tab_index(0)
        .role(Role::Slider)
        .aria_label(label)
        .aria_description("Use arrow keys to adjust. Hold Shift for larger steps. Home and End select the minimum and maximum.")
        .aria_min_numeric_value(0.)
        .aria_max_numeric_value(f64::from(max))
        .aria_numeric_value(f64::from(value * max))
        .aria_numeric_value_step(f64::from(step))
        .aria_value(format!("{:.0}{unit}", value * max))
        .focus_visible(|s| s.border_2().border_color(theme::rambutan_100()))
        .on_key_down(cx.listener(move |ws, event: &KeyDownEvent, w, cx| {
            let Some(sliders) = ws.sliders() else {
                return;
            };
            let axis = match channel {
                Channel::Hue => &mut sliders.hsv.hue,
                Channel::Saturation => &mut sliders.hsv.saturation,
                Channel::Brightness => &mut sliders.hsv.value,
            };
            let delta = if event.keystroke.modifiers.shift {
                large_step
            } else {
                step
            } / max;
            *axis = match event.keystroke.key.as_str() {
                "home" => 0.,
                "end" => 1.,
                "left" | "down" => (*axis - delta).max(0.),
                "right" | "up" => (*axis + delta).min(1.),
                _ => return,
            };
            ws.apply_slider_color(w, cx);
            w.prevent_default();
            cx.stop_propagation();
            cx.notify();
        }))
}
