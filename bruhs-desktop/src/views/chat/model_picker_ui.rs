//! The model picker popover: filtering, grouping by provider, keyboard picking, and the provider mark.

use super::*;
use crate::views::model_browser::{self, Browser, FAVORITES};
use bruhs_models::logic::cli_login::{CODEX_SIGNED_IN_LINE, CODEX_SIGNED_OUT_LINE};
use bruhs_models::runtime_models::{
    highlighted_id, listed_models, listing_error_line, reanchor_highlight, replace_listing,
    ModelChoice, RuntimeModels,
};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;

/// A runtime's model listing as a future the desktop's tokio runtime drives.
type ListingFuture = std::pin::Pin<Box<dyn std::future::Future<Output = RuntimeModels> + Send>>;

type RuntimeModelListerFn =
    std::sync::Arc<dyn Fn(AgentProvider, std::path::PathBuf) -> ListingFuture + Send + Sync>;

/// Lists a runtime's models for the picker: core's CLI listing in the app, or
/// whatever a test installs, so tests run the real fetch path without a CLI.
/// Without one installed, tests list nothing.
#[derive(Clone)]
pub(crate) struct RuntimeModelLister(pub(crate) RuntimeModelListerFn);

impl gpui::Global for RuntimeModelLister {}

fn runtime_model_lister(cx: &App) -> Option<RuntimeModelLister> {
    cx.try_global::<RuntimeModelLister>()
        .cloned()
        .or_else(default_model_lister)
}

#[cfg(not(test))]
fn default_model_lister() -> Option<RuntimeModelLister> {
    Some(RuntimeModelLister(std::sync::Arc::new(
        |provider, project| {
            Box::pin(async move {
                bruhs_core::agent::list_runtime_models_for_project(provider, &project).await
            })
        },
    )))
}

/// Tests never run a CLI unless they install a lister.
#[cfg(test)]
fn default_model_lister() -> Option<RuntimeModelLister> {
    None
}
pub(super) const STATUS_CHANGE_MODEL_LABEL: &str = "Change model";
pub(super) const MODEL_PICKER_WIDTH_PX: f32 = 352.0;
pub(super) const MODEL_PICKER_LIST_MAX_HEIGHT_PX: f32 = 288.0;
pub(super) const MODEL_PICKER_RADIUS_PX: f32 = 8.0;
pub(super) const MODEL_PICKER_ROW_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_LABEL_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
pub(super) const MODEL_PICKER_GROUP_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_KBD_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_EMPTY_LABEL: &str =
    "No models match. Enter provider/model, e.g. cursor/auto.";

/// A typed provider/model choice, or the current custom model while searching,
/// unless it is already one of the `listed` rows.
pub fn custom_model_match(query: &str, current: &str, listed: &[ModelChoice]) -> Option<String> {
    let typed = bruhs_models::agent::custom_model_selection(query.trim());
    let candidate = typed.or_else(|| {
        let hay = current.to_lowercase();
        query
            .split_whitespace()
            .all(|word| hay.contains(&word.to_lowercase()))
            .then(|| bruhs_models::agent::custom_model_selection(current))
            .flatten()
    })?;
    (!listed.iter().any(|model| model.id == candidate.as_str())).then(|| candidate.into_inner())
}

/// Web `auth_line` for the Claude provider.
pub(super) fn model_picker_claude_line(authenticated: bool, user: Option<&str>) -> String {
    if authenticated {
        match user {
            Some(user) if !user.is_empty() => format!("Claude Code signed in as {user}"),
            _ => "Claude Code signed in".to_string(),
        }
    } else {
        "Claude Code not signed in. Run `claude login`".to_string()
    }
}

/// Web `model_picker::filter_models`: every whitespace-separated word must
/// appear in the id, label, or provider name (case-insensitive).
pub fn filter_models_like_web(models: &[ModelChoice], query: &str) -> Vec<ModelChoice> {
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
        .cloned()
        .collect()
}

/// Web `model_picker::group_models`: providers in first-appearance order.
pub fn group_models_like_web(models: &[ModelChoice]) -> Vec<(AgentProvider, Vec<ModelChoice>)> {
    let mut groups: Vec<(AgentProvider, Vec<ModelChoice>)> = Vec::new();
    for model in models {
        match groups.iter_mut().find(|(p, _)| *p == model.provider) {
            Some((_, list)) => list.push(model.clone()),
            None => groups.push((model.provider, vec![model.clone()])),
        }
    }
    groups
}

fn model_picker_row_index(models: &[ModelChoice], selected: usize) -> usize {
    let mut option = 0;
    let mut row = 0;
    for (_, models) in group_models_like_web(models) {
        row += 1; // Provider heading is a direct child of the scroll area.
        for _ in models {
            if option == selected {
                return row;
            }
            option += 1;
            row += 1;
        }
    }
    if selected == option {
        row
    } else {
        0
    }
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
        self.model_picker_dropdown = None;
        self.model_picker_open = true;
        self.composer_options_open = false;
        self.stash_open = false;
        self.branch_menu_open = false;
        self.workspace_menu_open = false;
        self.effort_menu_open = false;
        self.picker_availability =
            bruhs_core::agent_extensions::availability::ProviderAvailability::load_for_project(
                self.project_path.as_ref(),
            )
            .map_err(|err| format!("Could not read provider settings: {err}"));
        self.refresh_acp_models();
        self.model_browser_tab = None;
        self.model_browser_tab = Some(self.model_browser(&self.available_picker_models()).active);
        self.refresh_runtime_models(cx);
        self.refresh_model_catalog(cx);
        // Codex rows show its sign-in, so learn it if no thread has yet.
        if self.codex_auth.is_none() && self.picker_provider_enabled(AgentProvider::Codex) {
            self.start_codex_auth_check(cx);
        }
        self.model_option_scroll.set_offset(Point::default());
        self.model_picker_scroll.set_offset(Point::default());
        self.model_picker_query.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        let rows = self.model_picker_matches(cx);
        self.model_picker_index = rows
            .iter()
            .position(|m| m.id == self.model_id)
            .unwrap_or_else(|| {
                if self.model_picker_custom_match(cx).is_some() {
                    rows.len()
                } else {
                    0
                }
            });
        cx.notify();
    }

    pub(super) fn close_model_picker(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.model_picker_open {
            return;
        }
        self.model_picker_dropdown = None;
        self.model_picker_open = false;
        cx.notify();
        self.focus_composer(window, cx);
    }

    fn refresh_acp_models(&mut self) {
        self.picker_acp_instances.clear();
        let mut listing = RuntimeModels::default();
        match crate::views::acp_models::load() {
            Ok(instances) => {
                for instance in instances.into_iter().filter(|i| {
                    self.picker_availability
                        .as_ref()
                        .is_ok_and(|prefs| i.enabled(prefs))
                }) {
                    listing.models.extend(instance.choices);
                    self.picker_acp_instances
                        .insert(instance.id, (instance.name, instance.accent));
                }
            }
            Err(err) => listing.error = Some(format!("Could not read ACP agents: {err}")),
        }
        replace_listing(&mut self.runtime_models, AgentProvider::Acp, listing);
    }

    fn model_accent(&self, model: &str) -> Option<u32> {
        let (instance, _) = model.strip_prefix("acp/")?.split_once('/')?;
        self.picker_acp_instances
            .get(instance)
            .and_then(|(_, accent)| *accent)
    }

    fn picker_model_enabled(&self, model: &str) -> bool {
        let provider = AgentProvider::for_model(model);
        self.picker_provider_enabled(provider)
            && (provider != AgentProvider::Acp
                || provider
                    .model_id(model)
                    .split_once('/')
                    .is_some_and(|(id, _)| self.picker_acp_instances.contains_key(id)))
    }

    fn picker_provider_enabled(&self, provider: AgentProvider) -> bool {
        self.picker_availability
            .as_ref()
            .is_ok_and(|prefs| prefs.enabled(provider))
    }

    fn available_picker_models(&self) -> Vec<ModelChoice> {
        let mut rows = listed_models(&self.runtime_models);
        for model in &self.model_catalog.models {
            if !rows.iter().any(|row| row.id == model.id) {
                let end = rows
                    .iter()
                    .rposition(|row| row.provider == model.provider)
                    .map_or(rows.len(), |i| i + 1);
                rows.insert(
                    end,
                    ModelChoice {
                        id: model.id.clone(),
                        label: model.label.clone(),
                        provider: model.provider,
                    },
                );
            }
        }
        rows.retain(|row| self.picker_model_enabled(&row.id));
        rows
    }

    fn model_browser(&self, models: &[ModelChoice]) -> Browser {
        let custom = self
            .picker_model_enabled(&self.model_id)
            .then_some(self.model_id.as_str());
        Browser::new(
            models.iter().map(|m| m.id.as_str()).chain(custom),
            self.model_browser_tab.as_deref(),
            &self.model_id,
            &self.favorite_models,
        )
    }

    pub(super) fn model_picker_matches(&self, cx: &App) -> Vec<ModelChoice> {
        let query = self.model_picker_query.read(cx).value().to_string();
        let mut models = self.available_picker_models();
        if self.model_picker_open && query.trim().is_empty() {
            let browser = self.model_browser(&models);
            models.retain(|m| browser.includes(&m.id, &self.favorite_models));
            models.sort_by_key(|m| {
                !self
                    .favorite_models
                    .contains(&model_browser::favorite_key(&m.id))
            });
        }
        filter_models_like_web(&models, &query)
    }

    pub(super) fn model_picker_custom_match(&self, cx: &App) -> Option<String> {
        let query = self.model_picker_query.read(cx).value();
        let models = self.available_picker_models();
        if self.model_picker_open
            && query.trim().is_empty()
            && !self
                .model_browser(&models)
                .includes(&self.model_id, &self.favorite_models)
        {
            return None;
        }
        custom_model_match(query.as_ref(), &self.model_id, &models)
            .filter(|model| self.picker_model_enabled(model))
    }

    /// Ask each runtime that can list models for the account's; core caches
    /// the CLI's answer. Tests install listings with
    /// [`Self::apply_runtime_listing`] instead of running a CLI.
    fn refresh_runtime_models(&self, cx: &mut Context<'_, Self>) {
        let Some(RuntimeModelLister(lister)) = runtime_model_lister(cx) else {
            return;
        };
        for provider in bruhs_models::runtime_models::DISCOVERABLE_PROVIDERS {
            if !self.picker_provider_enabled(provider) {
                continue;
            }
            let listing = crate::app_state::spawn_tokio(
                cx,
                lister(provider, self.project_path.as_str().into()),
            );
            cx.spawn(async move |chat, cx| {
                let listing = listing.await;
                let _ = chat.update(cx, |chat, cx| {
                    chat.apply_runtime_listing(provider, listing, cx);
                });
            })
            .detach();
        }
    }

    /// Show `provider`'s listed models, keeping the highlight on the model it
    /// was on.
    pub(crate) fn apply_runtime_listing(
        &mut self,
        provider: AgentProvider,
        listing: RuntimeModels,
        cx: &mut Context<'_, Self>,
    ) {
        let rows = self.model_picker_matches(cx);
        let custom = self.model_picker_custom_match(cx);
        let count = rows.len() + usize::from(custom.is_some());
        let index = self.model_picker_index.min(count.saturating_sub(1));
        let previous = highlighted_id(&rows, custom.as_deref(), index).map(str::to_owned);
        if !replace_listing(&mut self.runtime_models, provider, listing) {
            return;
        }
        // Arriving rows never move the highlight to another model.
        let rows = self.model_picker_matches(cx);
        let custom = self.model_picker_custom_match(cx);
        self.model_picker_index =
            reanchor_highlight(previous.as_deref(), &rows, custom.as_deref(), index);
        if self.model_picker_open {
            self.model_picker_scroll
                .scroll_to_item(model_picker_row_index(&rows, self.model_picker_index));
        }
        cx.notify();
    }

    pub(super) fn select_model_from_picker(
        &mut self,
        model_id: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let model_id = model_id.into();
        self.set_model(model_id.clone(), cx);
        cx.emit(ChatViewEvent::SelectModel(model_id));
        self.close_model_picker(window, cx);
    }

    pub(super) fn select_highlighted_model(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let matches = self.model_picker_matches(cx);
        let custom = self.model_picker_custom_match(cx);
        let count = matches.len() + usize::from(custom.is_some());
        let index = self.model_picker_index.min(count.saturating_sub(1));
        if let Some(model) = matches.get(index) {
            self.select_model_from_picker(model.id.clone(), window, cx);
        } else if let Some(id) = custom {
            self.select_model_from_picker(id, window, cx);
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
                self.select_model_from_picker(model.id.clone(), window, cx);
            } else if n - 1 == matches.len() {
                if let Some(id) = self.model_picker_custom_match(cx) {
                    self.select_model_from_picker(id, window, cx);
                }
            }
            return true;
        }
        match key {
            "escape" => {
                self.close_model_picker(window, cx);
                true
            }
            "up" | "down" => {
                let count = self.model_picker_matches(cx).len()
                    + usize::from(self.model_picker_custom_match(cx).is_some());
                if count > 0 {
                    let current = self.model_picker_index.min(count - 1);
                    self.model_picker_index = if key == "up" {
                        (current + count - 1) % count
                    } else {
                        (current + 1) % count
                    };
                    self.model_picker_scroll
                        .scroll_to_item(model_picker_row_index(
                            &self.model_picker_matches(cx),
                            self.model_picker_index,
                        ));
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

    fn focus_highlighted_model_options(
        &mut self,
        _: &bruhs_ui::input::MoveRight,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let input = self.model_picker_query.read(cx);
        let selection = input.selected_range();
        if !selection.is_empty() || selection.end != input.value().len() {
            return;
        }
        let rows = self.model_picker_matches(cx);
        let custom = self.model_picker_custom_match(cx);
        if let Some(id) = highlighted_id(&rows, custom.as_deref(), self.model_picker_index) {
            if id != self.model_id {
                self.set_model(id.to_owned(), cx);
                cx.emit(ChatViewEvent::SelectModel(id.to_owned()));
            }
        }
        self.focus_model_choices(window, cx);
        cx.stop_propagation();
    }

    /// Web `model_picker.rs`: a 22rem floating card (longan-700, white/15
    /// ring) — search row with an `esc` chip, grouped rows (label over
    /// provider glyph + id, `⌘n` chip, check on the current one), and the
    /// provider's sign-in line in the footer.
    pub(super) fn render_model_picker(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let matches = self.model_picker_matches(cx);
        let custom = self.model_picker_custom_match(cx);
        let browser = self.model_browser(&self.available_picker_models());
        let browse = self.model_picker_query.read(cx).value().trim().is_empty();
        let view = cx.weak_entity();
        let rail = browse.then(|| {
            browser.rail(
                "desktop-model-picker",
                &self.model_picker_rail,
                self.model_picker_query.focus_handle(cx),
                |id| {
                    self.picker_acp_instances
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| (id.to_owned(), None))
                },
                move |tab, window, cx| {
                    let _ = view.update(cx, |chat, cx| {
                        chat.model_browser_tab = Some(tab);
                        chat.model_picker_index = 0;
                        chat.model_picker_scroll.set_offset(Point::default());
                        chat.model_picker_query
                            .update(cx, |field, cx| field.focus(window, cx));
                        cx.notify();
                    });
                },
            )
        });
        let count = matches.len() + usize::from(custom.is_some());
        let highlighted = self.model_picker_index.min(count.saturating_sub(1));
        let highlighted_provider = matches
            .get(highlighted)
            .map(|model| model.provider)
            .or_else(|| custom.as_deref().map(AgentProvider::for_model))
            .unwrap_or_else(|| AgentProvider::for_model(&self.model_id));
        let selected_row = model_picker_row_index(&matches, highlighted);
        let scroll = self.model_picker_scroll.clone();
        let previous_viewport = scroll.bounds().size;
        let view_id = cx.entity_id();
        let groups = group_models_like_web(&matches);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut flat_index = 0usize;
        for (provider, models) in groups {
            rows.push(
                div()
                    .flex_shrink_0()
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
                let key = element_key(&model.id);
                let model_id = model.id.clone();
                let favorite_id = model.id.clone();
                let hover_group = format!("desktop-model-picker-row-{}", model.id);
                let view = cx.weak_entity();
                let star = model_browser::star(
                    "desktop-model-picker",
                    &model.id,
                    self.favorite_models
                        .contains(&model_browser::favorite_key(&model.id)),
                    move |window, cx| {
                        let _ = view.update(cx, |chat, cx| {
                            cx.emit(ChatViewEvent::ToggleFavoriteModel(favorite_id.clone()));
                            chat.model_picker_query
                                .update(cx, |field, cx| field.focus(window, cx));
                        });
                    },
                );
                rows.push(
                    div()
                        .id(("desktop-model-picker-option", key))
                        .group(hover_group)
                        .debug_selector(move || format!("desktop-model-picker-option-{key}"))
                        .flex_shrink_0()
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
                        .aria_selected(is_highlighted)
                        .border_1()
                        .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .cursor_pointer()
                        .hover(|el| el.bg(gpui::white().opacity(0.06)))
                        .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                            if this.model_picker_index != index {
                                this.model_picker_index = index;
                                cx.notify();
                            }
                        }))
                        .map(|el| {
                            on_press_view(el, cx, move |this, window, cx| {
                                this.select_model_from_picker(model_id.clone(), window, cx);
                            })
                        })
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
                                        .child(model.label.clone()),
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
                                        .child(crate::views::workspace::provider_identity_mark(
                                            provider,
                                            self.model_accent(&model.id),
                                            cx,
                                        ))
                                        .child(div().min_w_0().truncate().child(model.id)),
                                ),
                        )
                        .child(star)
                        .when(index < 9, |el| {
                            el.child(
                                div()
                                    .flex_shrink_0()
                                    .rounded(px(4.0))
                                    .bg(theme::longan_800())
                                    .px_1()
                                    .text_size(px(MODEL_PICKER_KBD_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_400())
                                    .font_features(theme::tabular_nums())
                                    .child(format!("⌘{}", index + 1)),
                            )
                        })
                        .when(is_current, |el| {
                            el.child(
                                svg()
                                    .path(assets::CHECK_ICON_PATH)
                                    .size(px(theme::ICON_PX))
                                    .text_color(theme::rambutan_100())
                                    .flex_shrink_0(),
                            )
                        })
                        .into_any_element(),
                );
            }
        }
        if let Some(id) = custom.as_ref() {
            let id_click = id.clone();
            let index = matches.len();
            rows.push(
                div()
                    .id("desktop-model-picker-custom")
                    .debug_selector(|| "desktop-model-picker-custom".to_string())
                    .flex_shrink_0()
                    .px_2()
                    .py_2()
                    .rounded(px(MODEL_PICKER_RADIUS_PX))
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(index == highlighted, |el| {
                        el.bg(gpui::white().opacity(0.06))
                    })
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(format!("Use model {id}"))
                    .aria_selected(index == highlighted)
                    .cursor_pointer()
                    .hover(|el| el.bg(gpui::white().opacity(0.06)))
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if this.model_picker_index != index {
                            this.model_picker_index = index;
                            cx.notify();
                        }
                    }))
                    .map(|el| {
                        on_press_view(el, cx, move |this, window, cx| {
                            this.select_model_from_picker(id_click.clone(), window, cx);
                        })
                    })
                    .child(match self.model_accent(id) {
                        Some(color) => {
                            crate::views::workspace::accent_dot(color).into_any_element()
                        }
                        None => svg()
                            .path(assets::BOT_ICON_PATH)
                            .size(px(theme::ICON_PX))
                            .text_color(theme::rambutan_400())
                            .flex_shrink_0()
                            .into_any_element(),
                    })
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
                                    .child(format!("Use {id}")),
                            )
                            .child(
                                div()
                                    .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_400())
                                    .child("Use the model configured with this provider."),
                            ),
                    )
                    // Web checks the custom row while it is the current model.
                    .when(self.model_id == *id, |el| {
                        el.child(
                            svg()
                                .debug_selector(|| "desktop-model-picker-custom-check".to_string())
                                .path(assets::CHECK_ICON_PATH)
                                .size(px(theme::ICON_PX))
                                .text_color(theme::rambutan_100())
                                .flex_shrink_0(),
                        )
                    })
                    .into_any_element(),
            );
        }
        if matches.is_empty() && custom.is_none() {
            rows.push(
                div()
                    .debug_selector(|| "desktop-model-picker-empty".to_string())
                    .px_3()
                    .py_6()
                    .text_center()
                    .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .child(
                        self.picker_availability
                            .as_ref()
                            .err()
                            .cloned()
                            .unwrap_or_else(|| {
                                if browse && browser.active == FAVORITES {
                                    "No favorites yet"
                                } else {
                                    MODEL_PICKER_EMPTY_LABEL
                                }
                                .to_owned()
                            }),
                    )
                    .into_any_element(),
            );
        }

        let has_options = self.has_picker_options();
        let stacked = window.viewport_size().width < px(640.);
        let (column, footer) = if has_options {
            (
                self.picker_model_options(false, stacked, cx),
                self.picker_model_options(true, stacked, cx),
            )
        } else {
            (None, None)
        };
        let picker_width = match (column.is_some(), stacked) {
            (true, true) => 404.,
            (true, false) => MODEL_PICKER_WIDTH_PX + 190.,
            _ => MODEL_PICKER_WIDTH_PX,
        };
        let expanded = column.is_some();
        let rail_focus = self.model_picker_rail.clone();
        let options = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .debug_selector(|| "desktop-model-picker-search-row".to_string())
                    .when(has_options, |el| {
                        el.capture_action(cx.listener(Self::focus_highlighted_model_options))
                    })
                    .when(browse, |el| {
                        el.capture_action(move |_: &bruhs_ui::input::MoveLeft, window, cx| {
                            rail_focus.focus(window, cx);
                            cx.stop_propagation();
                        })
                    })
                    .flex_shrink_0()
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
                            .size(px(theme::ICON_PX))
                            .text_color(theme::rambutan_400())
                            .flex_shrink_0(),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.model_picker_query)
                                .appearance(false)
                                // Web: a bare <input> after the icon's gap.
                                .px_0()
                                .role(Role::EditableComboBox)
                                .aria_label(matches.get(highlighted).map_or_else(
                                    || {
                                        custom.as_ref().map_or_else(
                                            || "Search models. No models match.".to_string(),
                                            |id| format!("Search models. Use {id} selected"),
                                        )
                                    },
                                    |model| {
                                        format!(
                                            "Search models. {} selected, {} of {}",
                                            model.label,
                                            highlighted + 1,
                                            matches.len()
                                        )
                                    },
                                )),
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
                            .font_features(theme::tabular_nums())
                            .child("esc"),
                    ),
            )
            .child(
                div()
                    .on_children_prepainted(move |_, _, cx| {
                        // Reveal the highlight after resizing, without pulling
                        // pointer scrolling back to the selected row.
                        if scroll.bounds().size != previous_viewport {
                            let scroll = scroll.clone();
                            cx.defer(move |cx| {
                                scroll.scroll_to_item(selected_row);
                                cx.notify(view_id);
                            });
                        }
                    })
                    .id("desktop-model-picker-list")
                    .debug_selector(|| "desktop-model-picker-list".to_string())
                    .role(Role::ListBox)
                    .aria_label("Models")
                    .min_h_0()
                    .max_h(px(MODEL_PICKER_LIST_MAX_HEIGHT_PX))
                    .flex()
                    .flex_col()
                    .track_scroll(&self.model_picker_scroll)
                    .overflow_y_scroll()
                    .p_1()
                    .children(rows)
                    .vertical_scrollbar(&self.model_picker_scroll),
            );
        div()
            .id("desktop-model-picker")
            .debug_selector(|| "desktop-model-picker".to_string())
            .role(Role::Dialog)
            .aria_label(STATUS_CHANGE_MODEL_LABEL)
            .w(px(picker_width))
            .max_w(window.viewport_size().width - px(18.0))
            .max_h(super::composer_ui::composer_popover_max_height(
                window,
                self.composer_options_trigger_bounds.get(),
            ))
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
                    .min_h_0()
                    .when(expanded, |el| el.h(px(346.)))
                    .when(stacked, |el| el.flex_col())
                    .child(
                        div()
                            .flex()
                            .min_h_0()
                            .min_w_0()
                            .flex_1()
                            .children(rail)
                            .child(options),
                    )
                    .children(column),
            )
            .child(
                div()
                    .id("model-options-footer")
                    .debug_selector(|| "model-options-footer".into())
                    .p_1()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_t_1()
                    .border_color(theme::ring_floating())
                    .child(div().flex_1().min_w_0().children(footer))
                    .child(self.picker_permission_control(cx)),
            )
            .on_mouse_down_out(cx.listener(|chat, event: &MouseDownEvent, _, cx| {
                if chat.model_picker_dropdown.is_none()
                    && !chat.model_chip_bounds.get().contains(&event.position)
                {
                    chat.model_picker_open = false;
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .debug_selector(|| "desktop-model-picker-auth-status".to_string())
                    .border_color(gpui::white().opacity(0.10))
                    .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                    .font_family(theme::UI_SMALL_FONT_FAMILY)
                    .text_color(theme::rambutan_400())
                    .child(self.model_picker_status_line(highlighted_provider)),
            )
            .into_any_element()
    }

    /// Web `auth_line`: who the current provider is signed in as.
    pub(super) fn model_picker_status_line(&self, provider: AgentProvider) -> String {
        if let Some(line) = listing_error_line(&self.runtime_models, provider) {
            return line;
        }
        match provider {
            AgentProvider::Claude => model_picker_claude_line(
                self.auth_status.is_authenticated(),
                self.auth_status.user(),
            ),
            // Web `auth_line`; before the first check, the general line.
            AgentProvider::Codex => match &self.codex_auth {
                Some(status) if status.is_authenticated() => CODEX_SIGNED_IN_LINE.into(),
                Some(_) => CODEX_SIGNED_OUT_LINE.into(),
                None => "Codex uses your CLI login.".into(),
            },
            AgentProvider::Cursor => "Uses `cursor-agent login`. App tools are unavailable.".into(),
            AgentProvider::Antigravity => {
                "Requires Full access and Antigravity CLI sign-in. App tools are unavailable."
                    .into()
            }
            AgentProvider::Grok => "Uses Grok CLI login. App tools are unavailable.".into(),
            AgentProvider::OpenCode => "Configure providers and models in OpenCode.".into(),
            AgentProvider::Acp => "Authentication is managed by the configured ACP agent.".into(),
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

#[cfg(test)]
mod provider_model_tests {
    use super::{custom_model_match, listed_models};

    #[test]
    fn custom_models_accept_explicit_runtimes_and_survive_an_empty_search() {
        let all = listed_models(&[]);
        assert_eq!(
            custom_model_match("cursor/my-model", "gpt-5.5", &all),
            Some("cursor/my-model".into())
        );
        assert_eq!(
            custom_model_match("", "opencode/local/my-model", &all),
            Some("opencode/local/my-model".into())
        );
        assert_eq!(
            custom_model_match("local", "opencode/local/my-model", &all),
            Some("opencode/local/my-model".into())
        );
        assert!(custom_model_match("unknown/my-model", "gpt-5.5", &all).is_none());
        assert!(custom_model_match("cursor/default", "gpt-5.5", &all).is_none());
    }
}
