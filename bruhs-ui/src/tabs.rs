//! Accessible tab lists, triggers, and panels (`role="tablist"` / `role="tab"` /
//! `role="tabpanel"`).
//!
//! Covers both pill tabs (used in import/move-in pickers) and segmented tabs
//! (used in compact destination switchers like Capture). Prefer [`Tabs`] over
//! raw [`tab_list`] + [`tab_trigger`] so the list owns roving focus and
//! Left/Right/Home/End keyboard movement per the W3C APG tabs pattern.

use std::rc::Rc;

use crate::press::on_press;
use crate::theme::{self, FontFace};
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{
    div, AnyElement, App, Div, ElementId, FocusHandle, FontWeight, KeyDownEvent, Orientation, Role,
    SharedString, Stateful, StyleRefinement, Window,
};

const TAB_RADIUS_PX: f32 = 8.0;
const SEGMENTED_TRIGGER_RADIUS_PX: f32 = 6.0;
const TAB_TEXT_SIZE_PX: f32 = 12.0;

/// Visual treatment for a tab list and its triggers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStyle {
    /// Scrollable row of pill tabs (`blueberry` active tint), for dialog source pickers.
    Pill,
    /// Recessed track (`longan-900/50`) with raised active tab (`longan-700`).
    Segmented,
}

/// Callback invoked when a tab is activated via pointer, Enter/Space, or arrow keys.
pub type TabSelectHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// One tab inside [`Tabs`].
pub struct TabItem {
    id: ElementId,
    aria_label: SharedString,
    debug_selector: Option<SharedString>,
    group: Option<SharedString>,
    focus: Option<FocusHandle>,
    disabled: bool,
    content: Vec<AnyElement>,
}

impl TabItem {
    pub fn new(id: impl Into<ElementId>, aria_label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            aria_label: aria_label.into(),
            debug_selector: None,
            group: None,
            focus: None,
            disabled: false,
            content: Vec::new(),
        }
    }

    /// Attach a debug selector to this tab trigger.
    pub fn debug_selector(mut self, selector: impl Into<SharedString>) -> Self {
        self.debug_selector = Some(selector.into());
        self
    }

    /// Attach a GPUI hover group name to this tab trigger.
    pub fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }

    /// Reuse a caller-owned focus handle instead of allocating an internal one.
    pub fn focus_handle(mut self, focus: FocusHandle) -> Self {
        self.focus = Some(focus);
        self
    }

    /// Prevent pointer and keyboard selection on this tab.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Append visual content inside the trigger (text, icon, etc.).
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.content.push(child.into_any_element());
        self
    }
}

/// A `role="tablist"` with roving focus and Left/Right/Home/End keyboard
/// navigation across its [`TabItem`]s.
#[derive(IntoElement)]
pub struct Tabs {
    id: SharedString,
    aria_label: SharedString,
    style: TabStyle,
    selected: usize,
    items: Vec<TabItem>,
    on_select: Option<TabSelectHandler>,
    column_triggers: bool,
    refinement: StyleRefinement,
}

impl Tabs {
    pub fn new(
        id: impl Into<SharedString>,
        aria_label: impl Into<SharedString>,
        style: TabStyle,
        selected: usize,
    ) -> Self {
        Self {
            id: id.into(),
            aria_label: aria_label.into(),
            style,
            selected,
            items: Vec::new(),
            on_select: None,
            column_triggers: false,
            refinement: StyleRefinement::default(),
        }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = TabItem>) -> Self {
        self.items.extend(items);
        self
    }

    pub fn item(mut self, item: TabItem) -> Self {
        self.items.push(item);
        self
    }

    /// Stack trigger children vertically (e.g., an icon above its label).
    pub fn column_triggers(mut self) -> Self {
        self.column_triggers = true;
        self
    }

    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(handler));
        self
    }
}

impl Styled for Tabs {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.refinement
    }
}

fn next_enabled_index(current: usize, enabled: &[bool], key: &str) -> Option<usize> {
    let count = enabled.len();
    if count == 0 || !enabled.iter().any(|ok| *ok) {
        return None;
    }
    match key {
        "right" | "arrowright" | "ArrowRight" => (1..=count)
            .map(|step| (current + step) % count)
            .find(|&idx| enabled[idx]),
        "left" | "arrowleft" | "ArrowLeft" => (1..=count)
            .map(|step| (current + count - step) % count)
            .find(|&idx| enabled[idx]),
        "home" | "Home" => (0..count).find(|&idx| enabled[idx]),
        "end" | "End" => (0..count).rev().find(|&idx| enabled[idx]),
        _ => None,
    }
}

impl RenderOnce for Tabs {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let count = self.items.len();
        let state = window.use_keyed_state(
            SharedString::from(format!("{}-tabs-focus", self.id)),
            cx,
            |_, _| Vec::<FocusHandle>::new(),
        );
        let handles: Rc<[FocusHandle]> = state.update(cx, |stored, cx| {
            while stored.len() < count {
                stored.push(cx.focus_handle());
            }
            stored.truncate(count);
            self.items
                .iter()
                .enumerate()
                .map(|(index, item)| item.focus.clone().unwrap_or_else(|| stored[index].clone()))
                .collect::<Vec<_>>()
                .into()
        });
        let enabled: Rc<[bool]> = self.items.iter().map(|item| !item.disabled).collect();
        let active_index = if count == 0 {
            0
        } else if self.selected < count && enabled[self.selected] {
            self.selected
        } else {
            enabled.iter().position(|ok| *ok).unwrap_or(0)
        };
        let on_select = self.on_select;
        let column_triggers = self.column_triggers;
        let style = self.style;

        let mut list = tab_list(self.id, self.aria_label, style);
        list.style().refine(&self.refinement);

        list.children(self.items.into_iter().enumerate().map(|(index, item)| {
            let selected = index == self.selected;
            let is_tab_stop = index == active_index && !item.disabled;
            let handle = handles[index].clone().tab_index(0).tab_stop(is_tab_stop);
            let mut trigger = tab_trigger(item.id, item.aria_label, selected, style)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(count)
                .tab_stop(is_tab_stop)
                .track_focus(&handle)
                .when_some(item.debug_selector, |el, selector| {
                    el.debug_selector(move || selector.to_string())
                })
                .when_some(item.group, |el, group| el.group(group))
                .when(column_triggers, |el| {
                    el.flex().flex_col().items_center().gap_1()
                })
                .when(item.disabled, |el| el.cursor_not_allowed().opacity(0.5))
                .children(item.content);

            if !item.disabled {
                if let Some(select) = on_select.clone() {
                    let focus_on_press = handle;
                    trigger = on_press(trigger, move |window, cx| {
                        focus_on_press.focus(window, cx);
                        select(index, window, cx);
                    });
                }
                let nav_handles = handles.clone();
                let nav_enabled = enabled.clone();
                let nav_select = on_select.clone();
                trigger = trigger.on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.modifiers.modified() {
                        return;
                    }
                    if let Some(next) =
                        next_enabled_index(index, &nav_enabled, event.keystroke.key.as_str())
                    {
                        window.prevent_default();
                        cx.stop_propagation();
                        nav_handles[next].focus(window, cx);
                        if let Some(select) = &nav_select {
                            select(next, window, cx);
                        }
                    }
                });
            }
            trigger
        }))
    }
}

/// A `role="tablist"` container.
pub fn tab_list(
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    style: TabStyle,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .role(Role::TabList)
        .aria_orientation(Orientation::Horizontal)
        .aria_label(aria_label.into())
        .flex()
        .gap_1();
    match style {
        TabStyle::Pill => base.overflow_x_scroll(),
        TabStyle::Segmented => base
            .rounded(px(TAB_RADIUS_PX))
            .bg(theme::longan_900().opacity(0.5))
            .border_1()
            .border_color(gpui::transparent_black())
            .focus_visible(|el| el.border_color(theme::persimmon_400()))
            .p_1(),
    }
}

/// A `role="tabpanel"` container paired with a tab list.
pub fn tab_panel(id: impl Into<ElementId>, aria_label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .role(Role::TabPanel)
        .aria_label(aria_label.into())
}

/// A `role="tab"` trigger. Attach activation with `on_press` / `on_press_view`.
pub fn tab_trigger(
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    selected: bool,
    style: TabStyle,
) -> Stateful<Div> {
    let base = div()
        .id(id)
        .role(Role::Tab)
        .aria_label(aria_label.into())
        .aria_selected(selected)
        .tab_index(0)
        .border_1()
        .cursor_pointer()
        .focus_visible(|el| el.border_color(theme::persimmon_400()));
    match style {
        TabStyle::Pill => base
            .flex_shrink_0()
            .whitespace_nowrap()
            .rounded(px(TAB_RADIUS_PX))
            .px(px(10.0))
            .py_1()
            .font_family(FontFace::Small)
            .font_weight(FontWeight::MEDIUM)
            .text_size(px(TAB_TEXT_SIZE_PX))
            .map(|el| {
                if selected {
                    el.bg(theme::blueberry_600().opacity(0.20))
                        .border_color(theme::blueberry_500().opacity(0.30))
                        .text_color(theme::blueberry_200())
                } else {
                    el.border_color(gpui::transparent_black())
                        .text_color(theme::rambutan_400())
                        .hover(|el| {
                            el.bg(theme::longan_800().opacity(0.50))
                                .text_color(theme::rambutan_200())
                        })
                }
            }),
        TabStyle::Segmented => base
            .border_color(gpui::transparent_black())
            .flex_1()
            .min_w_0()
            .rounded(px(SEGMENTED_TRIGGER_RADIUS_PX))
            .px_1()
            .py_2()
            .text_xs()
            .text_center()
            .text_color(if selected {
                theme::rambutan_100()
            } else {
                theme::rambutan_500()
            })
            .when(selected, |el| el.bg(theme::longan_700()))
            .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_200())),
    }
}

#[cfg(test)]
mod tests {
    use super::{tab_list, tab_panel, tab_trigger, TabItem, TabStyle, Tabs};
    use ::core::prelude::v1::test;
    use gpui::{
        actions, div, Context, FocusHandle, InteractiveElement, IntoElement, KeyBinding,
        ParentElement, Render, Styled, TestAppContext, Window,
    };

    actions!(tabs_test, [FocusNext]);

    struct Sample;

    impl Render for Sample {
        fn render(&mut self, _: &mut Window, _: &mut Context<'_, Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    tab_list("pills", "Sources", TabStyle::Pill)
                        .child(
                            tab_trigger("p1", "First", true, TabStyle::Pill)
                                .debug_selector(|| "tab-p1".to_owned())
                                .child("First"),
                        )
                        .child(
                            tab_trigger("p2", "Second", false, TabStyle::Pill)
                                .debug_selector(|| "tab-p2".to_owned())
                                .child("Second"),
                        ),
                )
                .child(
                    tab_list("segs", "Modes", TabStyle::Segmented).child(
                        tab_trigger("s1", "One", true, TabStyle::Segmented)
                            .debug_selector(|| "tab-s1".to_owned())
                            .child("One"),
                    ),
                )
        }
    }

    struct KeyboardTabs {
        selected: usize,
        root_focus: FocusHandle,
        after_focus: FocusHandle,
    }

    impl Render for KeyboardTabs {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let view = cx.entity().downgrade();
            let labels = ["First", "Second", "Third"];
            div()
                .size_full()
                .track_focus(&self.root_focus)
                .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
                .child(
                    Tabs::new("kb-tabs", "Choices", TabStyle::Segmented, self.selected)
                        .items(labels.into_iter().enumerate().map(|(i, label)| {
                            TabItem::new(("kb-tab", i), label)
                                .debug_selector(format!("kb-tab-{i}"))
                                .child(label)
                        }))
                        .on_select(move |index, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.selected = index;
                                cx.notify();
                            });
                        }),
                )
                .child(
                    tab_panel("kb-panel", labels[self.selected]).child(
                        div()
                            .id("after-tabs")
                            .track_focus(&self.after_focus)
                            .tab_index(0)
                            .child("Panel control"),
                    ),
                )
        }
    }

    #[gpui::test]
    fn tabs_render_both_styles(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| Sample);
        cx.run_until_parked();
        for selector in ["tab-p1", "tab-p2", "tab-s1"] {
            let bounds = cx.debug_bounds(selector).expect(selector);
            assert!(bounds.size.width > gpui::px(0.0), "{selector}");
            assert!(bounds.size.height > gpui::px(0.0), "{selector}");
        }
    }

    #[gpui::test]
    fn tabs_support_roving_focus_and_arrow_navigation(cx: &mut TestAppContext) {
        cx.update(|cx| {
            crate::init(cx);
            cx.bind_keys([KeyBinding::new("tab", FocusNext, None)]);
        });
        let (view, cx) = cx.add_window_view(|_, cx| KeyboardTabs {
            selected: 0,
            root_focus: cx.focus_handle(),
            after_focus: cx.focus_handle().tab_index(0).tab_stop(true),
        });
        cx.update(|window, cx| {
            let root = view.read(cx).root_focus.clone();
            root.focus(window, cx);
        });
        cx.run_until_parked();

        // Tab enters the tablist on the active tab (single tab stop).
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selected), 0);

        // Right/Left/End/Home move selection and wrap across tabs.
        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selected), 1);

        cx.simulate_keystrokes("end");
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selected), 2);

        cx.simulate_keystrokes("right");
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selected), 0);

        cx.simulate_keystrokes("left");
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selected), 2);

        cx.simulate_keystrokes("home");
        cx.run_until_parked();
        assert_eq!(view.read_with(cx, |v, _| v.selected), 0);

        // Tab leaves the entire tablist straight to the panel control instead of
        // stopping on every unselected tab.
        cx.simulate_keystrokes("tab");
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(
                view.read(cx).after_focus.is_focused(window),
                "Tab should leave the tablist directly to the panel control"
            );
        });
    }
}
