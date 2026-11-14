use super::*;

impl Workspace {
    pub(super) fn render_workspace_navigation(
        &self,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        div()
            .id("workspace-navigation")
            .debug_selector(|| "desktop-workspace-navigation".to_string())
            .role(Role::Navigation)
            .aria_label("Workspace")
            .window_control_area(WindowControlArea::Drag)
            .relative()
            .h(px(44.0))
            .flex_shrink_0()
            .px(px(12.0))
            .when(compact || !self.sidebar_visible, |el| {
                el.pl(px(SIDEBAR_HEADER_LEFT_INSET_PX))
            })
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(
                div()
                    .absolute()
                    .left_0()
                    .bottom_0()
                    .w_full()
                    .h(px(1.0))
                    .bg(theme::longan_800().opacity(0.5)),
            )
            .child(self.render_collapsed_sidebar_strip(cx))
            .child(
                div()
                    .id("workspace-threads")
                    .debug_selector(|| "desktop-workspace-threads".to_string())
                    .role(Role::Button)
                    .aria_label("Threads")
                    .aria_toggled(if self.active_view == AppView::Chat {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .tab_index(0)
                    .h(px(44.0))
                    .px(px(12.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .border_b_2()
                    .border_color(if self.active_view == AppView::Chat {
                        theme::blueberry_400()
                    } else {
                        gpui::transparent_black()
                    })
                    .text_size(px(14.0))
                    .text_color(if self.active_view == AppView::Chat {
                        theme::rambutan_100()
                    } else {
                        theme::rambutan_500()
                    })
                    .hover(|el| {
                        el.bg(theme::longan_800().opacity(0.4))
                            .text_color(theme::rambutan_100())
                    })
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.activate_sidebar_view(AppView::Chat, window, cx)
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if is_web_button_activation_key(event.keystroke.key.as_str()) {
                            window.prevent_default();
                            cx.stop_propagation();
                            this.activate_sidebar_view(AppView::Chat, window, cx);
                        }
                    }))
                    .child(svg().path("icons/message-square.svg").size(px(14.0)))
                    .child("Threads"),
            )
    }
}
