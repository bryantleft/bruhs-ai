//! Bruhs-styled dropdown for settings rows: a trigger showing the current
//! label, and — while open — an inline listbox with optional group headers
//! and per-option descriptions. The owning view holds the value and open
//! state; the element retains focus targets and scroll position across renders.

use gpui_component::zoom::px;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use gpui::prelude::*;
use gpui::*;
use gpui_component::scroll::ScrollableElement;

use crate::views::web_button::is_web_button_activation_key;
use crate::views::workspace::CloseOverlays;
use crate::{assets, theme};

const TRIGGER_HEIGHT_PX: f32 = theme::CONTROL_PX;
const TRIGGER_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
const TRIGGER_RADIUS_PX: f32 = 8.0;
const LIST_MAX_HEIGHT_PX: f32 = 420.0;
const LIST_RADIUS_PX: f32 = 8.0;
const OPTION_TEXT_SIZE_PX: f32 = 12.0;
const OPTION_RADIUS_PX: f32 = 6.0;
const GROUP_TEXT_SIZE_PX: f32 = 12.0;
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
    Dropdown {
        id,
        label: aria_label.to_string().into(),
        value: value.into(),
        groups,
        listbox,
        on_toggle,
        on_select,
    }
    .into_any_element()
}

struct DropdownFocus {
    trigger: FocusHandle,
    options: HashMap<&'static str, FocusHandle>,
    scroll: ScrollHandle,
}

#[derive(Clone)]
struct OptionFocus {
    value: &'static str,
    handle: FocusHandle,
    row: usize,
}

struct ListNavigation {
    targets: Rc<[OptionFocus]>,
    scroll: ScrollHandle,
    tab_stops: bool,
}

fn focus_targets(
    id: &'static str,
    groups: &[DropdownGroup],
    window: &mut Window,
    cx: &mut App,
) -> (FocusHandle, Rc<[OptionFocus]>, ScrollHandle) {
    let state = window.use_keyed_state(SharedString::from(format!("{id}-focus")), cx, |_, cx| {
        DropdownFocus {
            trigger: cx.focus_handle().tab_index(0).tab_stop(true),
            options: HashMap::new(),
            scroll: ScrollHandle::new(),
        }
    });
    state.update(cx, |state, cx| {
        let mut targets = Vec::new();
        let mut current_values = HashSet::new();
        let mut row = 0;
        for group in groups {
            row += usize::from(group.label.is_some());
            for option in &group.options {
                current_values.insert(option.value);
                let handle = state
                    .options
                    .entry(option.value)
                    .or_insert_with(|| cx.focus_handle())
                    .clone();
                targets.push(OptionFocus {
                    value: option.value,
                    handle,
                    row,
                });
                row += 1;
            }
        }
        state
            .options
            .retain(|value, _| current_values.contains(value));
        (state.trigger.clone(), targets.into(), state.scroll.clone())
    })
}

fn navigation_index(index: usize, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let index = index.min(count - 1);
    match key {
        "down" | "arrowdown" | "ArrowDown" => Some((index + 1) % count),
        "up" | "arrowup" | "ArrowUp" => Some(if index == 0 { count - 1 } else { index - 1 }),
        "home" | "Home" => Some(0),
        "end" | "End" => Some(count - 1),
        _ => None,
    }
}

fn focus_option(target: &OptionFocus, scroll: &ScrollHandle, window: &mut Window, cx: &mut App) {
    scroll.scroll_to_item(target.row);
    target.handle.focus(window, cx);
}

#[derive(IntoElement)]
struct Dropdown {
    id: &'static str,
    label: SharedString,
    value: String,
    groups: Vec<DropdownGroup>,
    listbox: Listbox,
    on_toggle: ToggleHandler,
    on_select: SelectHandler,
}

impl RenderOnce for Dropdown {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (trigger_focus, targets, scroll) = focus_targets(self.id, &self.groups, window, cx);
        let label = current_label(&self.groups, &self.value).unwrap_or_else(|| self.value.clone());
        let selected = targets
            .iter()
            .position(|target| target.value == self.value)
            .unwrap_or(0);
        let open = self.listbox == Listbox::Open;
        let toggle_for_key = self.on_toggle.clone();
        let toggle_for_close = self.on_toggle.clone();
        let toggle_for_action = self.on_toggle.clone();
        let close_focus = trigger_focus.clone();
        let action_focus = trigger_focus.clone();
        let selection_focus = trigger_focus.clone();
        let trigger_targets = targets.clone();
        let trigger_scroll = scroll.clone();
        let trigger = dropdown_trigger(self.id, self.label.clone(), label, open, &trigger_focus)
            .on_click(move |_, window, cx| (self.on_toggle)(window, cx))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if event.keystroke.modifiers.modified() {
                    return;
                }
                if is_web_button_activation_key(key) {
                    window.prevent_default();
                    cx.stop_propagation();
                    toggle_for_key(window, cx);
                } else if navigation_index(selected, trigger_targets.len(), key).is_some() {
                    window.prevent_default();
                    cx.stop_propagation();
                    if !open {
                        toggle_for_key(window, cx);
                    }
                    let index = match key {
                        "home" | "Home" => 0,
                        "end" | "End" => trigger_targets.len() - 1,
                        _ => selected,
                    };
                    if let Some(target) = trigger_targets.get(index) {
                        focus_option(target, &trigger_scroll, window, cx);
                    }
                }
            });
        let on_select: SelectHandler = Rc::new(move |value, window, cx| {
            (self.on_select)(value, window, cx);
            selection_focus.focus(window, cx);
        });
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .on_action(move |_: &CloseOverlays, window, cx| {
                if open {
                    cx.stop_propagation();
                    toggle_for_action(window, cx);
                    action_focus.focus(window, cx);
                } else {
                    cx.propagate();
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let modifiers = event.keystroke.modifiers;
                if !open
                    || modifiers.control
                    || modifiers.alt
                    || modifiers.platform
                    || modifiers.function
                {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" => {
                        window.prevent_default();
                        cx.stop_propagation();
                        toggle_for_close(window, cx);
                        close_focus.focus(window, cx);
                    }
                    "tab" => {
                        toggle_for_close(window, cx);
                        close_focus.focus(window, cx);
                    }
                    _ => {}
                }
            })
            .child(trigger)
            .when(open, |el| {
                el.child(render_dropdown_list(
                    self.id,
                    self.label,
                    &self.value,
                    self.groups,
                    on_select,
                    ListNavigation {
                        targets,
                        scroll,
                        tab_stops: false,
                    },
                ))
            })
    }
}

fn dropdown_trigger(
    id: &'static str,
    aria_label: SharedString,
    label: String,
    open: bool,
    focus: &FocusHandle,
) -> Stateful<Div> {
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
        .track_focus(focus)
        .role(Role::ComboBox)
        .aria_label(aria_label)
        .aria_expanded(open)
        .focus_visible(|el| el.border_color(theme::persimmon_400()))
        .cursor_pointer()
        .hover(|el| el.border_color(theme::rambutan_500().opacity(0.5)))
        .child(div().min_w_0().truncate().child(label))
        .child(
            svg()
                .path(assets::CHEVRON_DOWN_ICON_PATH)
                .size(px(theme::ICON_PX))
                .flex_shrink_0()
                .text_color(theme::rambutan_500()),
        )
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
    DropdownList {
        id,
        label: aria_label.to_string().into(),
        value: value.into(),
        groups,
        on_select,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct DropdownList {
    id: &'static str,
    label: SharedString,
    value: String,
    groups: Vec<DropdownGroup>,
    on_select: SelectHandler,
}

impl RenderOnce for DropdownList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (_, targets, scroll) = focus_targets(self.id, &self.groups, window, cx);
        render_dropdown_list(
            self.id,
            self.label,
            &self.value,
            self.groups,
            self.on_select,
            ListNavigation {
                targets,
                scroll,
                tab_stops: true,
            },
        )
    }
}

fn render_dropdown_list(
    id: &'static str,
    aria_label: SharedString,
    value: &str,
    groups: Vec<DropdownGroup>,
    on_select: SelectHandler,
    navigation: ListNavigation,
) -> AnyElement {
    let ListNavigation {
        targets,
        scroll,
        tab_stops,
    } = navigation;
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut option_index = 0;
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
            let check_selector = format!("{selector}-check");
            let index = option_index;
            option_index += 1;
            // a11y-exempt: Configures the focus handle; the row below draws its focus ring.
            let focus = targets[index]
                .handle
                .clone()
                .tab_index(0)
                .tab_stop(tab_stops);
            let key_targets = targets.clone();
            let key_scroll = scroll.clone();
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
                    .items_center()
                    .gap_2()
                    .text_size(px(OPTION_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(if is_current {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_300()
                    })
                    .tab_index(0)
                    .tab_stop(tab_stops)
                    .track_focus(&focus)
                    .role(Role::ListBoxOption)
                    .aria_label(option.label.clone())
                    .aria_selected(is_current)
                    .border_1()
                    .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800().opacity(HOVER_BG_OPACITY)))
                    .on_click(move |_ev, window, cx| select_for_click(option_value, window, cx))
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if event.keystroke.modifiers.modified() {
                            return;
                        }
                        let key = event.keystroke.key.as_str();
                        if is_web_button_activation_key(key) {
                            window.prevent_default();
                            cx.stop_propagation();
                            select_for_key(option_value, window, cx);
                        } else if let Some(next) = navigation_index(index, key_targets.len(), key) {
                            window.prevent_default();
                            cx.stop_propagation();
                            focus_option(&key_targets[next], &key_scroll, window, cx);
                        }
                    })
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
                                    .map(|d| div().text_color(theme::rambutan_400()).child(d)),
                            ),
                    )
                    // Web: the chosen row's check sits at the right.
                    .when(is_current, |el| {
                        el.child(
                            svg()
                                .debug_selector(move || check_selector)
                                .path(assets::CHECK_ICON_PATH)
                                .size(px(theme::ICON_XS_PX))
                                .flex_shrink_0()
                                .text_color(theme::rambutan_100()),
                        )
                    })
                    .into_any_element(),
            );
        }
    }

    div()
        .id(ElementId::Name(format!("{id}-list").into()))
        .debug_selector(move || format!("{id}-list"))
        .role(Role::ListBox)
        .aria_label(aria_label)
        .w_full()
        .max_h(px(LIST_MAX_HEIGHT_PX))
        .track_scroll(&scroll)
        .overflow_y_scroll()
        .rounded(px(LIST_RADIUS_PX))
        .border_1()
        .border_color(theme::ring_floating())
        .bg(theme::longan_700())
        .py_1()
        .children(rows)
        .vertical_scrollbar(&scroll)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;
    // Explicit imports: `super::*` would pull in gpui's `test` macro.
    use super::{
        current_label, dropdown, navigation_index, DropdownGroup, DropdownOption, Listbox,
    };
    use crate::views::workspace::CloseOverlays;
    use gpui::{
        div, Context, InteractiveElement, IntoElement, KeyBinding, Modifiers, ParentElement,
        Render, Styled, TestAppContext, Window,
    };
    use std::rc::Rc;

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

    #[test]
    fn option_navigation_wraps_and_handles_empty_and_changed_lists() {
        assert_eq!(navigation_index(0, 3, "up"), Some(2));
        assert_eq!(navigation_index(2, 3, "down"), Some(0));
        assert_eq!(navigation_index(99, 2, "home"), Some(0));
        assert_eq!(navigation_index(0, 2, "end"), Some(1));
        assert_eq!(navigation_index(0, 0, "down"), None);
        assert_eq!(navigation_index(0, 3, "tab"), None);
    }

    struct DropdownHarness {
        open: bool,
        value: &'static str,
        dismissed: bool,
        long: bool,
    }

    impl Render for DropdownHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let toggle = cx.entity().downgrade();
            let select = toggle.clone();
            div()
                .size_full()
                .on_action(cx.listener(|this, _: &CloseOverlays, _, _| this.dismissed = true))
                .child(dropdown(
                    "test-dropdown",
                    "Test dropdown",
                    self.value,
                    vec![DropdownGroup {
                        label: Some("Choices"),
                        mark: None,
                        options: (if self.long {
                            &[
                                "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m",
                                "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
                            ][..]
                        } else {
                            &["a", "b", "c"][..]
                        })
                        .iter()
                        .copied()
                        .map(|value| DropdownOption {
                            value,
                            label: value.into(),
                            description: None,
                        })
                        .collect(),
                    }],
                    Listbox::open_if(self.open),
                    Rc::new(move |_, cx| {
                        let _ = toggle.update(cx, |this, cx| {
                            this.open = !this.open;
                            cx.notify();
                        });
                    }),
                    Rc::new(move |value, _, cx| {
                        let _ = select.update(cx, |this, cx| {
                            this.value = value;
                            this.open = false;
                            cx.notify();
                        });
                    }),
                ))
        }
    }

    #[gpui::test]
    fn dropdown_keyboard_selection_escape_and_trigger_focus(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            cx.bind_keys([KeyBinding::new("escape", CloseOverlays, None)]);
        });
        let (view, cx) = cx.add_window_view(|_, _| DropdownHarness {
            open: false,
            value: "b",
            dismissed: false,
            long: false,
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("test-dropdown").expect("dropdown trigger");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        cx.simulate_keystrokes("down end enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.value, "c");
            assert!(!view.open);
        });
        // Space can reopen only if selection returned focus to the trigger.
        cx.simulate_keystrokes("space");
        cx.run_until_parked();
        view.read_with(cx, |view, _| assert!(view.open));
        cx.simulate_keystrokes("escape");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert!(!view.open);
            assert!(
                !view.dismissed,
                "Escape must close the dropdown before its parent"
            );
        });
        cx.simulate_keystrokes("down home enter");
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.value, "a");
            assert!(!view.open);
        });
    }

    /// Like web, only the chosen option shows a check, at the right of its
    /// label.
    #[gpui::test]
    fn dropdown_check_sits_at_the_right_of_the_chosen_option(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (_view, cx) = cx.add_window_view(|_, _| DropdownHarness {
            open: true,
            value: "b",
            dismissed: false,
            long: false,
        });
        cx.run_until_parked();
        let option = cx
            .debug_bounds("test-dropdown-option-b")
            .expect("chosen option");
        let check = cx
            .debug_bounds("test-dropdown-option-b-check")
            .expect("its check");
        assert!(
            check.left() > option.center().x,
            "the check sits at the right"
        );
        assert_eq!(check.size.width, gpui::px(crate::theme::ICON_XS_PX));
        assert!(cx.debug_bounds("test-dropdown-option-a-check").is_none());
    }

    #[gpui::test]
    fn long_dropdown_keeps_keyboard_focused_option_in_view(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (_, cx) = cx.add_window_view(|_, _| DropdownHarness {
            open: false,
            value: "a",
            dismissed: false,
            long: true,
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("test-dropdown").expect("dropdown trigger");
        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.run_until_parked();
        cx.simulate_keystrokes("down end");
        cx.run_until_parked();
        let list = cx
            .debug_bounds("test-dropdown-list")
            .expect("dropdown list");
        let last = cx
            .debug_bounds("test-dropdown-option-z")
            .expect("last option");
        assert!(last.top() >= list.top(), "focused row above viewport");
        assert!(last.bottom() <= list.bottom(), "focused row below viewport");
    }
    struct TriggerGeometryHarness {
        focus: gpui::FocusHandle,
    }

    impl Render for TriggerGeometryHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div().size_full().child(
                super::dropdown_trigger(
                    "focus-dropdown",
                    "Model".into(),
                    "Claude Sonnet 4.6".into(),
                    false,
                    &self.focus,
                )
                .w(gpui::px(240.0))
                .child(
                    div()
                        .debug_selector(|| "focus-dropdown-content".to_string())
                        .size(gpui::px(8.0)),
                ),
            )
        }
    }

    #[gpui::test]
    fn native_dropdown_keeps_content_still_during_keyboard_focus(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (view, cx) = cx.add_window_view(|_, cx| TriggerGeometryHarness {
            focus: cx.focus_handle(),
        });
        cx.run_until_parked();
        let before = cx.debug_bounds("focus-dropdown").unwrap();
        let children = cx.debug_bounds("focus-dropdown-content").unwrap();
        cx.simulate_mouse_move(before.center(), None, Modifiers::default());
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("focus-dropdown"), Some(before));
        assert_eq!(cx.debug_bounds("focus-dropdown-content").unwrap(), children);
        cx.simulate_keystrokes("right");
        view.update_in(cx, |view, window, cx| {
            view.focus.focus(window, cx);
            assert!(window.last_input_was_keyboard());
        });
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("focus-dropdown"), Some(before));
        assert_eq!(cx.debug_bounds("focus-dropdown-content").unwrap(), children);
    }
}
