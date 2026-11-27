//! The scoped model preference shared by Projects and Threads.
use super::*;
use crate::views::model_browser::{self, Browser, FAVORITES};
use bruhs_core::agent::health::Status;
use bruhs_core::agent_extensions::availability::ProviderAvailability;
use bruhs_models::runtime_models::{listed_models, DISCOVERABLE_PROVIDERS};
use bruhs_ui::dropdown::{saved_option_value, Dropdown};

const ROW: &str = "desktop-settings-row-model";
const TYPED: &str = "typed-model-id";

impl Workspace {
    pub(super) fn toggle_favorite_model(&mut self, id: &str, cx: &mut Context<'_, Self>) {
        let key = model_browser::favorite_key(id);
        if !self.favorite_models.remove(&key) {
            self.favorite_models.insert(key);
        }
        for thread in &self.threads {
            thread.chat.update(cx, |chat, cx| {
                chat.replace_picker_favorites(&self.favorite_models, cx);
            });
        }
        self.sync_linked_model_favorites(cx);
        self.persist_and_notify(cx);
    }

    fn model_health(&self, model: &str) -> Option<Status> {
        let provider = AgentProvider::for_model(model);
        let key = if provider == AgentProvider::Acp {
            let (instance, _) = model.strip_prefix("acp/")?.split_once('/')?;
            &model[.."acp/".len() + instance.len()]
        } else {
            provider.as_str()
        };
        self.health_result
            .as_ref()?
            .iter()
            .find(|row| row.id == key)
            .map(|row| row.status)
    }

    pub(super) fn enabled_new_thread_model<'a>(
        &'a self,
        requested: &'a str,
        automatic: bool,
        prefs: &ProviderAvailability,
        instances: impl Fn() -> &'a [crate::views::acp_models::Instance],
    ) -> Option<&'a str> {
        let enabled = |model: &str| {
            if let Some(selection) = model.strip_prefix("acp/") {
                return selection.split_once('/').is_some_and(|(id, _)| {
                    instances().iter().any(|i| i.id == id && i.enabled(prefs))
                });
            }
            prefs.enabled(AgentProvider::for_model(model))
        };
        let catalog = || {
            [requested, self.current_model.as_str()]
                .into_iter()
                .chain(MODEL_CATALOG.iter().map(|m| m.id))
                .filter(|id| enabled(id))
        };
        let acp = || {
            instances()
                .iter()
                .filter(|i| i.enabled(prefs))
                .filter_map(|i| i.choices.first())
                .map(|m| m.id.as_str())
        };
        let chosen = if automatic && self.health_result.is_some() {
            let ready = |id: &&str| {
                matches!(
                    self.model_health(id),
                    Some(Status::SignedIn | Status::Ready | Status::Configured)
                )
            };
            let installed = |id: &&str| self.model_health(id) != Some(Status::NotInstalled);
            catalog()
                .find(ready)
                .or_else(|| acp().find(ready))
                .or_else(|| catalog().find(installed))
                .or_else(|| acp().find(installed))
        } else {
            catalog().next().or_else(|| acp().next())
        }?;
        let fallback = self.model_for_new_thread(chosen);
        Some(if enabled(fallback) { fallback } else { chosen })
    }

    pub(super) fn model_enabled(&self, id: &str) -> bool {
        self.provider_availability.as_ref().is_ok_and(|prefs| {
            if let Some(rest) = id.strip_prefix("acp/") {
                return rest.split_once('/').is_some_and(|(instance, _)| {
                    self.acp_models
                        .iter()
                        .any(|i| i.id == instance && i.enabled(prefs))
                });
            }
            prefs.enabled(AgentProvider::for_model(id))
        })
    }

    fn open_settings_models(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.toggle_settings_dropdown(SETTINGS_MODEL_DROPDOWN_ID, cx);
        self.settings_models_load = None;
        if self.settings_open_dropdown() != Some(SETTINGS_MODEL_DROPDOWN_ID) {
            return;
        }
        self.model_browser_tab = None;
        self.settings_model_query.update(cx, |field, cx| {
            field.set_value("", window, cx);
            field.focus(window, cx);
        });
        self.settings_model_listings.clear();
        let scope = self.settings_project_scope.clone();
        let root = PathBuf::from(&scope);
        let providers: Vec<_> = DISCOVERABLE_PROVIDERS
            .into_iter()
            .filter(|p| {
                self.provider_availability
                    .as_ref()
                    .is_ok_and(|prefs| prefs.enabled(*p))
            })
            .collect();
        let load = crate::app_state::spawn_tokio(cx, async move {
            let mut listings = Vec::new();
            for provider in providers {
                let result =
                    bruhs_core::agent::list_runtime_models_for_project(provider, &root).await;
                listings.push((provider, result));
            }
            listings
        });
        self.settings_models_load = Some(cx.spawn(async move |this, cx| {
            let listings = load.await;
            let _ = this.update(cx, |ws, cx| {
                if ws.settings_project_scope == scope
                    && ws.settings_open_dropdown() == Some(SETTINGS_MODEL_DROPDOWN_ID)
                {
                    ws.settings_model_listings = listings;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn choose_settings_model(
        &mut self,
        value: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let value = if value == TYPED {
            self.settings_model_query.read(cx).value().trim().to_owned()
        } else {
            value.to_owned()
        };
        let model = match value.as_str() {
            "inherit" => None,
            id if self.model_enabled(id)
                && (model_info(id).is_some()
                    || bruhs_models::agent::custom_model_selection(id).is_some()) =>
            {
                Some(value)
            }
            _ => return,
        };
        let picker_open = self.settings_open_dropdown() == Some(SETTINGS_MODEL_DROPDOWN_ID);
        self.set_settings_open_dropdown(None);
        if self.settings_project_scope.is_empty() {
            match model {
                Some(id) => self.set_default_model(id, cx),
                None => self.save_default_model(None, cx),
            }
        } else {
            let scope = &self.settings_project_scope;
            let prefs = self.project_defaults.entry(scope.clone()).or_default();
            if prefs.model != model {
                prefs.model_options = Default::default();
            }
            prefs.model = model;
            if *prefs == Default::default() {
                self.project_defaults.remove(scope);
            }
            self.persist_and_notify(cx);
        }
        if picker_open {
            self.settings_model_trigger.focus(window, cx);
        }
    }

    pub(super) fn new_thread_model_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let scoped = !self.settings_project_scope.is_empty();
        let project = self
            .project_defaults
            .get(&self.settings_project_scope)
            .and_then(|p| p.model.as_deref());
        let environment = self.explicit_model.then_some(self.current_model.as_str());
        let saved = project.or(environment);
        let automatic = self.provider_availability.as_ref().ok().and_then(|prefs| {
            self.enabled_new_thread_model(DEFAULT_AGENT_MODEL, true, prefs, || {
                self.acp_models.as_slice()
            })
        });
        let effective = saved.or(automatic);
        let mut groups = self.catalog_model_groups(cx, |_| true);
        for model in &self.thread_catalog.models {
            if model_info(&model.id).is_none() && self.model_enabled(&model.id) {
                if let Some(group) = groups
                    .iter_mut()
                    .find(|g| g.label == Some(provider_label(model.provider)))
                {
                    group.options.push(DropdownOption {
                        value: saved_option_value(&model.id),
                        label: model.label.clone(),
                        description: Some(model.id.clone()),
                    });
                }
            }
        }
        for choice in listed_models(&self.settings_model_listings)
            .into_iter()
            .filter(|m| model_info(&m.id).is_none() && self.model_enabled(&m.id))
        {
            if let Some(group) = groups
                .iter_mut()
                .find(|g| g.label == Some(provider_label(choice.provider)))
            {
                group.options.push(DropdownOption {
                    value: saved_option_value(&choice.id),
                    label: choice.label,
                    description: Some(choice.id),
                });
            }
        }
        if let Ok(prefs) = &self.provider_availability {
            for instance in self
                .acp_models
                .iter()
                .filter(|i| i.enabled(prefs) && !i.choices.is_empty())
            {
                groups.push(DropdownGroup {
                    label: Some(saved_option_value(&instance.name)),
                    mark: Some(provider_mark(AgentProvider::Acp, cx)),
                    options: instance
                        .choices
                        .iter()
                        .map(|m| DropdownOption {
                            value: saved_option_value(&m.id),
                            label: m.label.clone(),
                            description: Some(m.id.clone()),
                        })
                        .collect(),
                });
            }
        }
        let any_providers = effective.is_some() && !groups.is_empty();
        let is_open = self.settings_open_dropdown() == Some(SETTINGS_MODEL_DROPDOWN_ID);
        let query = self.settings_model_query.read(cx).value();
        let query = query.trim();
        let mut custom = Vec::new();
        for id in [environment, project].into_iter().flatten() {
            if self.model_enabled(id)
                && !groups
                    .iter()
                    .flat_map(|g| &g.options)
                    .chain(&custom)
                    .any(|o| o.value == id)
            {
                custom.push(DropdownOption {
                    value: saved_option_value(id),
                    label: id.to_owned(),
                    description: None,
                });
            }
        }
        if is_open
            && bruhs_models::agent::custom_model_selection(query).is_some()
            && self.model_enabled(query)
            && !groups
                .iter()
                .flat_map(|g| &g.options)
                .chain(&custom)
                .any(|o| o.value == query)
        {
            custom.push(DropdownOption {
                value: TYPED,
                label: format!("Use {query}"),
                description: Some(query.to_owned()),
            });
        }
        let custom_ids: Vec<_> = custom.iter().map(|option| option.value).collect();
        if !custom.is_empty() {
            groups.push(DropdownGroup {
                label: Some("Custom"),
                mark: None,
                options: custom,
            });
        }
        let label = |id: &str| {
            groups
                .iter()
                .flat_map(|g| &g.options)
                .find(|o| o.value == id)
                .map_or_else(
                    || model_info(id).map_or(id, |m| m.label).to_owned(),
                    |o| o.label.clone(),
                )
        };
        let env_label = environment.map(label);
        let effective_label = effective.map_or_else(|| "Automatic".to_owned(), label);
        let saved_here = if scoped { project } else { environment };
        let control = if any_providers {
            let inherit = if scoped {
                "Use environment default"
            } else {
                "Automatic"
            };
            let browser = Browser::new(
                groups
                    .iter()
                    .flat_map(|g| &g.options)
                    .map(|o| o.value)
                    .filter(|id| *id != TYPED),
                self.model_browser_tab.as_deref(),
                effective.unwrap_or_default(),
                &self.favorite_models,
            );
            let browse = is_open && query.is_empty();
            let ws = cx.weak_entity();
            let rail = browse.then(|| {
                browser.rail(
                    "desktop-model",
                    &self.settings_model_rail,
                    self.settings_model_query.focus_handle(cx),
                    |id| {
                        self.acp_models
                            .iter()
                            .find(|i| i.id == id)
                            .map_or_else(|| (id.to_owned(), None), |i| (i.name.clone(), i.accent))
                    },
                    move |tab, window, cx| {
                        let _ = ws.update(cx, |ws, cx| {
                            ws.model_browser_tab = Some(tab);
                            ws.settings_model_query
                                .update(cx, |field, cx| field.focus(window, cx));
                            cx.notify();
                        });
                    },
                )
            });
            if browse {
                browser.filter_groups(&mut groups, &self.favorite_models);
            }
            for option in groups.iter_mut().flat_map(|g| &mut g.options) {
                if option.description.as_deref() == Some(option.value) {
                    option.description = None;
                }
            }
            let favorites_tab = browse && browser.active == FAVORITES;
            if !favorites_tab {
                groups.push(DropdownGroup {
                    label: None,
                    mark: None,
                    options: vec![DropdownOption {
                        value: "inherit",
                        label: inherit.to_owned(),
                        description: None,
                    }],
                });
            }
            let ws = cx.weak_entity();
            let stars = self.favorite_models.clone();
            let active = browser.active;
            let accessory = move |id: &str| {
                if id == "inherit" || custom_ids.contains(&id) {
                    return None;
                }
                let id = id.to_owned();
                let model = id.clone();
                let ws = ws.clone();
                let active = active.clone();
                Some(model_browser::star(
                    "desktop-model",
                    &id,
                    stars.contains(&model_browser::favorite_key(&id)),
                    None,
                    move |window, cx| {
                        let _ = ws.update(cx, |ws, cx| {
                            ws.model_browser_tab = Some(active.clone());
                            ws.toggle_favorite_model(&model, cx);
                            ws.settings_model_query
                                .update(cx, |field, cx| field.focus(window, cx));
                        });
                    },
                ))
            };
            let ws = cx.weak_entity();
            let toggle: ToggleHandler = Rc::new(move |window, cx| {
                let _ = ws.update(cx, |ws, cx| ws.open_settings_models(window, cx));
            });
            let (_, select) = self.settings_dropdown_handlers(SETTINGS_MODEL_DROPDOWN_ID, cx);
            let picker = Dropdown::new(
                SETTINGS_MODEL_DROPDOWN_ID,
                "Default model",
                saved_here.unwrap_or("inherit"),
                groups,
                self.settings_listbox(SETTINGS_MODEL_DROPDOWN_ID),
                toggle,
                select,
            )
            .option_mark(|id| {
                if matches!(id, "inherit" | TYPED) {
                    return None;
                }
                Some(provider_glyph(AgentProvider::for_model(id)).into_any_element())
            })
            .option_accessories(
                rail.map(|el| (el, self.settings_model_rail.clone())),
                accessory,
            )
            .empty_label(if favorites_tab {
                "No favorites yet"
            } else {
                "No models found"
            })
            .end_aligned_popover(px(360.))
            .selection_label(effective_label, None)
            .searchable(
                &self.settings_model_query,
                "Search models",
                &self.settings_model_trigger,
            );
            let picker = if let Some(model) = effective {
                picker.leading_icon(assets::provider_logo_path(AgentProvider::for_model(model)))
            } else {
                picker
            };
            self.with_model_options(
                effective.unwrap_or(DEFAULT_AGENT_MODEL),
                false,
                picker.into_any_element(),
                cx,
            )
        } else {
            div()
                .text_sm()
                .text_color(theme::rambutan_400())
                .child("No providers available")
                .into_any_element()
        };
        let actions = self.setting_source_actions(
            project_defaults::DefaultModel,
            SETTINGS_MODEL_DROPDOWN_ID,
            ROW,
            "Default model",
            env_label.map(Into::into),
            "Automatic",
            saved_here.is_some(),
            cx,
        );
        let status = if let Err(error) = &self.provider_availability {
            Some(error.to_owned())
        } else {
            saved
                .filter(|id| !self.model_enabled(id))
                .map(|id| format!("{} is unavailable for this scope.", id))
        };
        self.settings_row_with_reset(
            ROW,
            "Default model",
            if scoped {
                "Default model for new threads in this project."
            } else {
                "Default model for new threads."
            },
            status.map(Into::into),
            false,
            Some(
                div()
                    .w(px(self.model_control_width(effective)))
                    .min_w_0()
                    .max_w_full()
                    .flex_grow(1.0)
                    .child(control)
                    .into_any_element(),
            ),
            Some(actions),
        )
    }
}
