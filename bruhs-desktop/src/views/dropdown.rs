//! Bruhs-styled dropdown for settings rows: a trigger showing the current
//! label, and — while open — an inline listbox with optional group headers
//! and per-option descriptions. Mirrors bruhs-web's `ui::Dropdown`. Pure
//! element builders: the owning view holds the open state and handlers.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::*;
use gpui_component::scroll::ScrollableElement;

use crate::theme;
use crate::views::web_button::is_web_button_activation_key;

const TRIGGER_HEIGHT_PX: f32 = 32.0;
/// Web trigger: `text-body-sm` (14px Neon); options and groups `text-label-sm`.
const TRIGGER_TEXT_SIZE_PX: f32 = 14.0;
const TRIGGER_RADIUS_PX: f32 = 8.0;
const LIST_MAX_HEIGHT_PX: f32 = 420.0;
const LIST_RADIUS_PX: f32 = 8.0;
const OPTION_TEXT_SIZE_PX: f32 = 12.0;
const OPTION_RADIUS_PX: f32 = 6.0;
const GROUP_TEXT_SIZE_PX: f32 = 12.0;
const CHECK_WIDTH_PX: f32 = 12.0;
const HOVER_BG_OPACITY: f32 = 0.60;

pub struct DropdownOption {
    pub value: &'static str,
    pub label: String,
    pub description: Option<String>,
}

pub struct DropdownGroup {
    /// Uppercase header above the group; `None` for a flat list.
    pub label: Option<&'static str>,
    /// Glyph rendered before the header (provider logo).
    pub mark: Option<AnyElement>,
    pub options: Vec<DropdownOption>,
}

pub type ToggleHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub type SelectHandler = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;

/// Label of `value` in `groups`, for the trigger.
pub fn current_label(groups: &[DropdownGroup], value: &str) -> Option<String> {
    groups
        .iter()
        .flat_map(|g| g.options.iter())
        .find(|o| o.value == value)
        .map(|o| o.label.clone())
}

/// Whether a dropdown's listbox is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listbox {
    Open,
    Closed,
}

impl Listbox {
    pub fn open_if(open: bool) -> Self {
        if open {
            Self::Open
        } else {
            Self::Closed
        }
    }
}

/// Trigger + (when open) listbox stacked inline. `id` doubles as the
/// debug-selector prefix: `{id}` (trigger), `{id}-list`, `{id}-option-{value}`.
pub fn dropdown(
    id: &'static str,
    aria_label: &str,
    value: &str,
    groups: Vec<DropdownGroup>,
    listbox: Listbox,
    on_toggle: ToggleHandler,
    on_select: SelectHandler,
) -> AnyElement {
    let label = current_label(&groups, value).unwrap_or_else(|| value.to_string());
    let open = listbox == Listbox::Open;
    let trigger = dropdown_trigger(id, aria_label, label, listbox, on_toggle);
    if !open {
        return div().w_full().child(trigger).into_any_element();
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(trigger)
        .child(dropdown_list(id, aria_label, value, groups, on_select))
        .into_any_element()
}

/// The closed-state control: current label + chevron.
pub fn dropdown_trigger(
    id: &'static str,
    aria_label: &str,
    label: String,
    listbox: Listbox,
    on_toggle: ToggleHandler,
) -> AnyElement {
    let open = listbox == Listbox::Open;
    let toggle_for_click = on_toggle.clone();
    let toggle_for_key = on_toggle;
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .w_full()
        .h(px(TRIGGER_HEIGHT_PX))
        .px_3()
        .rounded(px(TRIGGER_RADIUS_PX))
        .border_1()
        .border_color(theme::longan_700())
        .bg(theme::longan_900())
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .font_family(theme::CODE_FONT_FAMILY)
        .text_size(px(TRIGGER_TEXT_SIZE_PX))
        .text_color(theme::rambutan_100())
        .tab_index(0)
        .role(Role::ComboBox)
        .aria_label(aria_label.to_string())
        .aria_expanded(open)
        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
        .cursor_pointer()
        .hover(|el| el.border_color(theme::rambutan_500().opacity(0.5)))
        .on_click(move |_ev, window, cx| toggle_for_click(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                window.prevent_default();
                cx.stop_propagation();
                toggle_for_key(window, cx);
            }
        })
        .child(div().min_w_0().truncate().child(label))
        .child(
            div()
                .flex_shrink_0()
                .text_color(theme::rambutan_500())
                .child(if open { "⌃" } else { "⌄" }),
        )
        .into_any_element()
}

/// The open listbox with group headers and options; callers decide where
/// it sits (inline under the trigger, or floated over content).
pub fn dropdown_list(
    id: &'static str,
    aria_label: &str,
    value: &str,
    groups: Vec<DropdownGroup>,
    on_select: SelectHandler,
) -> AnyElement {
    let mut rows: Vec<AnyElement> = Vec::new();
    for group in groups {
        if let Some(header) = group.label {
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(px(GROUP_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_500())
                    .children(group.mark)
                    .child(header.to_ascii_uppercase())
                    .into_any_element(),
            );
        }
        for option in group.options {
            let is_current = option.value == value;
            let option_value = option.value;
            let select_for_click = on_select.clone();
            let select_for_key = on_select.clone();
            let selector = format!("{id}-option-{}", option.value);
            rows.push(
                div()
                    .id(ElementId::Name(
                        format!("{id}-option-{}", option.value).into(),
                    ))
                    .debug_selector(move || selector)
                    .mx_1()
                    .px_2()
                    .py_1p5()
                    .rounded(px(OPTION_RADIUS_PX))
                    .flex()
                    .items_start()
                    .gap_2()
                    .text_size(px(OPTION_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(if is_current {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(option.label.clone())
                    .aria_selected(is_current)
                    .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY)))
                    .on_click(move |_ev, window, cx| select_for_click(option_value, window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            select_for_key(option_value, window, cx);
                        }
                    })
                    .child(
                        div()
                            .w(px(CHECK_WIDTH_PX))
                            .flex_shrink_0()
                            .text_color(theme::persimmon_400())
                            .child(if is_current { "●" } else { "" }),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(option.label))
                            .children(
                                option
                                    .description
                                    .map(|d| div().text_color(theme::rambutan_500()).child(d)),
                            ),
                    )
                    .into_any_element(),
            );
        }
    }

    div()
        .id(ElementId::Name(format!("{id}-list").into()))
        .debug_selector(move || format!("{id}-list"))
        .role(Role::ListBox)
        .aria_label(aria_label.to_string())
        .w_full()
        .max_h(px(LIST_MAX_HEIGHT_PX))
        .overflow_y_scrollbar()
        .rounded(px(LIST_RADIUS_PX))
        .border_1()
        .border_color(theme::ring_floating())
        .bg(theme::longan_700())
        .py_1()
        .children(rows)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // Explicit imports: `super::*` would pull in gpui's `test` macro.
    use super::{current_label, DropdownGroup, DropdownOption};

    #[test]
    fn current_label_finds_across_groups() {
        let groups = vec![
            DropdownGroup {
                label: Some("A"),
                mark: None,
                options: vec![DropdownOption {
                    value: "a1",
                    label: "First".into(),
                    description: None,
                }],
            },
            DropdownGroup {
                label: None,
                mark: None,
                options: vec![DropdownOption {
                    value: "b1",
                    label: "Second".into(),
                    description: Some("desc".into()),
                }],
            },
        ];
        assert_eq!(current_label(&groups, "b1").as_deref(), Some("Second"));
        assert_eq!(current_label(&groups, "a1").as_deref(), Some("First"));
        assert_eq!(current_label(&groups, "zz"), None);
    }
}
