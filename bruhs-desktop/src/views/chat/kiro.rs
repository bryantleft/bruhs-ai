use super::*;
use bruhs_core::agent_extensions::kiro::{effort_levels, Agents, Catalog, Selection};
use bruhs_ui::dropdown::{saved_option_value, Dropdown, DropdownGroup, DropdownOption, Listbox};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct KiroChoices {
    agents: BTreeMap<String, String>,
    efforts: BTreeMap<String, String>,
}
pub(crate) fn effort_label(level: &'static str) -> &'static str {
    match level {
        "none" => "None",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra High",
        "max" => "Max",
        _ => level,
    }
}
impl KiroChoices {
    pub(crate) fn selection(&self, model: &str) -> Selection {
        let Some((id, _)) = model.strip_prefix("acp/").and_then(|m| m.split_once('/')) else {
            return Selection::default();
        };
        Selection {
            agent: self.agents.get(id).cloned(),
            effort: self.efforts.get(model).cloned(),
        }
    }
    pub(crate) fn set(&mut self, model: &str, agent: bool, value: Option<String>) {
        let Some((id, _)) = model.strip_prefix("acp/").and_then(|m| m.split_once('/')) else {
            return;
        };
        let (store, key) = if agent {
            (&mut self.agents, id)
        } else {
            (&mut self.efforts, model)
        };
        if let Some(v) = value {
            store.insert(key.into(), v);
        } else {
            store.remove(key);
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.agents.is_empty() && self.efforts.is_empty()
    }
}

impl ChatView {
    fn kiro_model(&self) -> Option<(&str, &str)> {
        self.model_id.strip_prefix("acp/")?.split_once('/')
    }
    pub(super) fn refresh_kiro_choices(&mut self) {
        self.kiro_agents = self.kiro_model().and_then(|(id, _)| {
            let instance = bruhs_core::agent_extensions::load_config()
                .ok()?
                .acp
                .into_iter()
                .find(|a| {
                    a.id == id && a.driver == Some(bruhs_models::agent_extensions::AcpDriver::Kiro)
                })?;
            Some(Catalog::load(&instance).agents)
        });
        self.kiro_agent_menu = false;
    }
    pub(super) fn kiro_efforts(&self) -> &'static [&'static str] {
        self.kiro_model()
            .filter(|_| self.kiro_agents.is_some())
            .map_or(&[], |(_, model)| effort_levels(model).0)
    }
    pub(super) fn kiro_display_effort(&self) -> Option<&str> {
        let (_, model) = self.kiro_model()?;
        let (levels, default) = effort_levels(model);
        self.kiro_choices
            .efforts
            .get(&self.model_id)
            .map(String::as_str)
            .filter(|v| levels.contains(v))
            .or(default)
    }
    pub(super) fn kiro_selection(&self) -> Selection {
        if self.kiro_agents.is_none() {
            return Selection::default();
        }
        let mut selection = self.kiro_choices.selection(&self.model_id);
        selection.effort = selection
            .effort
            .filter(|value| self.kiro_efforts().contains(&value.as_str()));
        selection
    }
    pub(super) fn choose_kiro_effort(&mut self, value: Option<String>, cx: &mut Context<'_, Self>) {
        self.kiro_choices.set(&self.model_id, false, value);
        cx.emit(ChatViewEvent::ThreadOptionsChanged);
    }
    pub(super) fn kiro_agent_visible(&self) -> bool {
        self.kiro_agents
            .as_ref()
            .is_some_and(|a| a.choices.len() > 1)
    }
    pub(super) fn render_kiro_agents(&self, cx: &Context<'_, Self>) -> AnyElement {
        let agents: &Agents = self.kiro_agents.as_ref().expect("visible Kiro selector");
        let selected = self.kiro_selection().agent;
        let index_value = |i| saved_option_value(&format!("kiro-agent-{i}"));
        let mut options = Vec::new();
        let selected = selected
            .as_ref()
            .filter(|s| agents.choices.iter().any(|a| &a.value == *s))
            .or(agents.current.as_ref());
        let mut value = "default";
        if agents.current.is_none() {
            options.push(DropdownOption {
                value,
                label: "Provider default".into(),
                description: None,
            });
        }
        for (i, agent) in agents.choices.iter().enumerate() {
            let key = index_value(i);
            if selected == Some(&agent.value) {
                value = key;
            }
            options.push(DropdownOption {
                value: key,
                label: agent.name.clone(),
                description: agent.description.clone(),
            });
        }
        let entity = cx.weak_entity();
        let toggle = std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |chat, cx| {
                chat.kiro_agent_menu = !chat.kiro_agent_menu;
                cx.notify()
            });
        });
        let entity = cx.weak_entity();
        let select = std::rc::Rc::new(move |value: &'static str, _: &mut Window, cx: &mut App| {
            let _ = entity.update(cx, |chat, cx| {
                let Some((id, _)) = chat.kiro_model() else {
                    return;
                };
                let id = id.to_owned();
                if let Some(agent) = value
                    .strip_prefix("kiro-agent-")
                    .and_then(|s| s.parse::<usize>().ok())
                    .and_then(|i| chat.kiro_agents.as_ref()?.choices.get(i))
                {
                    chat.kiro_choices.agents.insert(id, agent.value.clone());
                } else if value == "default" {
                    chat.kiro_choices.agents.remove(&id);
                }
                chat.kiro_agent_menu = false;
                cx.emit(ChatViewEvent::ThreadOptionsChanged);
                cx.notify();
            });
        });
        Dropdown::new(
            "kiro-thread-agent",
            "Agent",
            value,
            vec![DropdownGroup {
                label: None,
                mark: None,
                options,
            }],
            if self.kiro_agent_menu {
                Listbox::Popover
            } else {
                Listbox::Closed
            },
            toggle,
            select,
        )
        .into_any_element()
    }
}
