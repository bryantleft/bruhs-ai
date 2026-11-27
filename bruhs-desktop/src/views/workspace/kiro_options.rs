use super::*;
use crate::views::chat::kiro::KiroChoices;
use bruhs_core::agent_extensions::kiro::{effort_levels, Agents, Selection};

impl Workspace {
    fn kiro_model_options<'a>(&self, model: &'a str) -> Option<(&Agents, &'a str)> {
        let (id, model) = model.strip_prefix("acp/")?.split_once('/')?;
        Some((
            self.acp_models
                .iter()
                .find(|i| i.id == id)?
                .kiro_agents
                .as_ref()?,
            model,
        ))
    }
    fn kiro_defaults_for(&self, path: &str, model: &str, fallback: bool) -> Selection {
        if fallback {
            return self.fallback_kiro.selection(model);
        }
        let global = self.default_kiro.selection(model);
        let local = self
            .project_defaults
            .get(path)
            .map(|p| p.kiro.selection(model))
            .unwrap_or_default();
        Selection {
            agent: local.agent.or(global.agent),
            effort: local.effort.or(global.effort),
        }
    }
    pub(super) fn seed_model_options(&self, thread: &DesktopThread, cx: &mut Context<'_, Self>) {
        let path = if thread.project_name.is_empty() {
            ""
        } else {
            thread.chat.read(cx).settings_project_path()
        };
        let requested = self
            .project_defaults
            .get(path)
            .and_then(|p| p.model.as_ref())
            .unwrap_or(&self.current_model);
        let model = thread.chat.read(cx).model_id.clone();
        let used_fallback = model != *requested && self.model_for_new_thread(requested) == model;
        let options = self.kiro_defaults_for(path, &model, used_fallback);
        let native = self.thread_options_for(path, &model);
        thread.chat.update(cx, |chat, cx| {
            chat.model_options = native;
            chat.replace_model_catalog(self.thread_catalog.clone(), cx);
        });
        if !options.is_empty() {
            thread.chat.update(cx, |chat, cx| {
                chat.kiro_choices.set(&model, true, options.agent);
                chat.kiro_choices.set(&model, false, options.effort);
                cx.notify()
            });
        }
    }
    /// Keep the option pickers next to the model they configure.
    pub(super) fn with_model_options(
        &self,
        model: &str,
        fallback: bool,
        picker: AnyElement,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let Some((agents, slug)) = self.kiro_model_options(model) else {
            return self.native_model_control(model, fallback, picker, cx);
        };
        let (levels, default_level) = effort_levels(slug);
        if levels.is_empty() && agents.choices.len() < 2 {
            return picker;
        }
        let scope = &self.settings_project_scope;
        let local = if fallback {
            Some(&self.fallback_kiro)
        } else if scope.is_empty() {
            Some(&self.default_kiro)
        } else {
            self.project_defaults.get(scope).map(|p| &p.kiro)
        };
        let selected = local.map(|p| p.selection(model)).unwrap_or_default();
        let effective = self.kiro_defaults_for(scope, model, fallback);
        let scoped = !fallback && !scope.is_empty();
        let mut groups = Vec::new();
        let mut checked = Vec::new();
        let mut defaults = Vec::new();
        let mut summary = Vec::new();
        if !levels.is_empty() {
            let effective = effective
                .effort
                .as_deref()
                .filter(|s| levels.contains(s))
                .or(default_level);
            let mut options = Vec::new();
            if scoped {
                options.push(inherit("inherit-effort"));
            }
            for &level in levels {
                let label = crate::views::chat::kiro::effort_label(level);
                if Some(level) == effective {
                    checked.push(SharedString::from(if scoped && selected.effort.is_none() {
                        "inherit-effort"
                    } else {
                        level
                    }));
                    summary.push(label.to_owned());
                }
                if Some(level) == default_level {
                    defaults.push(SharedString::from(level));
                }
                options.push(DropdownOption {
                    value: level.into(),
                    label: label.into(),
                    description: None,
                });
            }
            groups.push(DropdownGroup {
                label: Some("Reasoning".into()),
                mark: None,
                options,
            });
        }
        if agents.choices.len() > 1 {
            let effective = effective
                .agent
                .as_ref()
                .filter(|v| agents.choices.iter().any(|a| a.value == **v))
                .or(agents.current.as_ref());
            let mut options = Vec::new();
            if scoped {
                options.push(inherit("inherit-agent"));
            }
            if effective.is_none() {
                checked.push(SharedString::from(if scoped {
                    "inherit-agent"
                } else {
                    "agent-default"
                }));
                summary.push("Provider default".into());
            }
            if agents.current.is_none() && !scoped {
                options.push(DropdownOption {
                    value: "agent-default".into(),
                    label: "Provider default".into(),
                    description: None,
                });
            }
            for a in &agents.choices {
                let key = SharedString::from(format!("agent={}", a.value));
                if Some(&a.value) == effective {
                    checked.push(if scoped && selected.agent.is_none() {
                        SharedString::from("inherit-agent")
                    } else {
                        key.clone()
                    });
                    summary.push(a.name.clone());
                }
                if agents.current.as_ref() == Some(&a.value) {
                    defaults.push(key.clone());
                }
                options.push(DropdownOption {
                    value: key,
                    label: a.name.clone(),
                    description: a.description.clone(),
                });
            }
            groups.push(DropdownGroup {
                label: Some("Agent".into()),
                mark: None,
                options,
            });
        }
        let Some((first, others)) = checked.split_first() else {
            return picker;
        };
        let id = if fallback {
            "settings-fallback-kiro-options"
        } else {
            "settings-kiro-options"
        };
        let (toggle, select) = self.settings_dropdown_handlers(id, cx);
        let control = bruhs_ui::dropdown::Dropdown::new(
            id,
            "Model options",
            first.as_ref(),
            groups,
            self.settings_listbox(id),
            toggle,
            select,
        )
        .group_selection(summary.join(", "), others.to_vec(), defaults);
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_1()
            .child(picker)
            .child(control)
            .into_any_element()
    }
    pub(super) fn choose_kiro_default(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let fallback = id.starts_with("settings-fallback-");
        let agent =
            value == "inherit-agent" || value == "agent-default" || value.starts_with("agent=");
        let scope = self.settings_project_scope.clone();
        let model = if fallback {
            let Some(m) = self.fallback_model.as_ref() else {
                return;
            };
            m
        } else {
            self.project_defaults
                .get(&scope)
                .and_then(|p| p.model.as_ref())
                .unwrap_or(&self.current_model)
        }
        .clone();
        let Some((agents, slug)) = self.kiro_model_options(&model) else {
            return;
        };
        let value = if matches!(value, "inherit-agent" | "inherit-effort" | "agent-default") {
            None
        } else if agent {
            let Some(choice) = value
                .strip_prefix("agent=")
                .and_then(|id| agents.choices.iter().find(|a| a.value == id))
            else {
                return;
            };
            Some(choice.value.clone())
        } else if effort_levels(slug).0.contains(&value) {
            Some(value.into())
        } else {
            return;
        };
        let store: &mut KiroChoices = if fallback {
            &mut self.fallback_kiro
        } else if scope.is_empty() {
            &mut self.default_kiro
        } else {
            &mut self.project_defaults.entry(scope.clone()).or_default().kiro
        };
        store.set(&model, agent, value);
        if self
            .project_defaults
            .get(&scope)
            .is_some_and(|p| p == &project_defaults::ProjectDefaults::default())
        {
            self.project_defaults.remove(&scope);
        }
        self.persist_and_notify(cx);
    }
}

fn inherit(value: &'static str) -> DropdownOption {
    DropdownOption {
        value: value.into(),
        label: "Use environment default".into(),
        description: None,
    }
}
