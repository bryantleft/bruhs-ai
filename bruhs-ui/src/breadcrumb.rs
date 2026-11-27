//! Breadcrumb navigation row with consistent slash separators.
//!
//! Supports static labels, clickable ancestor buttons, and custom interactive
//! segments (such as toolbar filter dropdowns) in a single horizontal rail.

use crate::button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::press::on_press;
use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{div, AnyElement, App, Role, SharedString, Window};
use std::rc::Rc;

/// Handler invoked when a clickable [`BreadcrumbItem::Action`] is activated.
pub type BreadcrumbActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// One segment in a [`Breadcrumb`] trail.
pub enum BreadcrumbItem {
    /// Non-interactive root or ancestor label (`rambutan-100`).
    Root { label: SharedString },
    /// Current-location trailing label (`rambutan-400`, truncates to fit).
    Current {
        label: SharedString,
        debug_selector: Option<SharedString>,
    },
    /// Clickable ghost button segment.
    Action {
        id: SharedString,
        label: SharedString,
        aria_label: SharedString,
        debug_selector: Option<SharedString>,
        on_press: BreadcrumbActionHandler,
    },
    /// Caller-supplied element (for example, a toolbar filter dropdown).
    Custom(AnyElement),
}

impl BreadcrumbItem {
    pub fn root(label: impl Into<SharedString>) -> Self {
        Self::Root {
            label: label.into(),
        }
    }

    pub fn current(label: impl Into<SharedString>) -> Self {
        Self::Current {
            label: label.into(),
            debug_selector: None,
        }
    }

    pub fn current_with_selector(
        label: impl Into<SharedString>,
        debug_selector: impl Into<SharedString>,
    ) -> Self {
        Self::Current {
            label: label.into(),
            debug_selector: Some(debug_selector.into()),
        }
    }

    pub fn action(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        aria_label: impl Into<SharedString>,
        on_press: BreadcrumbActionHandler,
    ) -> Self {
        let id = id.into();
        Self::Action {
            debug_selector: Some(id.clone()),
            id,
            label: label.into(),
            aria_label: aria_label.into(),
            on_press,
        }
    }

    pub fn custom(element: impl IntoElement) -> Self {
        Self::Custom(element.into_any_element())
    }
}

/// Horizontal breadcrumb bar with `/` separators and `Role::Navigation`.
#[derive(IntoElement)]
pub struct Breadcrumb {
    id: SharedString,
    aria_label: SharedString,
    gap_px: f32,
    items: Vec<BreadcrumbItem>,
}

impl Breadcrumb {
    pub fn new(id: impl Into<SharedString>, items: Vec<BreadcrumbItem>) -> Self {
        Self {
            id: id.into(),
            aria_label: "Breadcrumb".into(),
            gap_px: 8.0,
            items,
        }
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = label.into();
        self
    }

    pub fn gap_px(mut self, gap: f32) -> Self {
        self.gap_px = gap;
        self
    }
}

impl RenderOnce for Breadcrumb {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut row = div()
            .id(self.id)
            .role(Role::Navigation)
            .aria_label(self.aria_label)
            .flex()
            .items_center()
            .min_w_0()
            .gap(px(self.gap_px));

        for (index, item) in self.items.into_iter().enumerate() {
            if index > 0 {
                row = row.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.0))
                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                        .text_color(theme::rambutan_600())
                        .child("/"),
                );
            }
            let node: AnyElement = match item {
                BreadcrumbItem::Root { label } => div()
                    .flex_shrink_0()
                    .text_size(px(14.0))
                    .text_color(theme::rambutan_100())
                    .child(label)
                    .into_any_element(),
                BreadcrumbItem::Current {
                    label,
                    debug_selector,
                } => div()
                    .when_some(debug_selector, |el, selector| {
                        el.debug_selector(move || selector.to_string())
                    })
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.0))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .child(label)
                    .into_any_element(),
                BreadcrumbItem::Action {
                    id,
                    label,
                    aria_label,
                    debug_selector,
                    on_press: handler,
                } => on_press(
                    web_button(
                        id,
                        WebButtonVariant::Ghost,
                        WebButtonSize::Small,
                        ButtonState::Enabled,
                    )
                    .when_some(debug_selector, |el, selector| {
                        el.debug_selector(move || selector.to_string())
                    })
                    .aria_label(aria_label)
                    .child(label),
                    move |window, cx| handler(window, cx),
                )
                .into_any_element(),
                BreadcrumbItem::Custom(el) => el,
            };
            row = row.child(node);
        }

        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, TestAppContext};

    struct BreadcrumbHarness {
        back_clicks: usize,
    }

    impl gpui::Render for BreadcrumbHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let entity = cx.entity().downgrade();
            Breadcrumb::new(
                "test-breadcrumb",
                vec![
                    BreadcrumbItem::root("Settings"),
                    BreadcrumbItem::action(
                        "crumb-back-about",
                        "About",
                        "Back to About",
                        Rc::new(move |_, cx| {
                            let _ = entity.update(cx, |this, cx| {
                                this.back_clicks += 1;
                                cx.notify();
                            });
                        }),
                    ),
                    BreadcrumbItem::current_with_selector(
                        "Open source licenses",
                        "test-crumb-current",
                    ),
                ],
            )
        }
    }

    #[gpui::test]
    fn breadcrumb_activates_action_and_lays_out_current_segment(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| BreadcrumbHarness { back_clicks: 0 });
        cx.run_until_parked();

        let action = cx.debug_bounds("crumb-back-about").expect("action crumb");
        let current = cx
            .debug_bounds("test-crumb-current")
            .expect("current crumb");
        assert!(current.left() > action.right());

        cx.simulate_click(action.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.back_clicks), 1);
    }
}
