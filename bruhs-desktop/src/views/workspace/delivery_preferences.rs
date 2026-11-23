use super::*;
use bruhs_models::delivery::{
    DeliveryPreferenceChange, DeliveryPreferences, MergeMethod, SetDeliveryPreference, WritingStyle,
};

fn friendly_delivery_error(error: &impl std::fmt::Display) -> String {
    settings_ui::friendly_server_error(error)
}

#[derive(Default)]
pub(super) struct HostPreferences {
    pub(super) catalog: bruhs_models::delivery::writer_catalog::WriterCatalog,
    loading_models: bool,
    value: Option<DeliveryPreferences>,
    pub(super) error: Option<String>,
    revision: u64,
    pub(super) saving: bool,
    pending: std::collections::VecDeque<SetDeliveryPreference>,
    pub(super) guide_editor: Option<super::source_writer::InstructionsEditor>,
    pub(super) editor: Option<super::source_writer::InstructionsEditor>,
}
impl Workspace {
    #[cfg(test)]
    pub(super) fn set_delivery_preferences_for_test(&mut self, value: DeliveryPreferences) {
        self.delivery_preferences.revision = self.delivery_preferences.revision.wrapping_add(1);
        self.delivery_preferences.value = Some(value);
    }
    fn delivery_settings_scope(&self) -> &str {
        if self.settings_section == SettingsSection::Updates {
            ""
        } else {
            &self.settings_project_scope
        }
    }
    fn refresh_writer_models(&mut self, cx: &mut Context<'_, Self>) {
        if self.delivery_preferences.loading_models {
            return;
        }
        self.delivery_preferences.loading_models = true;
        let scope = self.delivery_settings_scope().to_owned();
        let project = scope.clone();
        let request = crate::app_state::spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .writer_models(&project)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                this.delivery_preferences.loading_models = false;
                if scope != this.delivery_settings_scope() {
                    this.refresh_writer_models(cx);
                    return;
                }
                if let Ok(catalog) = result {
                    this.delivery_preferences.catalog = catalog;
                    cx.notify();
                }
            });
        })
        .detach();
    }
    pub(super) fn load_delivery_preferences(&mut self, cx: &mut Context<'_, Self>) {
        if self.settings_section == SettingsSection::SourceControl {
            self.refresh_writer_models(cx);
        }
        let state = &mut self.delivery_preferences;
        state.value = None;
        state.error = None;
        state.revision = state.revision.wrapping_add(1);
        let revision = state.revision;
        if state.saving {
            cx.notify();
            return;
        }
        let path = (!self.delivery_settings_scope().is_empty())
            .then(|| self.delivery_settings_scope().to_owned());
        let request = crate::app_state::spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .delivery_preferences(path)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if this.delivery_preferences.revision != revision {
                    return;
                }
                match result {
                    Ok(value) => this.delivery_preferences.value = Some(value),
                    Err(error) => {
                        this.delivery_preferences.error = Some(friendly_delivery_error(&error))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn save_delivery_preference(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) {
        use bruhs_models::delivery::{generation::GenerationTarget, GuideModel};
        let Some(prefs) = self.loaded_delivery_preferences() else {
            return;
        };
        let change = match (id, value) {
            ("settings-guide-options" | "settings-writer-options", token) => {
                let target = if id == "settings-guide-options" {
                    GenerationTarget::ReviewGuide
                } else {
                    GenerationTarget::Writer
                };
                let Some((model, _)) = target.selection(prefs) else {
                    return;
                };
                let Some((key, value)) = token.split_once('=') else {
                    return;
                };
                let Some(spec) = self
                    .delivery_preferences
                    .catalog
                    .model(model)
                    .into_iter()
                    .flat_map(|m| &m.options)
                    .find(|o| o.kind.key() == key)
                else {
                    return;
                };
                DeliveryPreferenceChange::GenerationOption {
                    target,
                    model: model.to_owned(),
                    kind: spec.kind,
                    value: value.to_owned(),
                }
            }
            ("settings-review-guides", "on") => {
                DeliveryPreferenceChange::ReviewGuidesEnabled(Some(true))
            }
            ("settings-review-guides", "off") => {
                DeliveryPreferenceChange::ReviewGuidesEnabled(Some(false))
            }
            ("settings-review-guides", "inherit") => {
                DeliveryPreferenceChange::ReviewGuidesEnabled(None)
            }
            ("settings-guide-instructions", "inherit") => {
                DeliveryPreferenceChange::ReviewGuideInstructions(None)
            }
            ("settings-guide-model", "inherit") => DeliveryPreferenceChange::ReviewGuideModel(None),
            ("settings-guide-model", "writer") => {
                DeliveryPreferenceChange::ReviewGuideModel(Some(GuideModel::Writer))
            }
            ("settings-guide-model", model)
                if self.delivery_preferences.catalog.model(model).is_some() =>
            {
                DeliveryPreferenceChange::ReviewGuideModel(Some(GuideModel::Separate {
                    model: model.to_owned(),
                    options: GenerationTarget::ReviewGuide
                        .selection(prefs)
                        .or_else(|| GenerationTarget::Writer.selection(prefs))
                        .filter(|(old, _)| *old == model)
                        .map(|(_, options)| options.clone())
                        .unwrap_or_default(),
                }))
            }
            ("settings-writing-style", "repository") => {
                DeliveryPreferenceChange::WritingStyle(Some(WritingStyle::Repository))
            }
            ("settings-writing-style", "conventional") => {
                DeliveryPreferenceChange::WritingStyle(Some(WritingStyle::Conventional))
            }
            ("settings-writing-style", "custom") => {
                DeliveryPreferenceChange::WritingStyle(Some(WritingStyle::Custom))
            }
            ("settings-writing-style", "inherit") => DeliveryPreferenceChange::ResetWritingStyle,
            ("settings-writer-model", "inherit") => DeliveryPreferenceChange::WriterModel(None),
            ("settings-writer-model", "text_generation") => DeliveryPreferenceChange::WriterModel(
                Some(bruhs_models::delivery::WriterModel::TextGeneration),
            ),
            ("settings-writer-model", model)
                if self.generator_model(model)
                    || self.delivery_preferences.catalog.model(model).is_some() =>
            {
                DeliveryPreferenceChange::WriterModel(Some(
                    bruhs_models::delivery::WriterModel::Separate {
                        model: model.to_owned(),
                        options: GenerationTarget::Writer
                            .selection(prefs)
                            .filter(|(old, _)| *old == model)
                            .map(|(_, options)| options.clone())
                            .unwrap_or_default(),
                    },
                ))
            }
            ("settings-delivery-merge", "merge") => {
                DeliveryPreferenceChange::MergeMethod(Some(MergeMethod::Merge))
            }
            ("settings-delivery-merge", "squash") => {
                DeliveryPreferenceChange::MergeMethod(Some(MergeMethod::Squash))
            }
            ("settings-delivery-merge", "rebase") => {
                DeliveryPreferenceChange::MergeMethod(Some(MergeMethod::Rebase))
            }
            ("settings-delivery-merge", "inherit") => DeliveryPreferenceChange::MergeMethod(None),
            ("settings-delivery-templates", "inherit") => {
                DeliveryPreferenceChange::FollowTemplates(None)
            }
            ("settings-delivery-templates", "on") => {
                DeliveryPreferenceChange::FollowTemplates(Some(true))
            }
            ("settings-delivery-templates", "off") => {
                DeliveryPreferenceChange::FollowTemplates(Some(false))
            }
            ("settings-delivery-credits", "on") => {
                DeliveryPreferenceChange::RemoveAgentCredits(Some(true))
            }
            ("settings-delivery-credits", "off") => {
                DeliveryPreferenceChange::RemoveAgentCredits(Some(false))
            }
            ("settings-delivery-credits", "inherit") => {
                DeliveryPreferenceChange::RemoveAgentCredits(None)
            }
            _ => return,
        };
        let scope = self.settings_project_scope.clone();
        self.queue_delivery_change(&scope, change, cx);
    }

    pub(super) fn loaded_delivery_preferences(&self) -> Option<&DeliveryPreferences> {
        self.delivery_preferences.value.as_ref()
    }

    pub(super) fn queue_delivery_change(
        &mut self,
        scope: &str,
        change: DeliveryPreferenceChange,
        cx: &mut Context<'_, Self>,
    ) {
        self.delivery_preferences
            .pending
            .push_back(SetDeliveryPreference {
                path: (!scope.is_empty()).then(|| scope.to_owned()),
                change,
            });
        self.flush_delivery_changes(cx);
    }

    fn flush_delivery_changes(&mut self, cx: &mut Context<'_, Self>) {
        if self.delivery_preferences.saving {
            return;
        }
        let Some(edit) = self.delivery_preferences.pending.pop_front() else {
            return;
        };
        let scope = edit.path.clone().unwrap_or_default();
        self.delivery_preferences.saving = true;
        self.delivery_preferences.error = None;
        self.delivery_preferences.revision = self.delivery_preferences.revision.wrapping_add(1);
        let revision = self.delivery_preferences.revision;
        let request = crate::app_state::spawn_tokio(cx, async move {
            crate::server_client::ServerClient::workbench_client()
                .set_delivery_preference(edit)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                this.delivery_preferences.saving = false;
                if this.delivery_settings_scope() == scope
                    && this.delivery_preferences.revision == revision
                {
                    match result {
                        Ok(value) => this.delivery_preferences.value = Some(value),
                        Err(error) => {
                            this.delivery_preferences.error =
                                Some(friendly_delivery_error(&error))
                        }
                    }
                } else if let Err(error) = result {
                    this.delivery_preferences.error = Some(friendly_delivery_error(&error));
                }
                if !this.delivery_preferences.pending.is_empty() {
                    this.flush_delivery_changes(cx);
                } else if this.delivery_settings_scope() != scope
                    || this.delivery_preferences.revision != revision
                {
                    this.load_delivery_preferences(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn delivery_preference_rows(
        &self,
        cx: &Context<'_, Self>,
    ) -> (Vec<AnyElement>, AnyElement) {
        let state = &self.delivery_preferences;
        let scoped = !self.settings_project_scope.is_empty();
        let method = state.value.as_ref().and_then(|p| {
            if scoped {
                p.overrides.merge_method
            } else {
                Some(p.merge_method)
            }
        });
        const METHODS: &[(&str, &str)] = &[
            ("inherit", "Environment default"),
            ("squash", "Squash"),
            ("merge", "Merge commit"),
            ("rebase", "Rebase"),
        ];
        const SWITCH: &[(&str, &str)] = &[
            ("inherit", "Environment default"),
            ("on", "On"),
            ("off", "Off"),
        ];
        let waiting = || {
            div()
                .text_color(theme::rambutan_400())
                .child(if state.saving {
                    "Saving…"
                } else if state.error.is_some() {
                    "Unavailable"
                } else {
                    "Loading…"
                })
                .into_any_element()
        };
        let merge = if state.value.is_none() || state.saving {
            waiting()
        } else {
            settings_ui::control_column(self.choice_control(
                "settings-delivery-merge",
                "Default merge method",
                match method {
                    None => "inherit",
                    Some(MergeMethod::Merge) => "merge",
                    Some(MergeMethod::Squash) => "squash",
                    Some(MergeMethod::Rebase) => "rebase",
                },
                if scoped { METHODS } else { &METHODS[1..] },
                cx,
            ))
        };
        let toggle = |id: &'static str, label: &'static str, value: Option<bool>, default: bool| {
            if state.value.is_none() || state.saving {
                return waiting();
            }
            if scoped {
                return settings_ui::control_column(self.choice_control(
                    id,
                    label,
                    match value {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "inherit",
                    },
                    SWITCH,
                    cx,
                ));
            }
            let enabled = value.unwrap_or(default);
            settings_ui::switch(id, enabled, false, label, cx, move |this, _, cx| {
                this.save_delivery_preference(id, if enabled { "off" } else { "on" }, cx)
            })
        };
        let credits = state.value.as_ref().and_then(|p| {
            if scoped {
                p.overrides.remove_agent_credits
            } else {
                Some(p.remove_agent_credits)
            }
        });
        let mut rows = vec![
            self.settings_row("desktop-settings-row-merge-credits","Remove agent credits when merging","Strip agent footers from merge and squash messages.",toggle("settings-delivery-credits","Remove agent credits when merging",credits,false)),
            self.settings_row(
                "desktop-settings-row-merge-method",
                "Default merge method",
                "",
                merge,
            ),
        ];
        if let Some(error) = &state.error {
            rows.push(
                self.settings_row(
                    "desktop-settings-delivery-status",
                    "Delivery host",
                    error.clone(),
                    on_press_view(
                        web_button(
                            "retry-delivery-prefs",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child("Retry"),
                        cx,
                        |this, _, cx| this.load_delivery_preferences(cx),
                    )
                    .into_any_element(),
                ),
            );
        }
        let templates = state.value.as_ref().map_or_else(waiting, |prefs| {
            let enabled = prefs.follow_templates;
            settings_ui::switch(
                "settings-delivery-templates",
                enabled,
                state.saving,
                "Follow pull request templates",
                cx,
                move |this, _, cx| {
                    this.save_delivery_preference(
                        "settings-delivery-templates",
                        if enabled { "off" } else { "on" },
                        cx,
                    )
                },
            )
        });
        let reset_templates = state.value.as_ref().is_some_and(|p| {
            if scoped {
                p.overrides.follow_templates.is_some()
            } else {
                !p.follow_templates
            }
        });
        let template_row = self.settings_row_with_reset(
            "desktop-settings-row-pr-templates",
            "Follow pull request templates",
            "Use the repository’s PR template.",
            None,
            false,
            Some(templates),
            reset_templates.then(|| {
                self.writer_reset(
                    "reset-pr-template",
                    "Reset pull request templates",
                    "settings-delivery-templates",
                    cx,
                )
            }),
        );
        (rows, template_row)
    }
}
