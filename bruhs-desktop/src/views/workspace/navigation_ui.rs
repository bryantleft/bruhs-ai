use super::*;

impl Workspace {
    /// The main column's title bar row: window drag area, room for the
    /// traffic lights when the sidebar is hidden, and the sidebar toggle.
    /// `sidebar_drawer`: the window is narrow enough that the sidebar opens
    /// as a drawer, so it is not docked beside this row.
    pub(super) fn render_workspace_navigation(
        &self,
        sidebar_drawer: bool,
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
            .when(sidebar_drawer || !self.sidebar_visible, |el| {
                el.pl(px(self.traffic_light_inset_px()))
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
            // The sidebar toggle. No Threads tab: the active thread's card
            // (and ⌘K) return to the conversation from any view.
            .child(self.render_collapsed_sidebar_strip(cx))
    }
}
