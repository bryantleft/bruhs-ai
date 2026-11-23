//! Route newly created threads when the selected provider reports an exhausted quota window.
use super::*;
use crate::app_state::spawn_tokio;
use bruhs_models::agent::{model_info, provider_for_model_id, MODEL_CATALOG};

impl Workspace {
    pub(super) fn model_for_new_thread<'a>(&'a self, requested: &'a str) -> &'a str {
        let Some(fallback) = self.fallback_model.as_deref() else {
            return requested;
        };
        let exhausted = |model: &str| {
            self.provider_resets
                .get(provider_for_model_id(model).as_str())
                .is_some_and(|at| *at > EpochMs::now().as_millis())
        };
        if exhausted(requested) && !exhausted(fallback) {
            fallback
        } else {
            requested
        }
    }

    pub(super) fn fallback_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let mut options = vec![DropdownOption {
            value: "off",
            label: "Off".into(),
            description: None,
        }];
        options.extend(
            MODEL_CATALOG
                .iter()
                .filter(|m| !m.provider.is_external())
                .map(|m| DropdownOption {
                    value: m.id,
                    label: m.label.into(),
                    description: None,
                }),
        );
        options.extend(
            self.thread_catalog
                .models
                .iter()
                .filter(|m| model_info(&m.id).is_none())
                .map(|m| DropdownOption {
                    value: bruhs_ui::dropdown::saved_option_value(&m.id),
                    label: m.label.clone(),
                    description: None,
                }),
        );
        for instance in self.acp_models.iter().filter(|i| {
            self.provider_availability
                .as_ref()
                .is_ok_and(|prefs| i.enabled(prefs))
        }) {
            options.extend(instance.choices.iter().map(|m| DropdownOption {
                value: bruhs_ui::dropdown::saved_option_value(&m.id),
                label: m.label.clone(),
                description: None,
            }));
        }
        let model = self.fallback_model.as_deref().unwrap_or("off");
        let (toggle, select) = self.settings_dropdown_handlers("settings-fallback-model", cx);
        let picker = dropdown(
            "settings-fallback-model",
            "Fallback model",
            model,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            self.settings_listbox("settings-fallback-model"),
            toggle,
            select,
        );
        self.settings_row(
            "settings-row-fallback-model",
            "Fallback model",
            "Used when the selected provider hits a usage limit.",
            div()
                .w(px(self.model_control_width(Some(model))))
                .max_w_full()
                .min_w_0()
                .child(self.with_model_options(model, true, picker, cx))
                .into_any_element(),
        )
    }

    pub(super) fn choose_fallback_model(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        let available_instance = self.acp_models.iter().any(|i| {
            self.provider_availability
                .as_ref()
                .is_ok_and(|p| i.enabled(p))
                && i.choices.iter().any(|m| m.id == value)
        });
        if value != "off"
            && !model_info(value).is_some_and(|m| !m.provider.is_external())
            && self.thread_catalog.model(value).is_none()
            && !available_instance
        {
            return;
        }
        if self.fallback_model.as_deref() != Some(value) {
            self.fallback_options = Default::default();
        }
        self.fallback_model = (value != "off").then(|| value.to_owned());
        self.monitor_provider_capacity(cx);
        self.persist_and_notify(cx);
    }
    pub(super) fn monitor_provider_capacity(&mut self, cx: &mut Context<'_, Self>) {
        self.capacity_task = None;
        if self.fallback_model.is_none() {
            self.provider_resets.clear();
            return;
        }
        if cfg!(test) {
            return;
        }
        self.capacity_task = Some(cx.spawn(async move |this, cx| loop {
            let Ok(probe) = this.update(cx, |_, cx| {
                spawn_tokio(cx, async {
                    let (claude, codex) = tokio::join!(
                        bruhs_core::coordination::provider_quota_reset("claude"),
                        bruhs_core::coordination::provider_quota_reset("codex")
                    );
                    [("claude", claude), ("codex", codex)]
                })
            }) else {
                return;
            };
            let readings = probe.await;
            if this
                .update(cx, |this, cx| {
                    let previous = this.provider_resets.clone();
                    for (provider, reading) in readings {
                        match reading {
                            Ok(Some(at)) => {
                                this.provider_resets
                                    .insert(provider.to_owned(), at.as_millis());
                            }
                            Ok(None) => {
                                this.provider_resets.remove(provider);
                            }
                            Err(_) => {}
                        }
                    }
                    this.provider_resets
                        .retain(|_, at| *at > EpochMs::now().as_millis());
                    if previous != this.provider_resets {
                        this.persist_and_notify(cx);
                    }
                })
                .is_err()
            {
                return;
            }
            cx.background_executor()
                .timer(std::time::Duration::from_secs(120))
                .await;
        }));
    }
}
#[cfg(test)]
mod tests {
    #[gpui::test]
    fn project_and_general_threads_use_fallback_only_until_a_provider_recovers(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |w, window, cx| {
            w.current_model = "gpt-5.4".into();
            w.choose_fallback_model("claude-sonnet-4-6", cx);
            w.provider_resets
                .insert("codex".into(), super::EpochMs::now().as_millis() + 60_000);
            assert_eq!(w.new_thread_defaults("/project").0, "claude-sonnet-4-6");
            w.new_general_thread(window, cx);
            assert_eq!(w.active_thread().unwrap().model_id, "claude-sonnet-4-6");
            w.provider_resets
                .insert("claude".into(), super::EpochMs::now().as_millis() + 60_000);
            assert_eq!(w.new_thread_defaults("/project").0, "gpt-5.4");
            w.provider_resets.insert("codex".into(), 1);
            assert_eq!(w.new_thread_defaults("/project").0, "gpt-5.4");
            assert_eq!(w.active_thread().unwrap().model_id, "claude-sonnet-4-6");
        });
    }
}
