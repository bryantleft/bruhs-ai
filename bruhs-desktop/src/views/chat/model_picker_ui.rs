//! The model picker popover: filtering, grouping by provider, keyboard picking, and the provider mark.

use super::*;
use crate::views::model_browser::{self, Browser, Favorites, FAVORITES};
use bruhs_models::delivery::writer_catalog::WriterCatalog;
use bruhs_models::logic::cli_login::CODEX_SIGNED_OUT_LINE;
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
const MODEL_PICKER_LIST_WIDTH_PX: f32 = 360.0;
const MODEL_PICKER_BODY_HEIGHT_PX: f32 = 346.0;
const MODEL_PICKER_ROW_HEIGHT_PX: f32 = 34.0;
pub(super) const MODEL_PICKER_OPTIONS_WIDTH_PX: f32 = 192.0;
pub(super) const MODEL_PICKER_RADIUS_PX: f32 = 8.0;
pub(super) const MODEL_PICKER_ROW_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const MODEL_PICKER_LABEL_TEXT_SIZE_PX: f32 = theme::APP_UI_TEXT_SIZE_PX;
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

#[cfg(test)]
pub fn filter_models_like_web(models: &[ModelChoice], query: &str) -> Vec<ModelChoice> {
    rank_model_matches(models, query, &Default::default())
}

fn rank_model_matches(
    models: &[ModelChoice],
    query: &str,
    favorites: &Favorites,
) -> Vec<ModelChoice> {
    let query = query.to_lowercase();
    let words: Vec<_> = query.split_whitespace().collect();
    if words.is_empty() {
        return models.to_vec();
    }
    let query = words.join(" ");
    let mut ranked: Vec<_> = models
        .iter()
        .filter_map(|model| {
            let fields = [
                model.label.to_lowercase(),
                model.id.to_lowercase(),
                provider_label(model.provider).to_lowercase(),
                model.provider.as_str().to_owned(),
            ];
            let score = if fields[..2].contains(&query) {
                0
            } else {
                let mut total = 1;
                for word in &words {
                    total += fields
                        .iter()
                        .enumerate()
                        .filter_map(|(index, field)| {
                            model_search_score(field, word).map(|score| score + index)
                        })
                        .min()?;
                }
                total
            };
            let favorite = favorites.contains(&model_browser::favorite_key(&model.id));
            Some((score, !favorite, model))
        })
        .collect();
    ranked.sort_by_key(|&(score, not_favorite, _)| (score, not_favorite));
    ranked
        .into_iter()
        .map(|(_, _, model)| model.clone())
        .collect()
}

fn model_search_score(field: &str, word: &str) -> Option<usize> {
    if field == word {
        return Some(0);
    }
    if field.starts_with(word) {
        return Some(4);
    }
    if field.contains(word) {
        return Some(8);
    }
    if word.contains('/') || word.chars().count() < 3 {
        return None;
    }
    let mut chars = field.chars();
    word.chars()
        .all(|letter| chars.any(|candidate| letter == candidate))
        .then_some(32)
}

fn model_picker_shortcut(index: usize) -> Option<Div> {
    (index < 9).then(|| bruhs_ui::kbd::kbd(format!("⌘{}", index + 1)))
}

struct PickerModels {
    rows: Vec<ModelChoice>,
    /// The presence of a disclosure is independent of its expanded state.
    older: Option<bool>,
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
        self.model_picker_opening_model.clone_from(&self.model_id);
        self.model_picker_expanded.clear();
        self.model_picker_show_highlight = false;
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
        let live_models = &self.model_catalog.models;
        let mut rows: Vec<_> = self
            .model_catalog
            .models
            .iter()
            .map(|model| ModelChoice {
                id: model.id.clone(),
                label: model.label.clone(),
                provider: model.provider,
            })
            .collect();
        // A native catalog already contains its own bundled fallback when a
        // probe fails. Keep a thread's current model visible across refreshes.
        rows.extend(
            listed_models(&self.runtime_models)
                .into_iter()
                .filter(|row| {
                    let catalog = &self.model_catalog.models;
                    !catalog.iter().any(|model| model.id == row.id)
                        && (row.provider.is_external()
                            || row.id == self.model_id
                            || !catalog.iter().any(|model| model.provider == row.provider))
                }),
        );
        rows.retain(|row| self.picker_model_enabled(&row.id));
        // The bundled undated Claude ID can name the same release as a dated
        // CLI row. Keep the actual CLI choice and any saved, explicit selection.
        rows.retain(|row| {
            if row.provider != AgentProvider::Claude
                || row.id == self.model_id
                || self
                    .favorite_models
                    .contains(&model_browser::favorite_key(&row.id))
            {
                return true;
            }
            let id = row.provider.model_id(&row.id);
            !live_models.iter().any(|reported| {
                reported.provider == row.provider
                    && row
                        .provider
                        .model_id(&reported.id)
                        .rsplit_once('-')
                        .is_some_and(|(base, date)| {
                            base == id
                                && date.len() == 8
                                && date.bytes().all(|b| b.is_ascii_digit())
                        })
            })
        });
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

    fn model_picker_rows(&self, cx: &App) -> PickerModels {
        let query = self.model_picker_query.read(cx).value().to_string();
        let mut models = self.available_picker_models();
        if !self.model_picker_open || !query.trim().is_empty() {
            return PickerModels {
                rows: rank_model_matches(&models, &query, &self.favorite_models),
                older: None,
            };
        }
        let browser = self.model_browser(&models);
        models.retain(|m| browser.includes(&m.id, &self.favorite_models));
        if browser.active == FAVORITES {
            return PickerModels {
                rows: models,
                older: None,
            };
        }
        let favorite = |m: &ModelChoice| {
            self.favorite_models
                .contains(&model_browser::favorite_key(&m.id))
        };
        models.sort_by_key(|m| !favorite(m));
        let (mut rows, older): (Vec<_>, Vec<_>) = models.into_iter().partition(|m| {
            favorite(m) || !self.model_catalog.model(&m.id).is_some_and(|m| m.is_legacy)
        });
        let expanded = (!older.is_empty()).then(|| {
            self.model_picker_expanded
                .get(&browser.active)
                .copied()
                .unwrap_or_else(|| {
                    older
                        .iter()
                        .any(|m| m.id == self.model_picker_opening_model)
                })
        });
        if expanded == Some(true) {
            rows.extend(older);
        }
        PickerModels {
            rows,
            older: expanded,
        }
    }

    pub(super) fn model_picker_matches(&self, cx: &App) -> Vec<ModelChoice> {
        self.model_picker_rows(cx).rows
    }

    fn toggle_older_picker_models(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let before = self.model_picker_rows(cx);
        let Some(expanded) = before.older else { return };
        let tab = self.model_browser(&self.available_picker_models()).active;
        self.model_picker_expanded.insert(tab, !expanded);
        self.model_picker_index = if expanded {
            self.model_picker_matches(cx).len()
                + usize::from(self.model_picker_custom_match(cx).is_some())
        } else {
            before.rows.len()
        };
        self.model_picker_show_highlight = true;
        self.model_picker_scroll
            .scroll_to_item(self.model_picker_index);
        self.model_picker_query
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    pub(super) fn model_picker_browsing(&self, cx: &App) -> bool {
        self.model_picker_query.read(cx).value().trim().is_empty()
    }

    fn browse_model_provider(
        &mut self,
        tab: String,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.model_browser_tab = Some(tab);
        self.model_picker_dropdown = None;
        self.model_picker_index = 0;
        self.model_picker_show_highlight = false;
        self.model_picker_scroll.set_offset(Point::default());
        self.model_picker_query.update(cx, |field, cx| {
            field.set_value("", window, cx);
            field.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn cycle_model_picker_provider(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let next = self
            .model_browser(&self.available_picker_models())
            .adjacent_tab(forward);
        self.browse_model_provider(next, window, cx);
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
        let previous = self.picker_highlight(cx);
        if !replace_listing(&mut self.runtime_models, provider, listing) {
            return;
        }
        self.restore_picker_highlight(previous.as_deref(), cx);
        cx.notify();
    }

    pub(crate) fn replace_model_catalog(
        &mut self,
        catalog: std::sync::Arc<WriterCatalog>,
        cx: &mut Context<'_, Self>,
    ) {
        let previous = self.picker_highlight(cx);
        self.model_catalog = catalog;
        self.restore_picker_highlight(previous.as_deref(), cx);
        cx.notify();
    }

    fn picker_highlight(&self, cx: &App) -> Option<String> {
        if !self.model_picker_open {
            return None;
        }
        let rows = self.model_picker_matches(cx);
        let custom = self.model_picker_custom_match(cx);
        highlighted_id(&rows, custom.as_deref(), self.model_picker_index).map(str::to_owned)
    }

    fn restore_picker_highlight(&mut self, previous: Option<&str>, cx: &App) {
        if !self.model_picker_open {
            return;
        }
        let picker = self.model_picker_rows(cx);
        let custom = self.model_picker_custom_match(cx);
        let selectable = picker.rows.len() + usize::from(custom.is_some());
        self.model_picker_index = if previous.is_none() && picker.older.is_some() {
            self.model_picker_index.min(selectable)
        } else {
            reanchor_highlight(
                previous,
                &picker.rows,
                custom.as_deref(),
                self.model_picker_index,
            )
        };
        self.model_picker_scroll
            .scroll_to_item(self.model_picker_index);
    }

    pub(in crate::views) fn replace_picker_favorites(
        &mut self,
        favorites: &Favorites,
        cx: &mut Context<'_, Self>,
    ) {
        let previous = self.picker_highlight(cx);
        self.favorite_models.clone_from(favorites);
        self.restore_picker_highlight(previous.as_deref(), cx);
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
        self.model_picker_index = self
            .model_picker_matches(cx)
            .iter()
            .position(|model| model.id == self.model_id)
            .unwrap_or(self.model_picker_index);
        if self.has_picker_options() {
            self.model_option_scroll.set_offset(Point::default());
            self.focus_model_choices(window, cx);
        } else {
            self.model_picker_query
                .update(cx, |field, cx| field.focus(window, cx));
        }
    }

    pub(super) fn select_highlighted_model(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let PickerModels {
            rows: matches,
            older,
        } = self.model_picker_rows(cx);
        let custom = self.model_picker_custom_match(cx);
        let count = matches.len() + usize::from(custom.is_some()) + usize::from(older.is_some());
        let index = self.model_picker_index.min(count.saturating_sub(1));
        if let Some(model) = matches.get(index) {
            self.select_model_from_picker(model.id.clone(), window, cx);
        } else if let Some(id) = custom.filter(|_| index == matches.len()) {
            self.select_model_from_picker(id, window, cx);
        } else if older.is_some() {
            self.toggle_older_picker_models(window, cx);
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
                let picker = self.model_picker_rows(cx);
                let count = picker.rows.len()
                    + usize::from(self.model_picker_custom_match(cx).is_some())
                    + usize::from(picker.older.is_some());
                if count > 0 {
                    let current = self.model_picker_index.min(count - 1);
                    self.model_picker_index = if key == "up" {
                        (current + count - 1) % count
                    } else {
                        (current + 1) % count
                    };
                    self.model_picker_show_highlight = true;
                    self.model_picker_scroll
                        .scroll_to_item(self.model_picker_index);
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
        let Some(id) = highlighted_id(&rows, custom.as_deref(), self.model_picker_index) else {
            return;
        };
        if !self.has_picker_options_for(id) {
            return;
        }
        if id == self.model_id {
            self.focus_model_choices(window, cx);
        } else {
            self.select_model_from_picker(id.to_owned(), window, cx);
        }
        cx.stop_propagation();
    }

    /// Composer dock with provider navigation, models, and controls for the selected model.
    pub(super) fn render_model_picker(
        &self,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let PickerModels {
            rows: matches,
            older,
        } = self.model_picker_rows(cx);
        let custom = self.model_picker_custom_match(cx);
        let browser = self.model_browser(&self.available_picker_models());
        let browse = self.model_picker_browsing(cx);
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
                        chat.browse_model_provider(tab, window, cx);
                    });
                },
            )
        });
        let model_count = matches.len() + usize::from(custom.is_some());
        let count = model_count + usize::from(older.is_some());
        let highlighted = self.model_picker_index.min(count.saturating_sub(1));
        let search_focused = self.model_picker_query.focus_handle(cx).is_focused(window);
        let highlighted_provider = matches
            .get(highlighted)
            .map(|model| model.provider)
            .or_else(|| custom.as_deref().map(AgentProvider::for_model))
            .unwrap_or_else(|| AgentProvider::for_model(&self.model_id));
        let selected_row = highlighted;
        let scroll = self.model_picker_scroll.clone();
        let previous_viewport = scroll.bounds().size;
        let view_id = cx.entity_id();
        let mut rows: Vec<AnyElement> = Vec::new();
        for (index, model) in matches.iter().enumerate() {
            let is_current = self.model_id == model.id;
            let is_highlighted = self.model_picker_show_highlight && index == highlighted;
            let key = element_key(&model.id);
            let row_group: SharedString = format!("model-picker-row-{key}").into();
            let model_id = model.id.clone();
            let favorite_id = model.id.clone();
            let view = cx.weak_entity();
            let favorite = self
                .favorite_models
                .contains(&model_browser::favorite_key(&model.id));
            let star = model_browser::star(
                "desktop-model-picker",
                &model.id,
                favorite,
                (!is_highlighted && (!favorite || (browse && browser.active == FAVORITES)))
                    .then(|| row_group.clone()),
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
                    .debug_selector(move || format!("desktop-model-picker-option-{key}"))
                    .group(row_group)
                    .flex_shrink_0()
                    .h(px(MODEL_PICKER_ROW_HEIGHT_PX))
                    .px_2()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(is_highlighted, |el| el.bg(theme::longan_800()))
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(format!("Select model {}", model.label))
                    .aria_description(provider_label(model.provider))
                    .aria_selected(is_current)
                    .border_1()
                    .border_color(gpui::hsla(0.0, 0.0, 0.0, 0.0))
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if this.model_picker_index != index || !this.model_picker_show_highlight {
                            this.model_picker_index = index;
                            this.model_picker_show_highlight = true;
                            cx.notify();
                        }
                    }))
                    .map(|el| {
                        on_press_view(el, cx, move |this, window, cx| {
                            this.select_model_from_picker(model_id.clone(), window, cx);
                        })
                    })
                    .child(model_browser::picker_provider_mark(
                        model.provider,
                        self.model_accent(&model.id),
                        16.,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(MODEL_PICKER_LABEL_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_100())
                                    .child(model.label.clone()),
                            )
                            .when(is_current, |el| {
                                el.child(
                                    svg()
                                        .path(assets::CHECK_ICON_PATH)
                                        .size(px(theme::ICON_INLINE_PX))
                                        .flex_none()
                                        .text_color(theme::persimmon_400()),
                                )
                            }),
                    )
                    .child(star)
                    .children(model_picker_shortcut(index))
                    .into_any_element(),
            );
        }
        if let Some(id) = custom.as_ref() {
            let id_click = id.clone();
            let index = matches.len();
            rows.push(
                div()
                    .id("desktop-model-picker-custom")
                    .debug_selector(|| "desktop-model-picker-custom".to_string())
                    .flex_shrink_0()
                    .h(px(MODEL_PICKER_ROW_HEIGHT_PX))
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .when(
                        self.model_picker_show_highlight && index == highlighted,
                        |el| el.bg(theme::longan_800()),
                    )
                    .tab_index(0)
                    .role(Role::ListBoxOption)
                    .aria_label(format!("Use model {id}"))
                    .aria_selected(self.model_id == *id)
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .focus_visible(|el| el.border_color(theme::persimmon_400()))
                    .cursor_pointer()
                    .hover(|el| el.bg(theme::longan_800()))
                    .on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| {
                        if this.model_picker_index != index || !this.model_picker_show_highlight {
                            this.model_picker_index = index;
                            this.model_picker_show_highlight = true;
                            cx.notify();
                        }
                    }))
                    .map(|el| {
                        on_press_view(el, cx, move |this, window, cx| {
                            this.select_model_from_picker(id_click.clone(), window, cx);
                        })
                    })
                    .child(model_browser::picker_provider_mark(
                        AgentProvider::for_model(id),
                        self.model_accent(id),
                        16.,
                    ))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(MODEL_PICKER_LABEL_TEXT_SIZE_PX))
                                    .text_color(theme::rambutan_100())
                                    .child(format!("Use {id}")),
                            )
                            .when(self.model_id == *id, |el| {
                                el.child(
                                    svg()
                                        .debug_selector(|| {
                                            "desktop-model-picker-custom-check".to_string()
                                        })
                                        .path(assets::CHECK_ICON_PATH)
                                        .size(px(theme::ICON_INLINE_PX))
                                        .text_color(theme::persimmon_400())
                                        .flex_none(),
                                )
                            }),
                    )
                    .children(model_picker_shortcut(index))
                    .into_any_element(),
            );
        }
        if let Some(expanded) = older {
            let label = if expanded { "Show less" } else { "Show more" };
            rows.push(
                on_press_view(
                    div()
                        .id("desktop-model-picker-disclosure")
                        .debug_selector(|| "desktop-model-picker-disclosure".into())
                        .h(px(MODEL_PICKER_ROW_HEIGHT_PX))
                        .flex_none()
                        .flex()
                        .items_center()
                        .px_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(gpui::transparent_black())
                        .when(
                            self.model_picker_show_highlight && highlighted == model_count,
                            |el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()),
                        )
                        .hover(|el| el.bg(theme::longan_800()).text_color(theme::rambutan_100()))
                        .focus_visible(|el| el.border_color(theme::persimmon_400()))
                        .text_color(theme::rambutan_400())
                        .text_size(px(MODEL_PICKER_LABEL_TEXT_SIZE_PX))
                        .tab_index(0)
                        .role(Role::Button)
                        .aria_label(label)
                        .aria_expanded(expanded)
                        .cursor_pointer()
                        .child(label),
                    cx,
                    Self::toggle_older_picker_models,
                )
                .into_any_element(),
            );
        }
        if count == 0 {
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

        let stacked = window.viewport_size().width < px(640.);
        let (column, footer) = if self.has_picker_options() {
            (
                self.picker_option_column(stacked, cx),
                self.picker_model_footer(cx),
            )
        } else {
            (None, None)
        };
        let column = column.unwrap_or_else(|| {
            self.model_options_column(stacked, cx)
                .child(
                    div()
                        .debug_selector(|| "desktop-model-picker-options-heading".into())
                        .h(px(model_browser::HEADER_HEIGHT_PX))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .px_2()
                        .border_b_1()
                        .border_color(theme::ring_card())
                        .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                        .text_color(theme::rambutan_400())
                        .child("No model options"),
                )
                .into_any_element()
        });
        let picker_width = if stacked {
            404.
        } else {
            MODEL_PICKER_LIST_WIDTH_PX + MODEL_PICKER_OPTIONS_WIDTH_PX
        };
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
                    .capture_action(cx.listener(Self::focus_highlighted_model_options))
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
                    .mx_3()
                    .h(px(model_browser::HEADER_HEIGHT_PX))
                    .border_b_1()
                    .border_color(if search_focused {
                        theme::persimmon_400()
                    } else {
                        theme::ring_card()
                    })
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
                                        if older.is_some() && highlighted == model_count {
                                            format!(
                                                "Search models. {} highlighted",
                                                if older == Some(true) {
                                                    "Show less"
                                                } else {
                                                    "Show more"
                                                }
                                            )
                                        } else {
                                            custom.as_ref().map_or_else(
                                                || "Search models. No models match.".to_string(),
                                                |id| format!("Search models. Use {id} selected"),
                                            )
                                        }
                                    },
                                    |model| {
                                        format!(
                                            "Search models. {} highlighted, {} of {}",
                                            model.label,
                                            highlighted + 1,
                                            matches.len()
                                        )
                                    },
                                )),
                        ),
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
                    .flex_1()
                    .flex()
                    .flex_col()
                    .track_scroll(&self.model_picker_scroll)
                    .overflow_y_scroll()
                    .px_3()
                    .py_2()
                    .children(rows),
            )
            .when_some(
                self.model_picker_notice(highlighted_provider),
                |el, notice| {
                    el.child(
                        div()
                            .id("desktop-model-picker-provider-notice")
                            .debug_selector(|| "desktop-model-picker-provider-notice".into())
                            .role(Role::Status)
                            .flex_shrink_0()
                            .px_2()
                            .py_1()
                            .border_t_1()
                            .border_color(theme::ring_card())
                            .text_size(px(MODEL_PICKER_ROW_TEXT_SIZE_PX))
                            .text_color(theme::rambutan_400())
                            .line_clamp(2)
                            .child(notice),
                    )
                },
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
                self.model_chip_bounds.get(),
            ))
            .rounded(px(MODEL_PICKER_RADIUS_PX))
            .border_1()
            .border_color(theme::ring_floating())
            .bg(theme::longan_950())
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .min_h_0()
                    .h(px(MODEL_PICKER_BODY_HEIGHT_PX))
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
                    .child(column),
            )
            .child(
                div()
                    .id("model-options-footer")
                    .debug_selector(|| "model-options-footer".into())
                    .px_3()
                    .py_1()
                    .min_h(px(44.))
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
            .into_any_element()
    }

    pub(super) fn model_picker_notice(&self, provider: AgentProvider) -> Option<String> {
        listing_error_line(&self.runtime_models, provider).or_else(|| match provider {
            AgentProvider::Claude if !self.auth_status.is_authenticated() => {
                Some("Sign in to Claude Code to use these models.".into())
            }
            AgentProvider::Codex
                if self
                    .codex_auth
                    .as_ref()
                    .is_some_and(|status| !status.is_authenticated()) =>
            {
                Some(CODEX_SIGNED_OUT_LINE.into())
            }
            _ => None,
        })
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
            .child(model_browser::picker_provider_mark(
                provider,
                self.model_accent(&self.model_id),
                assets::PROVIDER_LOGO_SIZE_PX,
            ))
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
