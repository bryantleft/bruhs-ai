use super::*;

#[derive(Clone)]
pub(super) struct ThreadCardHover {
    identity: CardIdentity,
    environment: SharedString,
    workspace: WeakEntity<Workspace>,
    thread: EntityId,
}

impl ThreadCardHover {
    pub(super) fn new(
        identity: CardIdentity,
        scope: &str,
        workspace: &Workspace,
        owner: WeakEntity<Workspace>,
        thread: EntityId,
    ) -> Self {
        let environment = if let Some((host, _)) = scope
            .strip_prefix("linked:")
            .and_then(|scope| scope.split_once(':'))
        {
            uuid::Uuid::parse_str(host)
                .ok()
                .and_then(|id| {
                    workspace
                        .linked_projects
                        .checkouts()
                        .find_map(|(host, name, _)| (host == id).then_some(name))
                })
                .unwrap_or_else(|| "Linked environment".into())
                .into()
        } else {
            "This computer".into()
        };
        Self {
            identity,
            environment,
            workspace: owner,
            thread,
        }
    }
}

fn detail(glyph: AnyElement, label: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .w(px(14.))
                .flex_none()
                .flex()
                .justify_center()
                .child(glyph),
        )
        .child(div().min_w_0().child(label.into()))
}

impl Render for ThreadCardHover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let identity = &self.identity;
        let model_text = identity.model.description(cx);
        let mut model_lines = model_text.lines();
        let model_name = model_lines.next().unwrap_or_default().to_owned();
        let provider = bruhs_models::agent::AgentProvider::for_model(&identity.model.id);
        let glyph = |path| icon(path, 14., theme::rambutan_400()).into_any_element();
        let width = px(284.).min(window.viewport_size().width - px(16.));
        let content = div()
            .id("desktop-sidebar-thread-hover")
            .debug_selector(|| "desktop-sidebar-thread-hover".to_string())
            .role(Role::Tooltip)
            .w(width)
            .px(px(12.))
            .py(px(10.))
            .rounded(px(SIDEBAR_RADIUS_GRAPE_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_800())
            .font_family(theme::UI_SMALL_FONT_FAMILY)
            .text_size(px(SIDEBAR_LABEL_TEXT_PX))
            .text_color(theme::rambutan_300())
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .mb(px(2.))
                    .text_size(px(SIDEBAR_CARD_TITLE_TEXT_PX))
                    .text_color(theme::rambutan_100())
                    .child(identity.title.clone()),
            )
            .child(detail(identity.mark.render(14.), identity.project.clone()))
            .child(detail(
                glyph(assets::TOOL_TERMINAL_ICON_PATH),
                self.environment.clone(),
            ))
            .when_some(identity.branch.clone(), |el, branch| {
                el.child(detail(glyph(assets::GIT_BRANCH_ICON_PATH), branch))
            })
            .child(detail(provider_mark(provider, cx), model_name))
            .children(model_lines.map(|line| {
                div()
                    .pl(px(22.))
                    .text_color(theme::rambutan_400())
                    .child(line.to_owned())
            }));
        let anchor = self.workspace.upgrade().and_then(|workspace| {
            workspace
                .read(cx)
                .sidebar_viewport
                .borrow()
                .thread_bounds(self.thread)
        });
        if let Some(anchor) = anchor {
            div().w(width).child(
                anchored()
                    .attach(anchor, Anchor::TopRight)
                    .offset(point(px(6.), px(0.)))
                    .snap_to_window()
                    .child(content),
            )
        } else {
            div().p(px(8.)).child(content)
        }
    }
}
