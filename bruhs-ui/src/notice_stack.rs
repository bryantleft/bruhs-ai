//! Floating top-right notification stack reusing [`BannerTone`] semantics.
//!
//! Keeps notices clear of bottom composers while providing accessible `Role::Status`
//! announcements, optional action triggers, and keyboard-accessible dismissal.

use crate::banner::BannerTone;
use crate::icons;
use crate::press::on_press;
use crate::theme;
use crate::zoom::px;
use gpui::prelude::*;
use gpui::{
    div, relative, svg, AnyElement, App, Hsla, Role, SharedString, StyleRefinement, Window,
};
use std::rc::Rc;

/// Handler invoked when a notice's action or dismiss button is activated.
pub type NoticeHandler = Rc<dyn Fn(&mut Window, &mut App)>;

/// Maximum width of a floating notice card.
pub const NOTICE_MAX_WIDTH_PX: f32 = 360.0;
const NOTICE_RADIUS_PX: f32 = 10.0;

fn focus_ring(style: StyleRefinement) -> StyleRefinement {
    style.border_color(theme::persimmon_400())
}

fn tone_chrome(tone: BannerTone) -> (Hsla, Hsla, &'static str) {
    match tone {
        BannerTone::Error => (
            theme::lychee_500().opacity(0.40),
            theme::lychee_400(),
            icons::CIRCLE_ALERT,
        ),
        BannerTone::Warning => (
            theme::durian_500().opacity(0.40),
            theme::durian_300(),
            icons::ALERT_TRIANGLE,
        ),
        BannerTone::Info => (
            theme::blueberry_500().opacity(0.40),
            theme::blueberry_300(),
            icons::HELP_CIRCLE,
        ),
        BannerTone::Success => (
            theme::pandan_500().opacity(0.40),
            theme::pandan_400(),
            icons::CHECK,
        ),
        BannerTone::Neutral => (theme::ring_floating(), theme::rambutan_300(), icons::CHECK),
    }
}

/// Optional inline action button rendered inside a [`NoticeCard`].
pub struct NoticeAction {
    pub id: SharedString,
    pub label: SharedString,
    pub debug_selector: Option<SharedString>,
    pub on_press: NoticeHandler,
}

impl NoticeAction {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        on_press: NoticeHandler,
    ) -> Self {
        let id = id.into();
        Self {
            debug_selector: Some(id.clone()),
            id,
            label: label.into(),
            on_press,
        }
    }
}

/// One floating toast card inside a [`NoticeStack`].
pub struct NoticeCard {
    id: SharedString,
    debug_selector: Option<SharedString>,
    tone: BannerTone,
    message: SharedString,
    action: Option<NoticeAction>,
    dismiss_id: SharedString,
    dismiss_selector: Option<SharedString>,
    on_dismiss: Option<NoticeHandler>,
}

impl NoticeCard {
    pub fn new(
        id: impl Into<SharedString>,
        tone: BannerTone,
        message: impl Into<SharedString>,
    ) -> Self {
        let id = id.into();
        let dismiss_id = SharedString::from(format!("{id}-dismiss"));
        Self {
            debug_selector: Some(id.clone()),
            dismiss_selector: Some(dismiss_id.clone()),
            id,
            tone,
            message: message.into(),
            action: None,
            dismiss_id,
            on_dismiss: None,
        }
    }

    pub fn action(mut self, action: NoticeAction) -> Self {
        self.action = Some(action);
        self
    }

    pub fn on_dismiss(mut self, handler: NoticeHandler) -> Self {
        self.on_dismiss = Some(handler);
        self
    }

    fn render_card(self, window: &Window) -> AnyElement {
        let (ring, accent, icon_path) = tone_chrome(self.tone);
        let bg = theme::longan_900();
        let fg = theme::rambutan_100();

        let action_el = self.action.map(|action| {
            let on_action = action.on_press;
            on_press(
                div()
                    .id(action.id)
                    .when_some(action.debug_selector, |el, sel| {
                        el.debug_selector(move || sel.to_string())
                    })
                    .role(Role::Button)
                    .aria_label(action.label.clone())
                    .tab_index(0)
                    .cursor_pointer()
                    .my_neg_1()
                    .min_h(px(theme::CONTROL_PX))
                    .min_w(px(theme::CONTROL_PX))
                    .px_1p5()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(focus_ring)
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::persimmon_300())
                    .hover(|el| el.text_color(theme::persimmon_200()))
                    .child(action.label),
                move |window, cx| on_action(window, cx),
            )
        });

        let dismiss_el = self.on_dismiss.map(|dismiss| {
            on_press(
                div()
                    .id(self.dismiss_id)
                    .when_some(self.dismiss_selector, |el, sel| {
                        el.debug_selector(move || sel.to_string())
                    })
                    .role(Role::Button)
                    .aria_label("Dismiss notification")
                    .tab_index(0)
                    .cursor_pointer()
                    .my_neg_1()
                    .size(px(theme::CONTROL_PX))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .hover(|el| el.bg(theme::longan_800()))
                    .focus_visible(focus_ring)
                    .child(
                        svg()
                            .path(icons::CLOSE)
                            .size(px(theme::ICON_INLINE_SM_PX))
                            .text_color(theme::rambutan_400()),
                    ),
                move |window, cx| dismiss(window, cx),
            )
        });

        div()
            .id(self.id)
            .when_some(self.debug_selector, |el, sel| {
                el.debug_selector(move || sel.to_string())
            })
            .occlude()
            .w_full()
            .min_w_0()
            .max_w(px(NOTICE_MAX_WIDTH_PX))
            .max_h(window.viewport_size().height * 0.3)
            .overflow_y_scroll()
            .flex_shrink_0()
            .rounded(px(NOTICE_RADIUS_PX))
            .bg(bg)
            .border_1()
            .border_color(ring)
            .shadow_md()
            .pl_3()
            .pr_2()
            .py_2()
            .flex()
            .items_center()
            .gap_2p5()
            .font_family(theme::FontFace::Interface.with_default(theme::UI_FONT_FAMILY))
            .text_size(px(14.0))
            .line_height(relative(1.4))
            .text_color(fg)
            .child(
                svg()
                    .path(icon_path)
                    .size(px(theme::ICON_PX))
                    .flex_shrink_0()
                    .text_color(accent),
            )
            .child(div().min_w_0().flex_1().child(self.message))
            .children(action_el)
            .children(dismiss_el)
            .into_any_element()
    }
}

/// Top-right floating rail displaying one or more [`NoticeCard`] items.
#[derive(IntoElement)]
pub struct NoticeStack {
    id: SharedString,
    debug_selector: Option<SharedString>,
    top_offset_px: f32,
    items: Vec<NoticeCard>,
}

impl NoticeStack {
    pub fn new(id: impl Into<SharedString>, items: Vec<NoticeCard>) -> Self {
        let id = id.into();
        Self {
            debug_selector: Some(id.clone()),
            id,
            top_offset_px: theme::APP_HEADER_HEIGHT_PX + 10.0,
            items,
        }
    }

    pub fn top_offset_px(mut self, offset: f32) -> Self {
        self.top_offset_px = offset;
        self
    }
}

impl RenderOnce for NoticeStack {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .when_some(self.debug_selector, |el, sel| {
                el.debug_selector(move || sel.to_string())
            })
            .role(Role::Status)
            .aria_label("Notifications")
            .absolute()
            .top(px(self.top_offset_px))
            .left_0()
            .right_0()
            .px_3()
            .flex()
            .flex_col()
            .items_end()
            .gap_2()
            .children(self.items.into_iter().map(|card| card.render_card(window)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Context, TestAppContext};

    struct NoticeHarness {
        visible: bool,
        actions: usize,
    }

    impl gpui::Render for NoticeHarness {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let act = cx.entity().downgrade();
            let dis = cx.entity().downgrade();
            div().relative().size_full().when(self.visible, |el| {
                el.child(NoticeStack::new(
                    "test-notice-rail",
                    vec![NoticeCard::new(
                        "test-notice",
                        BannerTone::Warning,
                        "Connection interrupted",
                    )
                    .action(NoticeAction::new(
                        "test-notice-action",
                        "Retry",
                        Rc::new(move |_, cx| {
                            let _ = act.update(cx, |this, cx| {
                                this.actions += 1;
                                cx.notify();
                            });
                        }),
                    ))
                    .on_dismiss(Rc::new(move |_, cx| {
                        let _ = dis.update(cx, |this, cx| {
                            this.visible = false;
                            cx.notify();
                        });
                    }))],
                ))
            })
        }
    }

    #[gpui::test]
    fn notice_stack_handles_action_and_dismiss(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| NoticeHarness {
            visible: true,
            actions: 0,
        });
        cx.run_until_parked();

        let action = cx
            .debug_bounds("test-notice-action")
            .expect("action button");
        cx.simulate_click(action.center(), gpui::Modifiers::default());
        assert_eq!(view.read_with(cx, |h, _| h.actions), 1);

        let dismiss = cx
            .debug_bounds("test-notice-dismiss")
            .expect("dismiss button");
        cx.simulate_click(dismiss.center(), gpui::Modifiers::default());
        assert!(!view.read_with(cx, |h, _| h.visible));
    }
}
