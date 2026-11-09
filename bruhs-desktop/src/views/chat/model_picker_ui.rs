//! The model picker popover: filtering, grouping by provider, keyboard picking, and the provider mark.

use super::*;
pub(super) const STATUS_CHANGE_MODEL_LABEL: &str = "Change model";
pub(super) const MODEL_PICKER_WIDTH_PX: f32 = 352.0;
pub(super) const MODEL_PICKER_LIST_MAX_HEIGHT_PX: f32 = 288.0;
pub(super) const MODEL_PICKER_RADIUS_PX: f32 = 8.0;
pub(super) const MODEL_PICKER_ROW_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_LABEL_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const MODEL_PICKER_GROUP_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_KBD_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_SEARCH_PLACEHOLDER: &str = "Search models…";
pub(super) const MODEL_PICKER_EMPTY_LABEL: &str = "No models match.";

/// Web `auth_line` for the Claude provider.
pub(super) fn model_picker_claude_line(authenticated: bool, user: Option<&str>) -> String {
    if authenticated {
        match user {
            Some(user) if !user.is_empty() => format!("Claude Code signed in as {user}"),
            _ => "Claude Code signed in".to_string(),
        }
    } else {
        "Claude Code not signed in — run `claude login`".to_string()
    }
}

/// Web `model_picker::filter_models`: every whitespace-separated word must
/// appear in the id, label, or provider name (case-insensitive).
pub fn filter_models_like_web(
    models: &'static [ModelInfo],
    query: &str,
) -> Vec<&'static ModelInfo> {
    let words: Vec<String> = query.split_whitespace().map(|w| w.to_lowercase()).collect();
    models
        .iter()
        .filter(|m| {
            let hay = format!(
                "{} {} {} {}",
                m.id,
                m.label,
                m.provider,
                provider_label(m.provider)
            )
            .to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect()
}

/// Web `model_picker::group_models`: providers in first-appearance order.
pub fn group_models_like_web(
    models: &[&'static ModelInfo],
) -> Vec<(AgentProvider, Vec<&'static ModelInfo>)> {
    let mut groups: Vec<(AgentProvider, Vec<&'static ModelInfo>)> = Vec::new();
    for model in models {
        match groups.iter_mut().find(|(p, _)| *p == model.provider) {
            Some((_, list)) => list.push(model),
            None => groups.push((model.provider, vec![model])),
        }
    }
    groups
}

impl ChatView {
    // ── Model picker (⌘⇧M popover) ─────────────────────────────────────────

    pub fn toggle_model_picker(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.model_picker_open {
            self.close_model_picker(window, cx);
        } else {
            self.open_model_picker(window, cx);
        }
    }

    pub(super) fn open_model_picker(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.model_picker_open = true;
        self.branch_menu_open = false;
        self.workspace_menu_open = false;
        self.effort_menu_open = false;
        self.model_picker_index = 0;
        self.model_picker_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn close_model_picker(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.model_picker_open {
            return;
        }
        self.model_picker_open = false;
        cx.notify();
        self.focus_composer(window, cx);
    }

    pub(super) fn model_picker_matches(&self, cx: &App) -> Vec<&'static ModelInfo> {
        let query = self.model_picker_query.read(cx).value().to_string();
        filter_models_like_web(MODEL_CATALOG, &query)
    }

    pub(super) fn select_model_from_picker(
        &mut self,
        model_id: &'static str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.set_model(model_id, cx);
        cx.emit(ChatViewEvent::SelectModel(model_id));
        self.close_model_picker(window, cx);
    }

    pub(super) fn select_highlighted_model(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let matches = self.model_picker_matches(cx);
        let index = self.model_picker_index.min(matches.len().saturating_sub(1));
        if let Some(model) = matches.get(index) {
            self.select_model_from_picker(model.id, window, cx);
        }
    }

    /// Keys while the picker is open. Returns true when consumed.
    pub(super) fn handle_model_picker_key(
        &mut self,
        key: &str,
        digit: Option<usize>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if let Some(n) = digit {
            let matches = self.model_picker_matches(cx);
            if let Some(model) = matches.get(n - 1) {
                self.select_model_from_picker(model.id, window, cx);
            }
            return true;
        }
        match key {
            "escape" => {
                self.close_model_picker(window, cx);
                true
            }
            "up" | "down" => {
                let count = self.model_picker_matches(cx).len();
                if count > 0 {
                    let current = self.model_picker_index.min(count - 1);
                    self.model_picker_index = if key == "up" {
                        (current + count - 1) % count
                    } else {
                        (current + 1) % count
                    };
                    cx.notify();
                }
                true
            }
            "enter" => {
                self.select_highlighted_model(window, cx);
                true
            }
            _ => false,
        }
    }

    /// Web `model_picker.rs`: a 22rem floating card (longan-700, white/15
    /// ring) — search row with an `esc` chip, grouped rows (label over
    /// provider glyph + id, `⌘n` chip, check on the current one), and the
    /// provider's sign-in line in the footer.
    pub(super) fn render_model_picker(&self, cx: &Context<'_, Self>) -> AnyElement {
        let matches = self.model_picker_matches(cx);
        let highlighted = self.model_picker_index.min(matches.len().saturating_sub(1));
        let groups = group_models_like_web(&matches);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut flat_index = 0usize;
        for (provider, models) in groups {
            rows.push(
                div()
                    .px_2()
                    .pt_2()
                    .pb_1()
                    .text_size(px(MODEL_PICKER_GROUP_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .child(provider_label(provider).to_ascii_uppercase())
                    .into_any_element(),
            );
            for model in models {
                let index = flat_index;
                flat_index += 1;
                let is_current = self.model_id == model.id;
                let is_highlighted = index == highlighted;
                let key = element_key(model.id);
                let model_id = model.id;
                let model_id_for_key = model.id;
                rows.push(
                    div()
                        .id(("desktop-model-picker-option", key))
                        .debug_selector(move || format!("desktop-model-picker-option-{key}"))
                        .px_2()
                        .py_2()
                        .rounded(px(MODEL_PICKER_RADIUS_PX))
                        .flex()
                        .items_center()
                        .gap_3()
                        .when(is_highlighted, |el| el.bg(gpui::white().opacity(0.06)))
                        .tab_index(0)
                        .role(Role::ListBoxOption)
                        .aria_label(format!("Select model {}", model.label))
                        .aria_selected(is_current)
                        .focus_visible(|el| el.border_2().border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .hover(|el| el.bg(gpui::white().opacity(0.06)))
                        .on_click(cx.listener(move |this, _ev, window, cx| {
                            cx.stop_propagation();
                            this.select_model_from_picker(model_id, window, cx);
                        }))
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if is_web_button_activation_key(event.keystroke.key.as_str()) {
                                window.prevent_default();
                                cx.stop_propagation();
                                this.select_model_from_picker(model_id_for_key, window, cx);
                            }
                        }))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(MODEL_PICKER_LABEL_TEXT_SIZE_PX))
                                        .text_color(theme::rambutan_100())
                                        .child(model.label),
                                )
                                .child(
                                    div()
                                        .mt(px(2.0))
                                        .flex()
                                        .items_center()
                                        .gap(px(6.0))
                                        .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                                        .font_family(theme::UI_SMALL_FONT_FAMILY)
                                        .text_color(theme::rambutan_400())
                                        .child(provider_mark(provider, cx))
                                        .child(div().min_w_0().truncate().child(model.id)),
                                ),
                        )
                        .when(index < 9, |el| {
                            el.child(
                                div()
                                    .flex_shrink_0()
                                    .rounded(px(4.0))
                                    .bg(theme::longan_800())
                                    .px_1()
                                    .text_size(px(MODEL_PICKER_KBD_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_400())
                                    .font(theme::tabular_system_font())
                                    .child(format!("⌘{}", index + 1)),
                            )
                        })
                        .when(is_current, |el| {
                            el.child(
                                svg()
                                    .path(assets::CHECK_ICON_PATH)
                                    .size(px(14.0))
                                    .text_color(theme::rambutan_100())
                                    .flex_shrink_0(),
                            )
                        })
                        .into_any_element(),
                );
            }
        }
        if matches.is_empty() {
            rows.push(
                div()
                    .debug_selector(|| "desktop-model-picker-empty".to_string())
                    .px_3()
                    .py_6()
                    .text_center()
                    .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .child(MODEL_PICKER_EMPTY_LABEL)
                    .into_any_element(),
            );
        }

        div()
            .id("desktop-model-picker")
            .debug_selector(|| "desktop-model-picker".to_string())
            .role(Role::Dialog)
            .aria_label(STATUS_CHANGE_MODEL_LABEL)
            .w(px(MODEL_PICKER_WIDTH_PX))
            .rounded(px(MODEL_PICKER_RADIUS_PX))
            .border_1()
            .border_color(gpui::white().opacity(0.15))
            .bg(theme::longan_700())
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(gpui::white().opacity(0.10))
                    .child(
                        svg()
                            .path(assets::TOOL_SEARCH_ICON_PATH)
                            .size(px(14.0))
                            .text_color(theme::rambutan_400())
                            .flex_shrink_0(),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.model_picker_query)
                                .appearance(false)
                                .aria_label("Search models"),
                        ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .rounded(px(4.0))
                            .bg(theme::longan_800())
                            .px_1()
                            .text_size(px(MODEL_PICKER_KBD_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .font(theme::tabular_system_font())
                            .child("esc"),
                    ),
            )
            .child(
                div()
                    .id("desktop-model-picker-list")
                    .role(Role::ListBox)
                    .max_h(px(MODEL_PICKER_LIST_MAX_HEIGHT_PX))
                    .overflow_y_scrollbar()
                    .p_1()
                    .children(rows),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(gpui::white().opacity(0.10))
                    .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .child(self.model_picker_status_line()),
            )
            .into_any_element()
    }

    /// Web `auth_line`: who the current provider is signed in as.
    pub(super) fn model_picker_status_line(&self) -> String {
        if self.is_claude_model() {
            model_picker_claude_line(self.auth_status.is_authenticated(), self.auth_status.user())
        } else {
            "Codex CLI signed in".to_string()
        }
    }

    pub(super) fn provider_mark(&self, _cx: &App) -> AnyElement {
        let logo_size = px(assets::PROVIDER_LOGO_SIZE_PX);
        let provider = AgentProvider::for_model(&self.model_id);
        div()
            .size(logo_size)
            .min_w(logo_size)
            .max_w(logo_size)
            .min_h(logo_size)
            .max_h(logo_size)
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .overflow_hidden()
            .debug_selector(|| format!("desktop-chat-provider-logo-{provider}"))
            .opacity(assets::PROVIDER_LOGO_OPACITY)
            .text_color(theme::rambutan_100())
            .child(
                svg()
                    .path(assets::provider_logo_path(provider))
                    .size(logo_size)
                    .text_color(theme::rambutan_100())
                    .min_w(logo_size)
                    .max_w(logo_size)
                    .min_h(logo_size)
                    .max_h(logo_size)
                    .flex_none()
                    .text_color(theme::rambutan_100()),
            )
            .into_any_element()
    }
}
