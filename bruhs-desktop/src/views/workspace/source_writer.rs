//! The source-control writer can follow the text generator or use a separate model.
use super::*;
use bruhs_models::{
    delivery::generation::GenerationOptions,
    delivery::{DeliveryPreferenceChange, GuideModel, WriterModel, WritingStyle},
};

pub(super) struct InstructionsEditor {
    scope: String,
    saved: String,
    submitted: Option<String>,
    field: Entity<TextareaState>,
    _subscription: Subscription,
}

#[derive(Clone, Copy)]
enum EditorKind {
    Writing,
    ReviewGuide,
}

impl EditorKind {
    fn slot(
        self,
        prefs: &mut super::delivery_preferences::HostPreferences,
    ) -> &mut Option<InstructionsEditor> {
        match self {
            Self::Writing => &mut prefs.editor,
            Self::ReviewGuide => &mut prefs.guide_editor,
        }
    }
    fn placeholder(self) -> &'static str {
        match self {
            Self::Writing => {
                "Keep commit subjects under 60 characters. Keep pull request descriptions short and plain."
            }
            Self::ReviewGuide => {
                "Show service interfaces before implementations. Keep summaries to two sentences."
            }
        }
    }
    fn change(self, text: String) -> DeliveryPreferenceChange {
        match self {
            Self::Writing => DeliveryPreferenceChange::WriterInstructions(Some(text)),
            Self::ReviewGuide => DeliveryPreferenceChange::ReviewGuideInstructions(Some(text)),
        }
    }
}
impl Workspace {
    pub(super) fn sync_writer_editor(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        for kind in [EditorKind::Writing, EditorKind::ReviewGuide] {
            let Some(prefs) = self.loaded_delivery_preferences() else {
                return;
            };
            let text = match kind {
                EditorKind::Writing if prefs.writing_style == WritingStyle::Custom => {
                    Some(prefs.writer_instructions.clone())
                }
                EditorKind::ReviewGuide if prefs.review_guides_enabled => {
                    Some(prefs.review_guide_instructions.clone())
                }
                _ => None,
            };
            let saving = self.delivery_preferences.saving;
            let slot = kind.slot(&mut self.delivery_preferences);
            let Some(text) = text else {
                *slot = None;
                continue;
            };
            let scope = self.settings_project_scope.clone();
            if let Some(editor) = slot.as_mut().filter(|e| e.scope == scope) {
                if editor.submitted.as_ref() == Some(&text) {
                    editor.saved.clone_from(&text);
                    editor.submitted = None;
                }
                if editor.saved == text || saving || editor.submitted.is_some() {
                    continue;
                }
            }
            let field = cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(4)
                    .default_value(text.clone())
                    .placeholder(kind.placeholder())
            });
            let subscription =
                cx.subscribe_in(&field, window, move |this, source, event, window, cx| {
                    if matches!(event, InputEvent::Blur)
                        && kind
                            .slot(&mut this.delivery_preferences)
                            .as_ref()
                            .is_some_and(|current| &current.field == source)
                    {
                        this.commit_instructions(kind, window, cx);
                    }
                });
            *slot = Some(InstructionsEditor {
                scope,
                saved: text,
                submitted: None,
                field,
                _subscription: subscription,
            });
        }
    }
    pub(super) fn commit_writer_instructions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.commit_instructions(EditorKind::Writing, window, cx);
        self.commit_instructions(EditorKind::ReviewGuide, window, cx);
    }
    fn commit_instructions(
        &mut self,
        kind: EditorKind,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let saving = self.delivery_preferences.saving;
        let Some(editor) = kind.slot(&mut self.delivery_preferences).as_mut() else {
            return;
        };
        let raw = editor.field.read(cx).value();
        let text = raw.trim().to_owned();
        if raw.as_ref() != text {
            editor
                .field
                .update(cx, |field, cx| field.set_value(text.clone(), window, cx));
        }
        if (editor.submitted.is_none() && text == editor.saved)
            || (saving && editor.submitted.as_ref() == Some(&text))
        {
            return;
        }
        let scope = editor.scope.clone();
        editor.submitted = Some(text.clone());
        self.queue_delivery_change(&scope, kind.change(text), cx);
    }

    pub(super) fn writer_reset(
        &self,
        id: &'static str,
        label: &'static str,
        setting: &'static str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let button = web_button(
            id,
            WebButtonVariant::Ghost,
            WebButtonSize::IconSmall,
            ButtonState::Enabled,
        )
        .aria_label(label)
        .debug_selector(move || id.into())
        .child(
            svg()
                .path(assets::ROTATE_CCW_ICON_PATH)
                .size(px(14.))
                .text_color(theme::rambutan_400()),
        );
        on_press_view(button, cx, move |this, window, cx| {
            this.commit_writer_instructions(window, cx);
            this.save_delivery_preference(setting, "inherit", cx)
        })
        .into_any_element()
    }

    pub(super) fn source_writer_card(
        &self,
        template_row: AnyElement,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let prefs = self.loaded_delivery_preferences();
        let mode = prefs.map(|p| p.writing_style).unwrap_or_default();
        let loading = || {
            div()
                .text_color(theme::rambutan_400())
                .child(if self.delivery_preferences.error.is_some() {
                    "Unavailable"
                } else {
                    "Loading…"
                })
                .into_any_element()
        };
        let style = if prefs.is_some() {
            settings_ui::control_column(self.choice_control(
                "settings-writing-style",
                "Commit and pull request writing",
                mode.key(),
                &[
                    ("repository", "Repository conventions"),
                    ("conventional", "Conventional Commits"),
                    ("custom", "Custom instructions"),
                ],
                cx,
            ))
        } else {
            loading()
        };
        let reset_style = prefs.is_some_and(|p| {
            if self.settings_project_scope.is_empty() {
                p.writing_style != WritingStyle::Repository || !p.writer_instructions.is_empty()
            } else {
                p.overrides.writing_style.is_some() || p.overrides.writer_instructions.is_some()
            }
        });
        let description = match mode {
            WritingStyle::Repository => "Match repository commit and PR style.",
            WritingStyle::Conventional => "Use Conventional Commits.",
            WritingStyle::Custom => "Use custom writing instructions.",
        };
        let row = self.settings_row_with_reset(
            "settings-row-writing-style",
            "Commit and pull request writing",
            description,
            None,
            false,
            Some(style),
            reset_style.then(|| {
                self.writer_reset(
                    "reset-writing-style",
                    "Reset commit and pull request writing",
                    "settings-writing-style",
                    cx,
                )
            }),
        );
        let mut writing = div().child(row);
        if mode == WritingStyle::Custom {
            if let Some(editor) = &self.delivery_preferences.editor {
                writing = writing.child(
                    div()
                        .px_4()
                        .pb_3()
                        .debug_selector(|| "writer-instructions-editor".into())
                        .child(
                            Textarea::new(&editor.field)
                                .aria_label("Custom commit and pull request instructions"),
                        ),
                );
            }
        }
        let guided = prefs.is_some_and(|p| p.review_guides_enabled);
        let guide_switch = if prefs.is_some() {
            settings_ui::switch(
                "settings-review-guides",
                guided,
                self.delivery_preferences.saving,
                "Review guides",
                cx,
                move |this, window, cx| {
                    this.commit_writer_instructions(window, cx);
                    this.save_delivery_preference(
                        "settings-review-guides",
                        if guided { "off" } else { "on" },
                        cx,
                    );
                },
            )
            .into_any_element()
        } else {
            loading()
        };
        let scoped = !self.settings_project_scope.is_empty();
        let reset_guides =
            scoped && prefs.is_some_and(|p| p.overrides.review_guides_enabled.is_some());
        let guide_row = self.settings_row_with_reset(
            "settings-row-review-guides",
            "Review guides (beta)",
            "Add an agent-generated Guide tab to pull requests.",
            None,
            false,
            Some(guide_switch),
            reset_guides.then(|| {
                self.writer_reset(
                    "reset-review-guides",
                    "Use the environment review guide setting",
                    "settings-review-guides",
                    cx,
                )
            }),
        );
        let mut rows = vec![writing.into_any_element(), guide_row];
        if let Some(p) = prefs.filter(|p| p.review_guides_enabled) {
            let model = match &p.review_guide_model {
                GuideModel::Separate { model, options } => Some((model.as_str(), options)),
                GuideModel::Writer => None,
            };
            let default = match &p.writer_model {
                WriterModel::Separate { model, .. }
                    if self.delivery_preferences.catalog.model(model).is_some() =>
                {
                    model.clone()
                }
                _ => self.active_settings_writer_default(false),
            };
            let control = self.source_model_control(
                "settings-guide-model",
                "settings-guide-model-switch",
                "Review guide model",
                model,
                default,
                "writer",
                cx,
            );
            let reset_model = scoped && p.overrides.review_guide_model.is_some();
            rows.push(self.settings_row_with_reset(
                "settings-row-guide-model",
                "Review guide model",
                "Separate coding agent for PR guides.",
                None,
                false,
                Some(control),
                reset_model.then(|| {
                    self.writer_reset(
                        "reset-guide-model",
                        "Use the environment review guide model",
                        "settings-guide-model",
                        cx,
                    )
                }),
            ));
            let reset = if scoped {
                p.overrides.review_guide_instructions.is_some()
            } else {
                !p.review_guide_instructions.is_empty()
            };
            let label = self.settings_row_with_reset(
                "settings-row-guide-instructions",
                "Review guide instructions",
                "What guides should emphasize or skip.",
                None,
                false,
                None,
                reset.then(|| {
                    self.writer_reset(
                        "reset-guide-instructions",
                        "Reset review guide instructions",
                        "settings-guide-instructions",
                        cx,
                    )
                }),
            );
            let mut row = div().child(label);
            if let Some(editor) = &self.delivery_preferences.guide_editor {
                row = row.child(
                    div()
                        .px_4()
                        .pb_3()
                        .debug_selector(|| "review-guide-instructions-editor".into())
                        .child(
                            Textarea::new(&editor.field).aria_label("Review guide instructions"),
                        ),
                );
            }
            rows.push(row.into_any_element());
        }
        rows.push(self.settings_advanced_button(
            "source-writer-advanced",
            "Advanced commit and pull request settings",
            cx,
        ));
        if self.show_settings_advanced("source-writer-advanced") {
            rows.push(template_row);
            let control = prefs.map_or_else(loading, |p| {
                let model = match &p.writer_model {
                    WriterModel::Separate { model, options }
                        if !self.generator_unavailable(model) =>
                    {
                        Some((model.as_str(), options))
                    }
                    _ => None,
                };
                self.source_model_control(
                    "settings-writer-model",
                    "settings-writer-model-switch",
                    "Source control writer model",
                    model,
                    self.active_settings_writer_default(true),
                    "text_generation",
                    cx,
                )
            });
            let reset_model = !self.settings_project_scope.is_empty()
                && prefs.is_some_and(|p| p.overrides.writer_model.is_some());
            rows.push(self.settings_row_with_reset(
                "settings-row-writer-model",
                "Source control writer model",
                "Separate model for commits, PRs, and branch names.",
                None,
                false,
                Some(control),
                reset_model.then(|| {
                    self.writer_reset(
                        "reset-writer-model",
                        "Use the environment writer model",
                        "settings-writer-model",
                        cx,
                    )
                }),
            ));
        }
        self.settings_card("Commits & pull requests", "", rows)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "renders a separate-model toggle and picker with explicit ids, labels, and fallback key"
    )]
    fn source_model_control(
        &self,
        id: &'static str,
        switch_id: &'static str,
        label: &'static str,
        model: Option<(&str, &GenerationOptions)>,
        default: String,
        fallback: &'static str,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let mut controls = div()
            .flex()
            .flex_wrap()
            .min_w_0()
            .max_w_full()
            .justify_end()
            .items_center()
            .gap_2();
        let enabled = model.is_some();
        if let Some((model, options)) = model {
            let (toggle, select) = self.settings_dropdown_handlers(id, cx);
            let picker = bruhs_ui::dropdown::Dropdown::new(
                id,
                label,
                model,
                self.writer_picker_groups(id == "settings-writer-model", cx),
                self.settings_listbox(id),
                toggle,
                select,
            )
            .compact();
            controls = controls.child(div().min_w_0().max_w(px(220.)).child(picker));
            let options_id = if id == "settings-guide-model" {
                "settings-guide-options"
            } else {
                "settings-writer-options"
            };
            if let Some(control) = self.generation_options_control(
                options_id,
                &self.delivery_preferences.catalog,
                model,
                options,
                cx,
            ) {
                controls = controls.child(div().min_w_0().max_w(px(224.)).child(control));
            }
        }
        let switch_label = if id == "settings-writer-model" {
            "Use a separate source control writer model"
        } else {
            "Use a separate review guide model"
        };
        controls
            .child(settings_ui::switch(
                switch_id,
                enabled,
                self.delivery_preferences.saving,
                switch_label,
                cx,
                move |this, _, cx| {
                    this.save_delivery_preference(
                        id,
                        if enabled { fallback } else { &default },
                        cx,
                    );
                },
            ))
            .into_any_element()
    }

    fn writer_picker_groups(
        &self,
        include_endpoints: bool,
        cx: &Context<'_, Self>,
    ) -> Vec<DropdownGroup> {
        use bruhs_ui::dropdown::saved_option_value;
        let mut groups = Vec::new();
        for provider in [AgentProvider::Claude, AgentProvider::Codex] {
            if !self
                .provider_availability
                .as_ref()
                .is_ok_and(|prefs| prefs.enabled(provider))
            {
                continue;
            }

            let options: Vec<_> = self
                .delivery_preferences
                .catalog
                .models
                .iter()
                .filter(|m| m.provider == provider)
                .map(|m| DropdownOption {
                    value: saved_option_value(&m.id),
                    label: m.label.clone(),
                    description: Some(provider.model_id(&m.id).to_owned()),
                })
                .collect();
            if !options.is_empty() {
                groups.push(DropdownGroup {
                    label: Some(provider_label(provider)),
                    mark: Some(provider_mark(provider, cx)),
                    options,
                });
            }
        }
        if include_endpoints {
            groups.extend(self.generator_groups());
        }
        groups
    }

    pub(super) fn generation_options_control(
        &self,
        id: &'static str,
        catalog: &bruhs_models::delivery::writer_catalog::WriterCatalog,
        model: &str,
        options: &GenerationOptions,
        cx: &Context<'_, Self>,
    ) -> Option<AnyElement> {
        use bruhs_ui::dropdown::{saved_option_value, Dropdown};
        let mut groups = Vec::new();
        let mut selected = Vec::new();
        let mut summary = Vec::new();
        let mut defaults = Vec::new();
        use bruhs_models::delivery::generation::OptionKind;
        let specs = &catalog.model(model)?.options;
        for spec in specs {
            let mut rows = Vec::new();
            for choice in &spec.choices {
                let (value, label) = (choice.value.as_str(), choice.label.as_str());
                let token = saved_option_value(&format!("{}={value}", spec.kind.key()));
                if options
                    .configured(spec.kind)
                    .filter(|v| spec.accepts(v))
                    .unwrap_or(&spec.default)
                    == value
                {
                    selected.push(token);
                    if spec.kind == OptionKind::Thinking {
                        summary.push(format!("Thinking {label}"));
                    } else if value == "on" {
                        summary.push(spec.label().to_owned());
                    } else if value != "off"
                        && (spec.kind != OptionKind::CyberAccessProgram || specs.len() == 1)
                    {
                        summary.push(label.to_owned());
                    }
                }
                if spec.default == value && !matches!(value, "on" | "off") {
                    defaults.push(token);
                }
                let description = match (spec.kind, value) {
                    (bruhs_models::delivery::generation::OptionKind::ServiceTier, "ultrafast") => {
                        Some("Even faster, more expensive".to_owned())
                    }
                    (bruhs_models::delivery::generation::OptionKind::Effort, _) => None,
                    _ => choice.description.clone(),
                };
                rows.push(DropdownOption {
                    value: token,
                    label: label.to_owned(),
                    description,
                });
            }
            groups.push(DropdownGroup {
                label: Some(spec.label()),
                mark: None,
                options: rows,
            });
        }
        let (primary, others) = selected.split_first()?;
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let label = if id == "settings-thread-model-options" {
            "Default model options"
        } else if id == "settings-fallback-model-options" {
            "Fallback model options"
        } else if id == "settings-guide-options" {
            "Review guide model options"
        } else {
            "Source control writer model options"
        };
        Some(
            Dropdown::new(
                id,
                label,
                primary,
                groups,
                self.settings_listbox(id),
                toggle,
                select,
            )
            .compact()
            .group_selection(summary.join(", "), others.to_vec(), defaults)
            .into_any_element(),
        )
    }

    fn active_settings_writer_default(&self, allow_generator: bool) -> String {
        let project = &self.settings_project_scope;
        let active = self
            .project_defaults
            .get(project)
            .and_then(|p| p.model.as_deref())
            .unwrap_or(&self.current_model);
        let model = self.effective_text_model(Some(project), active);
        if (allow_generator && self.generator_model(&model))
            || self.delivery_preferences.catalog.model(&model).is_some()
        {
            model
        } else {
            bruhs_models::logic::prompt_suggestion::default_suggestion_model(AgentProvider::Claude)
                .to_owned()
        }
    }
}
