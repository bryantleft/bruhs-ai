//! Recover only the latest provider-accepted user turn. The OS lock and its
//! run ID distinguish an interrupted process from a second open desktop or a
//! terminal event that reached the provider before the workspace finished saving.
use super::*;
use uuid::Uuid;

const MAX_RESTARTS: u8 = 3;
const PROMPT: &str = "Continue the task that was running before bruhs restarted. Check what already completed before taking the next step.";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(in crate::views::workspace) struct Interrupted {
    #[serde(flatten)]
    source: Source,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(in crate::views::workspace) reply: Option<PersistedMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Source {
    run: Uuid,
    depth: u8,
    session: String,
    model: String,
    project: String,
    worktree: Option<String>,
    permission: PermissionMode,
    effective_permission: PermissionMode,
    messages: usize,
    user_started: i64,
}

impl Source {
    fn from_chat(chat: &ChatView, run: Uuid, depth: u8) -> Option<Self> {
        if !chat.run_locked
            || chat.fresh_session
            || depth >= MAX_RESTARTS
            || chat.session_provider
                != Some(bruhs_models::agent::provider_for_model_id(&chat.model_id))
        {
            return None;
        }
        // A live snapshot omits the streaming assistant placeholder.
        let messages = if chat.messages.last()?.state.is_streaming() {
            chat.messages.len() - 1
        } else {
            chat.messages.len()
        };
        let user = chat.messages.get(messages.checked_sub(1)?)?;
        if user.role != MessageRole::User {
            return None;
        }
        Some(Self {
            run,
            depth,
            session: chat.session_id.clone().filter(|s| !s.is_empty())?,
            model: chat.model_id.clone(),
            project: chat.project_path.clone(),
            worktree: chat.worktree_path.clone(),
            permission: chat.permission_mode,
            effective_permission: bruhs_models::roles::narrow_access(
                chat.permission_mode,
                chat.role_profile.as_ref().and_then(|r| r.access),
            ),
            messages,
            user_started: user.started_at_ms?,
        })
    }
}
impl Workspace {
    pub(in crate::views::workspace) fn continue_threads(&self, path: &str) -> bool {
        self.project_defaults
            .get(path)
            .and_then(|p| p.continue_threads)
            .unwrap_or(self.maintenance.prefs.continue_threads)
    }

    pub(super) fn restart_setting(&self, cx: &Context<'_, Self>) -> AnyElement {
        self.scoped_switch_row(
            ScopedSwitch::Restart,
            "settings-continue-threads",
            "desktop-settings-row-continue-threads",
            "Continue threads after restarts",
            "Resume active sessions after restart.",
            self.maintenance.prefs.continue_threads,
            cx,
        )
    }

    fn restart_eligible(&self, t: &DesktopThread, cx: &App) -> bool {
        let chat = t.chat.read(cx);
        !t.settled
            && !t
                .snoozed_until_ms
                .is_some_and(|at| at > EpochMs::now().as_millis())
            && self.continue_threads(chat.settings_project_path())
            && chat.recovery_composer_clear(cx)
    }

    pub(in crate::views::workspace) fn interrupted_runs(
        &self,
        cx: &App,
    ) -> BTreeMap<String, Interrupted> {
        self.threads
            .iter()
            .filter(|t| {
                (t.chat.read(cx).is_streaming || self.foreign_runs.contains_key(&t.id))
                    && self.restart_eligible(t, cx)
            })
            .filter_map(|t| {
                let chat = t.chat.read(cx);
                let record = if let Some((run, depth)) = chat.restart_marker() {
                    let source = Source::from_chat(chat, run, depth)?;
                    let reply = chat
                        .messages
                        .last()
                        .filter(|m| m.role == MessageRole::Assistant && m.state.is_streaming())
                        .map(PersistedMessage::from_message);
                    Interrupted { source, reply }
                } else {
                    let previous = self.foreign_runs.get(&t.id)?;
                    if chat.is_streaming
                        || Source::from_chat(chat, previous.source.run, previous.source.depth)
                            .as_ref()
                            != Some(&previous.source)
                    {
                        return None;
                    }
                    previous.clone()
                };
                Some((t.id.clone(), record))
            })
            .collect()
    }

    pub(in crate::views::workspace) fn restore_interrupted_runs(
        &mut self,
        runs: BTreeMap<String, Interrupted>,
        cx: &mut Context<'_, Self>,
    ) {
        for (id, record) in runs {
            let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
                continue;
            };
            if !self.restart_eligible(thread, cx) {
                continue;
            }
            let chat = thread.chat.clone();
            let view = chat.read(cx);
            if !view.may_resume_owned_session(cx)
                || Source::from_chat(view, record.source.run, record.source.depth).as_ref()
                    != Some(&record.source)
            {
                continue;
            }
            let Ok(thread_id) = bruhs_models::ids::ThreadId::try_from(id.as_str()) else {
                continue;
            };
            match bruhs_core::reactive::claim_desktop_turn(&thread_id, Some(record.source.run)) {
                Ok(lock) => {
                    chat.update(cx, |chat, cx| {
                        if let Some(saved) = record.reply {
                            let mut reply = saved.into_message();
                            reply.state = TurnState::InterruptedByRestart;
                            chat.messages.push(reply);
                        }
                        chat.resume_owned_session(PROMPT, record.source.depth + 1, Some(lock), cx)
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.foreign_runs.insert(id, record);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(%error,"Could not inspect the interrupted thread"),
            }
        }
    }
}
