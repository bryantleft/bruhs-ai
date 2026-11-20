//! Only a provider-reported future reset can schedule a limited desktop thread.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    snooze: bool,
    resume: bool,
}
impl Workspace {
    pub(super) fn should_wait_on_quota(&self, path: &str) -> bool {
        self.quota_preferences(path) != Preferences::default()
    }

    fn quota_preferences(&self, path: &str) -> Preferences {
        let p = self.project_defaults.get(path);
        Preferences {
            snooze: p
                .and_then(|p| p.snooze_limited)
                .unwrap_or(self.quota_prefs.snooze),
            resume: p
                .and_then(|p| p.resume_limited)
                .unwrap_or(self.quota_prefs.resume),
        }
    }
    pub(super) fn choose_quota_preference(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let on = match value {
            "inherit" => None,
            "on" => Some(true),
            "off" => Some(false),
            _ => return,
        };
        let snooze = id == "settings-quota-snooze";
        if self.settings_project_scope.is_empty() {
            let Some(on) = on else { return };
            if snooze {
                self.quota_prefs.snooze = on
            } else {
                self.quota_prefs.resume = on
            }
        } else {
            let key = self.settings_project_scope.clone();
            let p = self.project_defaults.entry(key.clone()).or_default();
            if snooze {
                p.snooze_limited = on
            } else {
                p.resume_limited = on
            }
            if *p == Default::default() {
                self.project_defaults.remove(&key);
            }
        }
        if !snooze {
            let cancel: Vec<_> = self
                .threads
                .iter()
                .filter(|t| {
                    !self
                        .quota_preferences(t.chat.read(cx).settings_project_path())
                        .resume
                })
                .map(|t| t.id.clone())
                .collect();
            for id in cancel {
                self.quota_timers.remove(&id);
            }
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn render_quota_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let key = &self.settings_project_scope;
        let p = self.project_defaults.get(key);
        let mut rows = Vec::new();
        for (id,selector,label,desc,environment,project) in [
            ("settings-quota-resume","desktop-settings-row-quota-resume","Resume after usage limits","Continue the same provider session when its reported limit resets while bruhs is open.",self.quota_prefs.resume,p.and_then(|p|p.resume_limited)),
            ("settings-quota-snooze","desktop-settings-row-quota-snooze","Snooze until limits reset","Move a limited thread to Snoozed until its provider reports capacity will return.",self.quota_prefs.snooze,p.and_then(|p|p.snooze_limited))]{
            let control=if key.is_empty(){ settings_ui::switch(id,environment,false,label,cx,move|this,_,cx|this.choose_quota_preference(id,if environment{"off"}else{"on"},cx)) }
            else {settings_ui::control_column(self.choice_control(id,label,match project{None=>"inherit",Some(true)=>"on",Some(false)=>"off"},&[("inherit","Environment default"),("on","On"),("off","Off")],cx))};
            rows.push(self.settings_row(selector,label,desc,control));
        }
        self.settings_card("Usage limits", "", rows)
    }

    pub(super) fn handle_quota_limit(
        &mut self,
        chat: &Entity<ChatView>,
        reset_ms: Option<i64>,
        generation: u64,
        cx: &mut Context<'_, Self>,
    ) {
        let view = chat.read(cx);
        let prefs = self.quota_preferences(view.settings_project_path());
        if prefs == Preferences::default() || !view.is_quota_turn(generation) {
            return;
        }
        let Some(id) = view.thread_id.clone() else {
            return;
        };
        let model = view.model_id.clone();
        let provider = bruhs_models::agent::provider_for_model_id(&model);
        if !matches!(
            provider,
            bruhs_models::agent::AgentProvider::Codex | bruhs_models::agent::AgentProvider::Claude
        ) {
            return;
        }
        let lookup = crate::app_state::spawn_tokio(cx, async move {
            if let Some(reset) = reset_ms {
                Some(reset)
            } else {
                bruhs_core::coordination::provider_quota_reset(provider.as_str())
                    .await
                    .ok()
                    .flatten()
                    .map(|t| t.as_millis())
            }
        });
        let thread_key = id.clone();
        let task = cx.spawn(async move |this, cx| {
            let Some(at) = lookup.await else { return };
            let Ok(Some(delay)) = this.update(cx, |this, cx| {
                this.schedule_limited_thread(&id, &model, generation, at, cx)
            }) else {
                return;
            };
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                let Some(t) = this.threads.iter().find(|t| t.id == id) else {
                    return;
                };
                let view = t.chat.read(cx);
                if t.settled
                    || view.model_id != model
                    || !this.quota_preferences(view.settings_project_path()).resume
                {
                    return;
                }
                t.chat
                    .update(cx, |chat, cx| chat.resume_quota_turn(generation, cx));
            });
        });
        self.quota_timers.insert(thread_key, task);
    }

    fn schedule_limited_thread(
        &mut self,
        id: &str,
        model: &str,
        generation: u64,
        at: i64,
        cx: &mut Context<'_, Self>,
    ) -> Option<std::time::Duration> {
        let delay = at
            .checked_sub(EpochMs::now().as_millis())
            .filter(|n| *n > 0)?;
        let index = self.threads.iter().position(|t| t.id == id)?;
        let t = &self.threads[index];
        let view = t.chat.read(cx);
        if t.settled || view.model_id != model || !view.is_quota_turn(generation) {
            return None;
        }
        let prefs = self.quota_preferences(view.settings_project_path());
        if prefs.snooze {
            self.threads[index].snoozed_until_ms = Some(at);
            self.persist_and_notify(cx);
        }
        prefs
            .resume
            .then(|| std::time::Duration::from_millis(delay as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::super::EpochMs;
    #[gpui::test]
    fn quota_policy_scopes_snooze_and_rejects_stale_turns(cx: &mut gpui::TestAppContext) {
        let (workspace, cx, id, _) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |this, _, cx| {
            this.choose_quota_preference("settings-quota-snooze", "on", cx);
            let t = this.threads.iter().find(|t| t.id == id).unwrap();
            let model = t.chat.read(cx).model_id.clone();
            let current = t.chat.read(cx).generation_for_test();
            let at = EpochMs::now().as_millis() + 120_000;
            this.schedule_limited_thread(&id, &model, current.wrapping_add(1), at, cx);
            assert_eq!(
                this.threads
                    .iter()
                    .find(|t| t.id == id)
                    .unwrap()
                    .snoozed_until_ms,
                None
            );
            assert!(this
                .schedule_limited_thread(&id, &model, current, at, cx)
                .is_none());
            assert_eq!(
                this.snapshot(cx)
                    .threads
                    .into_iter()
                    .find(|t| t.id == id)
                    .unwrap()
                    .snoozed_until_ms,
                Some(at)
            );
            this.settings_project_scope = "/tmp/project-scope".into();
            this.choose_quota_preference("settings-quota-resume", "on", cx);
            assert!(this.quota_preferences("/tmp/project-scope").resume);
            assert!(!this.quota_preferences("/tmp/other").resume);
            this.choose_quota_preference("settings-quota-resume", "inherit", cx);
            assert!(!this.quota_preferences("/tmp/project-scope").resume);
        });
    }
}
