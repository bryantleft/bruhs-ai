//! Options stored with a default or fallback model and copied to a new conversation.
use super::*;

impl Workspace {
    pub(super) fn refresh_thread_models(&mut self, cx: &mut Context<'_, Self>) {
        if cfg!(test) {
            return;
        }
        let scope = self.settings_project_scope.clone();
        let path = PathBuf::from(&scope);
        let job = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::agent::writer_catalog_for_project(&path).await
        });
        self.thread_catalog_load = Some(cx.spawn(async move |ws, cx| {
            let catalog = job.await;
            let _ = ws.update(cx, |ws, cx| {
                if ws.settings_project_scope == scope {
                    ws.thread_catalog = Arc::new(catalog);
                    cx.notify();
                }
            });
        }));
    }

    fn settings_primary_model(&self) -> Option<&str> {
        self.project_defaults
            .get(&self.settings_project_scope)
            .and_then(|p| p.model.as_deref())
            .or_else(|| self.explicit_model.then_some(self.current_model.as_str()))
            .or_else(|| {
                self.enabled_new_thread_model(
                    DEFAULT_AGENT_MODEL,
                    true,
                    self.provider_availability.as_ref().ok()?,
                    || self.acp_models.as_slice(),
                )
            })
    }

    fn primary_options(&self, path: &str) -> &GenerationOptions {
        self.project_defaults
            .get(path)
            .filter(|p| p.model.is_some())
            .map_or(&self.default_options, |p| &p.model_options)
    }

    pub(super) fn thread_options_for(&self, path: &str, model: &str) -> GenerationOptions {
        let requested = self
            .project_defaults
            .get(path)
            .and_then(|p| p.model.as_deref())
            .unwrap_or(&self.current_model);
        if requested == model {
            return self.primary_options(path).clone();
        }
        if self.fallback_model.as_deref() == Some(model)
            && self.model_for_new_thread(requested) == model
        {
            return self.fallback_options.clone();
        }
        GenerationOptions::default()
    }

    pub(super) fn model_control_width(&self, model: Option<&str>) -> f32 {
        if model
            .and_then(|m| self.thread_catalog.model(m))
            .is_some_and(|m| !m.options.is_empty())
        {
            340.
        } else {
            SETTINGS_ROW_CONTROL_WIDTH_PX
        }
    }

    pub(super) fn native_model_control(
        &self,
        model: &str,
        fallback: bool,
        picker: AnyElement,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        if !self.model_enabled(model) {
            return picker;
        }
        let (id, values) = if fallback {
            ("settings-fallback-model-options", &self.fallback_options)
        } else {
            (
                "settings-thread-model-options",
                self.primary_options(&self.settings_project_scope),
            )
        };
        let Some(control) =
            self.generation_options_control(id, &self.thread_catalog, model, values, cx)
        else {
            return picker;
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .justify_end()
            .gap_1()
            .child(div().flex_1().min_w(px(116.)).child(picker))
            .child(control)
            .into_any_element()
    }

    pub(super) fn set_native_model_option(
        &mut self,
        id: &str,
        token: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let fallback = id == "settings-fallback-model-options";
        let selected = if fallback {
            self.fallback_model.as_deref()
        } else {
            self.settings_primary_model()
        };
        let Some(model) = selected.map(str::to_owned) else {
            return;
        };
        if !self.model_enabled(&model) {
            return;
        }
        let Some(row) = self.thread_catalog.model(&model) else {
            return;
        };
        let Some((kind, value)) = token.split_once('=') else {
            return;
        };
        let Some(spec) = row
            .options
            .iter()
            .find(|spec| spec.kind.key() == kind && spec.accepts(value))
        else {
            return;
        };
        let saved = if fallback {
            &self.fallback_options
        } else {
            self.primary_options(&self.settings_project_scope)
        };
        // A new model selection captures the other current choices alongside the edited one.
        let mut options = GenerationOptions::default();
        for opt in &row.options {
            let old = saved
                .configured(opt.kind)
                .filter(|val| opt.accepts(val))
                .unwrap_or(&opt.default);
            options.set_choice(opt, old);
        }
        options.set_choice(spec, value);
        if fallback {
            self.fallback_options = options;
        } else if self.settings_project_scope.is_empty() {
            self.default_options = options;
            self.current_model = model;
            self.explicit_model = true;
        } else {
            let prefs = self
                .project_defaults
                .entry(self.settings_project_scope.clone())
                .or_default();
            prefs.model = Some(model);
            prefs.model_options = options;
        }
        self.persist_and_notify(cx);
    }
}
