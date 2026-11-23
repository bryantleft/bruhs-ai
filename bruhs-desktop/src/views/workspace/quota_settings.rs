//! Only a provider-reported future reset can schedule a limited desktop thread.
use super::*;
mod recovery;
pub(super) use recovery::Recovery;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub(super) snooze: bool,
    pub(super) resume: bool,
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
        let setting = match id {
            "settings-quota-snooze" => ScopedSwitch::Snooze,
            "settings-quota-resume" => ScopedSwitch::Resume,
            _ => return,
        };
        if on.is_some() || !self.settings_project_scope.is_empty() {
            setting.choose(self, on, cx);
        }
    }

    pub(super) fn cancel_disabled_quota_recoveries(&mut self, cx: &App) {
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
            self.cancel_quota_recovery(&id);
        }
    }

    pub(super) fn render_quota_settings(&self, cx: &Context<'_, Self>) -> AnyElement {
        let rows = vec![
            self.scoped_switch_row(ScopedSwitch::Resume, "settings-quota-resume", "desktop-settings-row-quota-resume",
                "Resume after usage limits", "Continue the interrupted session when its limit resets. If bruhs is closed, continue the next time it opens.",
                self.quota_prefs.resume, cx),
            self.scoped_switch_row(ScopedSwitch::Snooze, "settings-quota-snooze", "desktop-settings-row-quota-snooze",
                "Snooze until limits reset", "Move a limited thread to Snoozed until its provider reports capacity will return.",
                self.quota_prefs.snooze, cx),
        ];
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
        if prefs == Preferences::default()
            || !view.is_quota_turn(generation)
            || !view
                .messages
                .last()
                .is_some_and(|m| m.role == MessageRole::Assistant && m.state.error().is_some())
        {
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
        // A reset supplied with the failure can be saved with the finished turn.
        self.cancel_quota_recovery(&id);
        if let Some(at) = reset_ms {
            self.schedule_limited_thread(&id, &model, generation, at, cx);
            return;
        }
        let lookup = crate::app_state::spawn_tokio(cx, async move {
            bruhs_core::coordination::provider_quota_reset(provider.as_str())
                .await
                .ok()
                .flatten()
                .map(|t| t.as_millis())
        });
        let key = id.clone();
        let task = cx.spawn(async move |this, cx| {
            if let Some(at) = lookup.await {
                let _ = this.update(cx, |this, cx| {
                    this.schedule_limited_thread(&id, &model, generation, at, cx);
                });
            }
        });
        self.quota_timers.insert(key, task);
    }

    fn schedule_limited_thread(
        &mut self,
        id: &str,
        model: &str,
        generation: u64,
        at: i64,
        cx: &mut Context<'_, Self>,
    ) -> Option<std::time::Duration> {
        let delay = at.saturating_sub(EpochMs::now().as_millis());
        // The reference worker can arm a recently missed reset after waking up.
        if delay < -86_400_000 {
            return None;
        }
        let index = self.threads.iter().position(|t| t.id == id)?;
        let t = &self.threads[index];
        let view = t.chat.read(cx);
        if t.settled || view.model_id != model || !view.is_quota_turn(generation) {
            return None;
        }
        let prefs = self.quota_preferences(view.settings_project_path());
        let recovery = prefs
            .resume
            .then(|| Recovery::for_turn(view, at, generation))
            .flatten();
        if prefs.snooze && delay > 0 {
            self.threads[index].snoozed_until_ms = Some(at);
        }
        if let Some(recovery) = recovery {
            self.arm_quota_timer(id, at, recovery.request, cx);
            self.quota_recoveries.insert(id.to_owned(), recovery);
        }
        if prefs.snooze || self.quota_recoveries.contains_key(id) {
            self.persist_and_notify(cx);
        }
        self.quota_recoveries
            .contains_key(id)
            .then(|| std::time::Duration::from_millis(delay.max(0).cast_unsigned()))
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
            let current = t.chat.read(cx).turn_generation();
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
