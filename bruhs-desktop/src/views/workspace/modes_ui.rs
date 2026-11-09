use super::*;

impl Workspace {
    fn select_workspace_mode(&mut self, mode: DesktopSidebarTab, cx: &mut Context<'_, Self>) {
        if self.sidebar_tab == mode && self.active_view == AppView::Chat {
            return;
        }
        self.sidebar_tab = mode;
        self.active_view = AppView::Chat;
        self.sidebar_filter_open = false;
        self.persist_and_notify(cx);
    }

    pub(super) fn render_workspace_modes(
        &self,
        compact: bool,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        div()
            .id("workspace-modes")
            .debug_selector(|| "desktop-workspace-modes".to_string())
            .role(Role::Navigation)
            .aria_label("Workspace mode")
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
            .children(
                [DesktopSidebarTab::Chat, DesktopSidebarTab::Code].map(|mode| {
                    let selected = self.active_view == AppView::Chat && self.sidebar_tab == mode;
                    div()
                        .id(ElementId::Name(
                            format!("workspace-mode-{}", mode.as_str()).into(),
                        ))
                        .debug_selector(move || format!("desktop-workspace-mode-{}", mode.as_str()))
                        .role(Role::Button)
                        .aria_label(mode.label())
                        .aria_toggled(if selected {
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
                        .border_color(if selected {
                            theme::blueberry_400()
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(px(14.0))
                        .text_color(if selected {
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
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.select_workspace_mode(mode, cx)),
                        )
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.select_workspace_mode(mode, cx);
                            }
                        }))
                        .child(
                            svg()
                                .path(if mode == DesktopSidebarTab::Chat {
                                    "icons/message-square.svg"
                                } else {
                                    "icons/terminal.svg"
                                })
                                .size(px(14.0)),
                        )
                        .child(mode.label())
                }),
            )
    }
}
