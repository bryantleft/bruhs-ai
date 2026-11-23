//! A due turn is removed by a guarded workspace save before the provider is called.
//! The save receipt only covers claims present in that particular snapshot.
use super::*;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::views::workspace) struct Recovery {
    pub(super) request: Uuid,
    #[serde(skip)]
    generation: u64,
    reset_at: i64,
    model: String,
    session: String,
    project: String,
    worktree: Option<String>,
    permission: PermissionMode,
    effective_permission: PermissionMode,
    message_count: usize,
    started: i64,
    completed: i64,
}

impl Recovery {
    pub(super) fn for_turn(chat: &ChatView, reset_at: i64, generation: u64) -> Option<Self> {
        let last = chat.messages.last()?;
        let session = chat.session_id.as_ref().filter(|id| !id.is_empty())?;
        let completed = last.completed_at_ms?;
        if last.role != MessageRole::Assistant
            || last.state.error().is_none()
            || completed >= reset_at
            || !chat.run_locked
            || chat.fresh_session
            || chat.session_provider
                != Some(bruhs_models::agent::provider_for_model_id(&chat.model_id))
        {
            return None;
        }
        Some(Self {
            request: Uuid::new_v4(),
            generation,
            reset_at,
            model: chat.model_id.clone(),
            session: session.clone(),
            project: chat.project_path.clone(),
            worktree: chat.worktree_path.clone(),
            permission: chat.permission_mode,
            effective_permission: bruhs_models::roles::narrow_access(
                chat.permission_mode,
                chat.role_profile.as_ref().and_then(|r| r.access),
            ),
            message_count: chat.messages.len(),
            started: last.started_at_ms?,
            completed,
        })
    }
    fn matches(&self, chat: &ChatView) -> bool {
        chat.session_provider.is_some_and(|p| {
            matches!(p, AgentProvider::Codex | AgentProvider::Claude)
                && p == bruhs_models::agent::provider_for_model_id(&self.model)
        }) && chat.session_id.as_ref() == Some(&self.session)
            && chat.run_locked
            && !chat.fresh_session
            && chat.model_id == self.model
            && chat.is_quota_turn(self.generation)
            && chat.permission_mode == self.permission
            && bruhs_models::roles::narrow_access(
                chat.permission_mode,
                chat.role_profile.as_ref().and_then(|r| r.access),
            ) == self.effective_permission
            && chat.project_path == self.project
            && chat.worktree_path == self.worktree
            && self.reset_at > self.completed
            && chat.messages.len() == self.message_count
            && chat.messages.last().is_some_and(|m| {
                m.role == MessageRole::Assistant
                    && m.state.error().is_some()
                    && m.started_at_ms == Some(self.started)
                    && m.completed_at_ms == Some(self.completed)
            })
    }
}
impl Workspace {
    pub(in crate::views::workspace) fn cancel_quota_recovery(&mut self, id: &str) {
        self.quota_timers.remove(id);
        self.quota_recoveries.remove(id);
        self.quota_dispatching.remove(id);
    }
    fn recovery_chat(&self, id: &str, r: &Recovery, cx: &App) -> Option<Entity<ChatView>> {
        let thread = self.threads.iter().find(|t| t.id == id && !t.settled)?;
        let chat = thread.chat.read(cx);
        (self.quota_preferences(chat.settings_project_path()).resume && r.matches(chat))
            .then(|| thread.chat.clone())
    }
    pub(in crate::views::workspace) fn restore_quota_recoveries(
        &mut self,
        cx: &mut Context<'_, Self>,
    ) {
        let mut dropped = false;
        for (id, mut r) in self.quota_recoveries.clone() {
            // Starting a restored ChatView gives it a new local generation.
            if let Some(thread) = self.threads.iter().find(|t| t.id == id) {
                r.generation = thread.chat.read(cx).turn_generation();
            }
            if let Some(chat) = self.recovery_chat(&id, &r, cx) {
                chat.update(cx, |chat, cx| {
                    chat.reported_limit = Some((r.generation, Some(r.reset_at)));
                    cx.notify();
                });
                self.arm_quota_timer(&id, r.reset_at, r.request, cx);
                self.quota_recoveries.insert(id, r);
            } else {
                self.quota_recoveries.remove(&id);
                dropped = true;
            }
        }
        if dropped {
            self.persist(cx);
        }
    }
    pub(super) fn arm_quota_timer(
        &mut self,
        id: &str,
        at: i64,
        request: Uuid,
        cx: &mut Context<'_, Self>,
    ) {
        let key = id.to_string();
        let task = cx.spawn(async move |this, cx| {
            loop {
                let now = EpochMs::now().as_millis();
                if now >= at {
                    break;
                }
                // Recheck the wall clock after sleep or a time change.
                let delay = u64::try_from(at.saturating_sub(now).min(60_000)).unwrap_or(0);
                cx.background_executor()
                    .timer(Duration::from_millis(delay))
                    .await;
            }
            let _ = this.update(cx, |ws, cx| ws.claim_quota_recovery(&key, request, cx));
        });
        self.quota_timers.insert(id.to_string(), task);
    }
    fn claim_quota_recovery(&mut self, id: &str, request: Uuid, cx: &mut Context<'_, Self>) {
        if !self
            .quota_recoveries
            .get(id)
            .is_some_and(|r| r.request == request)
        {
            return;
        }
        let Some(r) = self.quota_recoveries.remove(id) else {
            return;
        };
        if self
            .recovery_chat(id, &r, cx)
            .is_some_and(|chat| chat.read(cx).quota_composer_available(cx))
        {
            self.quota_dispatching.insert(id.to_string(), r);
        }
        self.persist_and_notify(cx);
    }
    pub(in crate::views::workspace) fn quota_save_receipts(&self) -> Vec<(String, Uuid)> {
        self.quota_dispatching
            .iter()
            .map(|(id, r)| (id.clone(), r.request))
            .collect()
    }
    pub(in crate::views::workspace) fn dispatch_saved_quota(
        &mut self,
        receipts: Vec<(String, Uuid)>,
        cx: &mut Context<'_, Self>,
    ) {
        for (id, request) in receipts {
            if self
                .quota_dispatching
                .get(&id)
                .is_some_and(|r| r.request == request)
            {
                let r = self
                    .quota_dispatching
                    .remove(&id)
                    .expect("matching recovery");
                if let Some(chat) = self.recovery_chat(&id, &r, cx) {
                    chat.update(cx, |chat, cx| chat.resume_quota_turn(cx));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
