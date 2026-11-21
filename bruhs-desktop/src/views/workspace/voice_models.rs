//! Browse the verified speech model files on the capture host.
use super::*;
use bruhs_models::{
    dictation_runtime::{ModelAction, ModelEntry},
    WhisperModel,
};

pub(super) struct Browser {
    open: bool,
    filter: Entity<InputState>,
    import_path: Entity<InputState>,
    importing: Option<WhisperModel>,
    language: &'static str,
    sort: &'static str,
    installed_only: bool,
    entries: Vec<ModelEntry>,
    error: Option<String>,
    action_pending: bool,
    revision: u64,
}
impl Browser {
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<'_, Workspace>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Search models"));
        let import_path = cx
            .new(|cx| InputState::new(window, cx).placeholder("Model file or folder on this host"));
        subscriptions.push(cx.subscribe(
            &import_path,
            |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => this.import_voice_model(cx),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify()
            }
        }));
        Self {
            open: false,
            filter,
            import_path,
            importing: None,
            language: "all",
            sort: "recommended",
            installed_only: false,
            entries: Vec::new(),
            error: None,
            action_pending: false,
            revision: 0,
        }
    }
    pub(super) fn close(&mut self) {
        self.open = false;
        self.importing = None;
    }
}
impl Workspace {
    fn import_voice_model(&mut self, cx: &mut Context<'_, Self>) {
        let Some(model) = self.voice_models.importing else {
            return;
        };
        let path = self
            .voice_models
            .import_path
            .read(cx)
            .value()
            .trim()
            .to_string();
        if !path.is_empty() {
            self.fetch_voice_models(Some(ModelAction::Import { model, path }), cx);
        }
    }
    fn fetch_voice_models(&mut self, action: Option<ModelAction>, cx: &mut Context<'_, Self>) {
        if self.voice_models.action_pending {
            return;
        }
        self.voice_models.revision = self.voice_models.revision.wrapping_add(1);
        let revision = self.voice_models.revision;
        self.voice_models.action_pending = action.is_some();
        self.voice_models.error = None;
        let is_import = matches!(action, Some(ModelAction::Import { .. }));
        let request = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::capture_client::dictation_models(action).await
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let poll = this
                .update(cx, |this, cx| {
                    let catalog = &mut this.voice_models;
                    if catalog.revision != revision {
                        return false;
                    }
                    catalog.action_pending = false;
                    match result {
                        Ok(entries) => {
                            catalog.entries = entries;
                            if is_import {
                                catalog.importing = None;
                            }
                        }
                        Err(error) => catalog.error = Some(error.to_string()),
                    }
                    cx.notify();
                    catalog.error.is_none()
                        && catalog.entries.iter().any(|e| e.downloaded_bytes.is_some())
                })
                .unwrap_or(false);
            if poll {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(450))
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.settings_visible()
                        && this.settings_section == SettingsSection::Voice
                        && this.voice_models.open
                        && this.voice_models.revision == revision
                    {
                        this.fetch_voice_models(None, cx)
                    }
                });
            }
        })
        .detach();
        cx.notify();
    }
    pub(super) fn select_model_filter(
        &mut self,
        id: &str,
        value: &'static str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        match id {
            "voice-model-sort" => self.voice_models.sort = value,
            "voice-model-language" => self.voice_models.language = value,
            _ => return false,
        }
        cx.notify();
        true
    }
    pub(super) fn voice_model_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let catalog = &self.voice_models;
        let browse = web_button(
            "voice-browse-models",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .aria_label("Browse transcription models")
        .child(if catalog.open {
            "Close browser"
        } else {
            "Browse models"
        });
        let mut rows = vec![self.settings_row(
            "voice-row-local-models",
            "Local models",
            "Download and manage the Whisper files used by this host.",
            on_press_view(browse, cx, |this, _, cx| {
                this.voice_models.open = !this.voice_models.open;
                if this.voice_models.open {
                    this.fetch_voice_models(None, cx);
                }
                cx.notify();
            })
            .into_any_element(),
        )];
        if !catalog.open {
            return rows;
        }
        let mut languages = vec![("all", "All languages")];
        languages.extend(super::voice_settings::LANGUAGES.iter().copied().skip(1));
        let toolbar = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w(px(130.))
                    .child(Input::new(&catalog.filter).aria_label("Search transcription models")),
            )
            .child(div().w(px(150.)).max_w_full().child(self.choice_control(
                "voice-model-language",
                "Browse models by language",
                catalog.language,
                &languages,
                cx,
            )))
            .child(div().w(px(140.)).max_w_full().child(self.choice_control(
                "voice-model-sort",
                "Sort models",
                catalog.sort,
                &[
                    ("recommended", "Recommended"),
                    ("size", "Smallest first"),
                    ("name", "Name"),
                ],
                cx,
            )));
        let mut body = div()
            .mx_3()
            .mb_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .child(settings_ui::switch(
                        "voice-model-installed",
                        catalog.installed_only,
                        false,
                        "Downloaded only",
                        cx,
                        |this, _, cx| {
                            this.voice_models.installed_only = !this.voice_models.installed_only;
                            cx.notify();
                        },
                    ))
                    .child("Downloaded only"),
            );
        if let Some(error) = &catalog.error {
            let retry = web_button(
                "voice-model-retry",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .child("Retry");
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(error.clone())
                    .child(on_press_view(retry, cx, |this, _, cx| {
                        this.fetch_voice_models(None, cx)
                    })),
            );
        }
        let query = catalog.filter.read(cx).value().trim().to_lowercase();
        let mut entries: Vec<_> = catalog
            .entries
            .iter()
            .filter(|entry| {
                let d = entry.model.descriptor();
                (!catalog.installed_only || entry.installed)
                    && (matches!(catalog.language, "all" | "en") || d.multilingual)
                    && (d.label.to_lowercase().contains(&query) || d.filename.contains(&query))
            })
            .collect();
        match catalog.sort {
            "name" => entries.sort_by_key(|e| e.model.descriptor().label),
            "size" => entries.sort_by_key(|e| e.model.descriptor().size_bytes),
            _ => entries.sort_by_key(|e| {
                (
                    e.model != WhisperModel::default(),
                    e.model.descriptor().size_bytes,
                )
            }),
        }
        let mut list = div()
            .id("voice-model-list")
            .aria_label("Transcription models")
            .rounded_md()
            .border_1()
            .border_color(theme::longan_700());
        if entries.is_empty() {
            list = list.child(div().p_2().text_xs().child(
                if catalog.entries.is_empty() && catalog.error.is_none() {
                    "Reading the capture host…"
                } else {
                    "No models match."
                },
            ));
        }
        for (index, entry) in entries.into_iter().enumerate() {
            let model = entry.model;
            let d = model.descriptor();
            let name = d.label.split(',').next().unwrap_or(d.label);
            let selected = self.dictation_prefs.model == model;
            let mut detail = format!(
                "{} MB · {}",
                d.size_mb,
                if entry.installed {
                    "Downloaded"
                } else {
                    "Available"
                }
            );
            if let Some(bytes) = entry.downloaded_bytes {
                detail = if bytes == d.size_bytes {
                    "Verifying download…".into()
                } else {
                    format!("{}% downloaded", bytes * 100 / d.size_bytes)
                };
            }
            if let Some(error) = &entry.error {
                detail = format!("Download failed. {error}");
            }
            let mut row = div()
                .flex()
                .items_center()
                .flex_wrap()
                .gap_2()
                .px_2()
                .py_2()
                .when(index > 0, |el| {
                    el.border_t_1().border_color(theme::longan_700())
                })
                .child(
                    div().flex_1().min_w(px(100.)).child(name).child(
                        div()
                            .text_xs()
                            .text_color(theme::rambutan_400())
                            .child(detail),
                    ),
                );
            let (label, action) = if entry.downloaded_bytes.is_some() {
                ("Cancel", Some(ModelAction::Cancel(model)))
            } else if !entry.installed {
                ("Download", Some(ModelAction::Download(model)))
            } else if selected {
                ("Selected", None)
            } else {
                ("Use model", None)
            };
            let button = web_button(
                SharedString::from(format!("voice-model-action-{}", model.key())),
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(catalog.action_pending || label == "Selected"),
            )
            .aria_label(format!("{label} {name}"))
            .child(label);
            row = row.child(on_press_view(button, cx, move |this, _, cx| {
                if let Some(action) = action.clone() {
                    this.fetch_voice_models(Some(action), cx)
                } else {
                    this.dictation_prefs.model = model;
                    this.stamp_run_prefs_on_threads(cx);
                    this.persist_and_notify(cx);
                }
            }));
            if entry.installed && entry.downloaded_bytes.is_none() {
                let remove = web_button(
                    SharedString::from(format!("voice-model-remove-{}", model.key())),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(catalog.action_pending),
                )
                .aria_label(format!("Delete {name}"))
                .child("Delete");
                row = row.child(on_press_view(remove, cx, move |this, _, cx| {
                    this.fetch_voice_models(Some(ModelAction::Remove(model)), cx)
                }));
            }
            if !entry.installed && entry.downloaded_bytes.is_none() {
                let existing = web_button(
                    SharedString::from(format!("voice-model-import-{}", model.key())),
                    WebButtonVariant::Ghost,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(catalog.action_pending),
                )
                .aria_label(format!("Import existing {name} file"))
                .child("Use file");
                row = row.child(on_press_view(existing, cx, move |this, window, cx| {
                    this.voice_models.importing = if this.voice_models.importing == Some(model) {
                        None
                    } else {
                        Some(model)
                    };
                    this.voice_models
                        .import_path
                        .update(cx, |field, cx| field.set_value("", window, cx));
                    cx.notify();
                }));
            }
            list = list.child(row);
            if catalog.importing == Some(model) && !entry.installed {
                let import = web_button(
                    "voice-import-confirm",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(
                        catalog.action_pending
                            || catalog.import_path.read(cx).value().trim().is_empty(),
                    ),
                )
                .child(if catalog.action_pending {
                    "Verifying…"
                } else {
                    "Import"
                });
                list=list.child(div().px_2().pb_2().flex().flex_col().gap_1().child(div().flex().gap_2().items_center().child(div().flex_1().min_w_0().child(Input::new(&catalog.import_path).aria_label("Path to a model file or folder"))).child(on_press_view(import,cx,|this,_,cx|this.import_voice_model(cx))))
                    .child(div().text_xs().text_color(theme::rambutan_400()).child("A verified copy is saved on this host. Your original file stays in place.")));
            }
        }
        rows.push(body.child(list).into_any_element());
        rows
    }
}
