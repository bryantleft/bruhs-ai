use crate::theme;
use gpui::prelude::*;
use gpui::*;
use gpui_component::zoom::px;

const WEB_BUTTON_FONT_FAMILY: &str = "Monaspace Argon";
const WEB_BUTTON_GAP_PX: f32 = 6.0;
const WEB_BUTTON_RADIUS_PX: f32 = 8.0;
const WEB_BUTTON_LINE_HEIGHT: f32 = 1.5;
const WEB_BUTTON_DISABLED_OPACITY: f32 = 0.6;
const WEB_BUTTON_DEFAULT_MIN_HEIGHT_PX: f32 = theme::APP_CONTROL_HEIGHT_PX;
/// shadcn Button `sm`: `px-3`.
const WEB_BUTTON_DEFAULT_X_PADDING_PX: f32 = 12.0;
const WEB_BUTTON_DEFAULT_Y_PADDING_PX: f32 = 5.0;
const WEB_BUTTON_DEFAULT_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const WEB_BUTTON_SMALL_MIN_HEIGHT_PX: f32 = theme::APP_COMPACT_CONTROL_HEIGHT_PX;
const WEB_BUTTON_SMALL_X_PADDING_PX: f32 = 8.0;
const WEB_BUTTON_SMALL_Y_PADDING_PX: f32 = 4.0;
const WEB_BUTTON_SMALL_TEXT_SIZE_PX: f32 = 12.0;
const WEB_BUTTON_ICON_DEFAULT_MIN_SIZE_PX: f32 = theme::APP_CONTROL_HEIGHT_PX;
const WEB_BUTTON_ICON_DEFAULT_PADDING_PX: f32 = 8.0;
const WEB_BUTTON_ICON_DEFAULT_TEXT_SIZE_PX: f32 = 14.0;
const WEB_BUTTON_ICON_SMALL_MIN_SIZE_PX: f32 = theme::APP_COMPACT_CONTROL_HEIGHT_PX;
const WEB_BUTTON_ICON_SMALL_PADDING_PX: f32 = 6.0;
const WEB_BUTTON_ICON_SMALL_TEXT_SIZE_PX: f32 = 12.0;
const WEB_BUTTON_OUTLINE_RING_OPACITY: f32 = 0.15;

/// Enter and Space press a focused button, like a native `<button>`.
pub fn is_web_button_activation_key(key: &str) -> bool {
    matches!(key, "enter" | "space")
}

/// Run `action` on a click, or on Enter or Space while the control has
/// focus, once per press. gpui also clicks a focused element on key up, so
/// keyboard clicks are ignored here; the key down already acted.
pub(crate) fn on_press<T: 'static>(
    el: Stateful<Div>,
    cx: &Context<'_, T>,
    action: impl Fn(&mut T, &mut Window, &mut Context<'_, T>) + 'static,
) -> Stateful<Div> {
    let action = std::rc::Rc::new(action);
    let key = std::rc::Rc::clone(&action);
    el.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
        if !event.is_keyboard() {
            cx.stop_propagation();
            action(this, window, cx);
        }
    }))
    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
        if is_web_button_activation_key(event.keystroke.key.as_str()) {
            window.prevent_default();
            cx.stop_propagation();
            key(this, window, cx);
        }
    }))
}

/// Whether a button takes clicks, focus and hover styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Enabled,
    Disabled,
}

impl ButtonState {
    pub fn disabled_if(disabled: bool) -> Self {
        if disabled {
            Self::Disabled
        } else {
            Self::Enabled
        }
    }
}

#[derive(Clone, Copy)]
pub enum WebButtonSize {
    Default,
    Small,
    IconDefault,
    IconSmall,
}

#[derive(Clone, Copy)]
pub enum WebButtonVariant {
    Primary,
    Ghost,
    Outline,
    Destructive,
}

pub fn web_button(
    id: impl Into<ElementId>,
    variant: WebButtonVariant,
    size: WebButtonSize,
    state: ButtonState,
) -> Stateful<Div> {
    let disabled = state == ButtonState::Disabled;
    div()
        .id(id)
        .role(Role::Button)
        .when(!disabled, |el| el.tab_index(0))
        .border_1()
        .border_color(gpui::transparent_black())
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(WEB_BUTTON_GAP_PX))
        .font_family(WEB_BUTTON_FONT_FAMILY)
        .font_weight(FontWeight::MEDIUM)
        .rounded(px(WEB_BUTTON_RADIUS_PX))
        .line_height(relative(WEB_BUTTON_LINE_HEIGHT))
        .when(matches!(size, WebButtonSize::Default), |el| {
            el.min_h(px(WEB_BUTTON_DEFAULT_MIN_HEIGHT_PX))
                .px(px(WEB_BUTTON_DEFAULT_X_PADDING_PX))
                .py(px(WEB_BUTTON_DEFAULT_Y_PADDING_PX))
                .text_size(px(WEB_BUTTON_DEFAULT_TEXT_SIZE_PX))
        })
        .when(matches!(size, WebButtonSize::Small), |el| {
            el.min_h(px(WEB_BUTTON_SMALL_MIN_HEIGHT_PX))
                .px(px(WEB_BUTTON_SMALL_X_PADDING_PX))
                .py(px(WEB_BUTTON_SMALL_Y_PADDING_PX))
                .text_size(px(WEB_BUTTON_SMALL_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
        })
        .when(matches!(size, WebButtonSize::IconDefault), |el| {
            el.min_w(px(WEB_BUTTON_ICON_DEFAULT_MIN_SIZE_PX))
                .min_h(px(WEB_BUTTON_ICON_DEFAULT_MIN_SIZE_PX))
                .p(px(WEB_BUTTON_ICON_DEFAULT_PADDING_PX))
                .text_size(px(WEB_BUTTON_ICON_DEFAULT_TEXT_SIZE_PX))
        })
        .when(matches!(size, WebButtonSize::IconSmall), |el| {
            el.min_w(px(WEB_BUTTON_ICON_SMALL_MIN_SIZE_PX))
                .min_h(px(WEB_BUTTON_ICON_SMALL_MIN_SIZE_PX))
                .p(px(WEB_BUTTON_ICON_SMALL_PADDING_PX))
                .text_size(px(WEB_BUTTON_ICON_SMALL_TEXT_SIZE_PX))
                .font_family(theme::UI_SMALL_FONT_FAMILY)
        })
        .map(|el| match variant {
            WebButtonVariant::Primary => el
                .border_1()
                .border_color(theme::persimmon_500())
                .bg(theme::persimmon_500())
                .text_color(theme::longan_950())
                .when(!disabled, |el| {
                    el.hover(|el| el.bg(theme::persimmon_400()))
                        .active(|el| el.bg(theme::persimmon_600()))
                }),
            WebButtonVariant::Ghost => el.text_color(theme::rambutan_200()).when(!disabled, |el| {
                el.hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
            }),
            WebButtonVariant::Outline => el
                .border_1()
                .border_color(theme::rambutan_100().opacity(WEB_BUTTON_OUTLINE_RING_OPACITY))
                .bg(hsla(0.0, 0.0, 0.0, 0.0))
                .text_color(theme::rambutan_100())
                .when(!disabled, |el| el.hover(|el| el.bg(theme::longan_800()))),
            WebButtonVariant::Destructive => el
                .border_1()
                .border_color(theme::lychee_600())
                .bg(theme::lychee_600())
                .text_color(theme::lychee_50())
                .when(!disabled, |el| {
                    el.hover(|el| el.bg(theme::lychee_500()))
                        .active(|el| el.bg(theme::lychee_600()))
                }),
        })
        .when(disabled, |el| {
            el.opacity(WEB_BUTTON_DISABLED_OPACITY).cursor_not_allowed()
        })
        .when(!disabled, |el| el.cursor_pointer())
}

#[cfg(test)]
mod tests {
    use gpui::{
        div, px, Context, FocusHandle, InteractiveElement, IntoElement, Modifiers, MouseButton,
        ParentElement, Render, Styled, TestAppContext, Window,
    };

    use super::{
        is_web_button_activation_key, WEB_BUTTON_DEFAULT_MIN_HEIGHT_PX,
        WEB_BUTTON_DEFAULT_TEXT_SIZE_PX, WEB_BUTTON_DEFAULT_X_PADDING_PX,
        WEB_BUTTON_DEFAULT_Y_PADDING_PX, WEB_BUTTON_DISABLED_OPACITY, WEB_BUTTON_FONT_FAMILY,
        WEB_BUTTON_GAP_PX, WEB_BUTTON_ICON_DEFAULT_MIN_SIZE_PX, WEB_BUTTON_ICON_DEFAULT_PADDING_PX,
        WEB_BUTTON_ICON_DEFAULT_TEXT_SIZE_PX, WEB_BUTTON_ICON_SMALL_MIN_SIZE_PX,
        WEB_BUTTON_ICON_SMALL_PADDING_PX, WEB_BUTTON_ICON_SMALL_TEXT_SIZE_PX,
        WEB_BUTTON_LINE_HEIGHT, WEB_BUTTON_OUTLINE_RING_OPACITY, WEB_BUTTON_RADIUS_PX,
        WEB_BUTTON_SMALL_MIN_HEIGHT_PX, WEB_BUTTON_SMALL_TEXT_SIZE_PX,
        WEB_BUTTON_SMALL_X_PADDING_PX, WEB_BUTTON_SMALL_Y_PADDING_PX,
    };

    #[test]
    fn activation_keys_should_match_web_button_keyboard_behavior() {
        assert!(is_web_button_activation_key("enter"));
        assert!(is_web_button_activation_key("space"));
        assert!(!is_web_button_activation_key("escape"));
    }

    struct ButtonHarness {
        focus: FocusHandle,
    }

    impl Render for ButtonHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().flex().items_start().child(
                super::web_button(
                    "alignment-button",
                    super::WebButtonVariant::Ghost,
                    super::WebButtonSize::Default,
                    super::ButtonState::Enabled,
                )
                .track_focus(&self.focus)
                .debug_selector(|| "alignment-button".to_string())
                .child(
                    div()
                        .debug_selector(|| "alignment-label".to_string())
                        .child("Open"),
                ),
            )
        }
    }

    #[gpui::test]
    fn button_label_stays_centered_without_hover_or_focus_layout_shift(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, cx| ButtonHarness {
            focus: cx.focus_handle(),
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("alignment-button").expect("button");
        let label = cx.debug_bounds("alignment-label").expect("label");
        assert!((f32::from(label.center().x - bounds.center().x)).abs() <= 0.5);
        assert!((f32::from(label.center().y - bounds.center().y)).abs() <= 0.5);

        cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("alignment-button"), Some(bounds));
        assert_eq!(cx.debug_bounds("alignment-label"), Some(label));

        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("alignment-button"), Some(bounds));
        cx.simulate_mouse_up(bounds.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_keystrokes("right");
        view.update_in(cx, |view, window, cx| view.focus.focus(window, cx));
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("alignment-button"), Some(bounds));
        assert_eq!(cx.debug_bounds("alignment-label"), Some(label));
        assert!(bounds.size.height >= px(32.0));
    }

    #[test]
    fn desktop_web_button_metrics_should_match_web_reference() {
        assert_eq!(WEB_BUTTON_FONT_FAMILY, "Monaspace Argon");
        assert_eq!(WEB_BUTTON_GAP_PX, 6.0);
        assert_eq!(WEB_BUTTON_RADIUS_PX, 8.0);
        assert_eq!(WEB_BUTTON_LINE_HEIGHT, 1.5);
        assert_eq!(WEB_BUTTON_DISABLED_OPACITY, 0.6);
        assert_eq!(WEB_BUTTON_DEFAULT_MIN_HEIGHT_PX, 32.0);
        assert_eq!(WEB_BUTTON_DEFAULT_X_PADDING_PX, 12.0);
        assert_eq!(WEB_BUTTON_DEFAULT_Y_PADDING_PX, 5.0);
        assert_eq!(WEB_BUTTON_DEFAULT_TEXT_SIZE_PX, 13.0);
        assert_eq!(WEB_BUTTON_SMALL_MIN_HEIGHT_PX, 28.0);
        assert_eq!(WEB_BUTTON_SMALL_X_PADDING_PX, 8.0);
        assert_eq!(WEB_BUTTON_SMALL_Y_PADDING_PX, 4.0);
        assert_eq!(WEB_BUTTON_SMALL_TEXT_SIZE_PX, 12.0);
        assert_eq!(WEB_BUTTON_ICON_DEFAULT_MIN_SIZE_PX, 32.0);
        assert_eq!(WEB_BUTTON_ICON_DEFAULT_PADDING_PX, 8.0);
        assert_eq!(WEB_BUTTON_ICON_DEFAULT_TEXT_SIZE_PX, 14.0);
        assert_eq!(WEB_BUTTON_ICON_SMALL_MIN_SIZE_PX, 28.0);
        assert_eq!(WEB_BUTTON_ICON_SMALL_PADDING_PX, 6.0);
        assert_eq!(WEB_BUTTON_ICON_SMALL_TEXT_SIZE_PX, 12.0);
        assert_eq!(WEB_BUTTON_OUTLINE_RING_OPACITY, 0.15);
    }
}
