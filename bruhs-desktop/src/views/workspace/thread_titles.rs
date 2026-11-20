//! Thread naming from the first user message, including manual regeneration.
use super::threads_ui::{title_instructions_text, titled_like_web};
use super::*;
use crate::app_state::spawn_tokio;
use bruhs_models::agent::{model_info, AgentProvider};
use bruhs_models::logic::prompt_suggestion::default_suggestion_model;

use bruhs_core::prompt_suggestion::SuggestionError;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct PendingTitle {
    #[serde(skip)]
    serial: u64,
    expected: String,
    /// Only initial requests are tied to a specific first message.
    first_message: Option<String>,
    failures: u8,
    retry_at_ms: i64,
}

fn message_key(message: &Message) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    let mut add = |s: &str| {
        hash.update((s.len() as u64).to_le_bytes());
        hash.update(s.as_bytes());
    };
    add(message.text_for_title());
    for file in &message.files {
        add(&file.name);
        add(&file.path);
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn title_conversation(messages: &[Message]) -> bruhs_core::thread_title::Conversation {
    bruhs_core::thread_title::conversation(messages.iter().filter(|m| !m.state.is_streaming()).map(
        |m| bruhs_core::thread_title::TitleTurn {
            role: m.role,
            text: m.text_for_title(),
            files: &m.files,
        },
    ))
}

impl PendingTitle {
    pub(super) fn new(serial: u64, expected: String, first_message: Option<String>) -> Self {
        Self {
            serial,
            expected,
            first_message,
            failures: 0,
            retry_at_ms: 0,
        }
    }

    fn retry_after(&self, error: &SuggestionError) -> Option<Duration> {
        if self.first_message.is_none()
            || matches!(
                error,
                SuggestionError::UsageReserved
                    | SuggestionError::InvalidRequest
                    | SuggestionError::UnsupportedProvider
            )
        {
            return None;
        }
        [2, 4]
            .get(usize::from(self.failures))
            .copied()
            .map(Duration::from_secs)
    }

    fn matches(&self, view: &ChatView) -> bool {
        view.thread_title == self.expected
            && self.first_message.as_ref().is_none_or(|key| {
                view.messages
                    .iter()
                    .find(|m| m.role == MessageRole::User)
                    .is_some_and(|message| message_key(message) == *key)
            })
    }
}

impl Workspace {
    pub(super) fn text_generation_model_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let scope = &self.settings_project_scope;
        let mut choices = Vec::new();
        let selected = if scope.is_empty() {
            self.text_generation_model.as_deref().unwrap_or("automatic")
        } else {
            choices.push(("inherit", "Use environment default"));
            self.project_defaults
                .get(scope)
                .and_then(|p| p.text_generation_model.as_deref())
                .unwrap_or("inherit")
        };
        let selected = if self.generator_unavailable(selected) {"automatic"} else {selected};
        choices.extend([
            ("automatic", "Automatic"),
            ("local", "Local first-message text"),
        ]);
        self.settings_row("desktop-settings-row-text-generation-model","Text generation model",
            "Choose the model for thread titles and worktree branch names. Automatic uses a lightweight model from the thread’s provider.",
            self.text_model_control("settings-text-generation-model","Text generation model",selected,&choices,cx))
    }

    pub(super) fn choose_title_model(&mut self, value: &str, cx: &mut Context<'_, Self>) {
        if !matches!(value, "inherit" | "automatic" | "local")
            && !self.generator_model(value)
            && !model_info(value).is_some_and(|m| !m.provider.is_external())
        {
            return;
        }
        if self.settings_project_scope.is_empty() {
            if value == "inherit" {
                return;
            }
            self.text_generation_model = (value != "automatic").then(|| value.to_owned());
        } else {
            let path = self.settings_project_scope.clone();
            let p = self.project_defaults.entry(path.clone()).or_default();
            p.text_generation_model = (value != "inherit").then(|| value.to_owned());
            if *p == Default::default() {
                self.project_defaults.remove(&path);
            }
        }
        self.stamp_run_prefs_on_threads(cx);
        self.persist_and_notify(cx);
    }

    pub(super) fn effective_text_model(&self, project: Option<&str>, active_model: &str) -> String {
        let selected = project
            .and_then(|p| self.project_defaults.get(p))
            .and_then(|p| p.text_generation_model.as_deref())
            .or(self.text_generation_model.as_deref());
        let selected = selected.filter(|model| !self.generator_unavailable(model));
        match selected {
            None | Some("automatic") => {
                default_suggestion_model(AgentProvider::for_model(active_model)).to_owned()
            }
            Some(model) => model.to_owned(),
        }
    }

    pub(super) fn generate_thread_title(
        &mut self,
        id: &str,
        regenerate: bool,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(thread) = self.threads.iter().find(|t| t.id == id) else {
            return;
        };
        let chat = thread.chat.clone();
        let view = chat.read(cx);
        let first = view.messages.iter().find(|m| m.role == MessageRole::User);
        let model = self.effective_text_model(Some(view.settings_project_path()), &view.model_id);
        let local = model == "local" || model.is_empty();
        let expected = if regenerate && !local {
            view.thread_title.clone()
        } else {
            let instructions = title_instructions_text(
                &self.title_instructions,
                &self.title_instructions_by_project,
                &thread.project_name,
            );
            titled_like_web(
                &view.thread_title,
                first.map(|m| m.content.as_str()),
                &instructions,
            )
        };
        let first_message = if regenerate {
            None
        } else {
            first.map(message_key)
        };
        self.title_requests.remove(id);
        if view.thread_title != expected {
            chat.update(cx, |chat, cx| {
                chat.thread_title.clone_from(&expected);
                cx.notify();
            });
        }
        if local || (!regenerate && first_message.is_none()) {
            return;
        }
        self.title_request_serial = self.title_request_serial.wrapping_add(1);
        let serial = self.title_request_serial;
        self.title_requests.insert(
            id.to_owned(),
            PendingTitle::new(serial, expected, first_message),
        );
        self.start_title_attempt(id, serial, cx);
    }

    pub(super) fn resume_title_requests(&mut self, cx: &mut Context<'_, Self>) {
        for (id, request) in &self.title_requests {
            // Bound the wait even if the system clock moved backwards while closed.
            let remaining = request
                .retry_at_ms
                .saturating_sub(EpochMs::now().as_millis())
                .clamp(0, 4_000);
            self.schedule_title_attempt(
                id,
                request.serial,
                Duration::from_millis(remaining as u64),
                cx,
            );
        }
    }

    fn schedule_title_attempt(
        &self,
        id: &str,
        serial: u64,
        delay: Duration,
        cx: &mut Context<'_, Self>,
    ) {
        let id = id.to_owned();
        cx.spawn(async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let _ = this.update(cx, |this, cx| this.start_title_attempt(&id, serial, cx));
        })
        .detach();
    }

    fn start_title_attempt(&mut self, id: &str, serial: u64, cx: &mut Context<'_, Self>) {
        let Some(request) = self.title_requests.get(id).filter(|r| r.serial == serial) else {
            return;
        };
        let thread = self
            .threads
            .iter()
            .find(|t| t.id == id && request.matches(t.chat.read(cx)));
        let Some(thread) = thread else {
            self.title_requests.remove(id);
            self.persist_and_notify(cx);
            return;
        };
        let view = thread.chat.read(cx);
        let model = self.effective_text_model(Some(view.settings_project_path()), &view.model_id);
        if model == "local" || model.is_empty() {
            self.title_requests.remove(id);
            self.persist_and_notify(cx);
            return;
        }
        let regenerate = request.first_message.is_none();
        let context = if regenerate {
            title_conversation(&view.messages)
        } else {
            view.messages
                .iter()
                .find(|m| m.role == MessageRole::User)
                .map(|m| bruhs_core::thread_title::Conversation {
                    text: m.text_for_title().to_owned(),
                    files: m.files.clone(),
                })
                .unwrap_or_default()
        };
        let source = view.fork.as_ref().map(|fork| {
            let title = self
                .threads
                .iter()
                .find(|t| t.id == fork.parent)
                .map(|parent| parent.chat.read(cx).thread_title.as_str());
            fork.title_source(title)
        });
        if context.text.trim().is_empty()
            && context.files.is_empty()
            && source
                .as_ref()
                .is_none_or(|source| source.history.is_empty())
        {
            if regenerate {
                self.meetings_hub.update(cx, |hub, cx| {
                    hub.show_notice("This thread has no messages to title yet.", true, cx)
                });
            }
            self.title_requests.remove(id);
            self.persist_and_notify(cx);
            return;
        }
        let previous = regenerate.then(|| view.thread_title.clone());
        let project = thread.project_name.clone();
        let root = PathBuf::from(&view.project_path);
        let worktree = view.worktree_path.as_ref().map(PathBuf::from);
        let instructions = title_instructions_text(
            &self.title_instructions,
            &self.title_instructions_by_project,
            &project,
        );
        let id = id.to_owned();
        let attempt = spawn_tokio(cx, async move {
            let cwd = worktree.filter(|p| p.is_dir()).unwrap_or(root);
            bruhs_core::thread_title::title_with_source(
                &model,
                bruhs_core::thread_title::FirstMessage {
                    text: &context.text,
                    files: &context.files,
                },
                &instructions,
                &project,
                &cwd,
                previous.as_deref(),
                source.as_ref(),
            )
            .await
        });
        cx.spawn(async move |this, cx| {
            let result = attempt.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_thread_title(&id, serial, result, cx)
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_thread_title(
        &mut self,
        id: &str,
        serial: u64,
        result: Result<String, SuggestionError>,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .title_requests
            .get(id)
            .is_none_or(|request| request.serial != serial)
        {
            return;
        }
        let mut request = self.title_requests.remove(id).unwrap();
        let Some(thread) = self
            .threads
            .iter()
            .find(|t| t.id == id && request.matches(t.chat.read(cx)))
        else {
            self.persist_and_notify(cx);
            return;
        };
        let chat = thread.chat.clone();
        if let Some(delay) = result
            .as_ref()
            .err()
            .and_then(|error| request.retry_after(error))
        {
            request.failures += 1;
            request.retry_at_ms = EpochMs::now().as_millis() + delay.as_millis() as i64;
            self.title_requests.insert(id.to_owned(), request);
            self.schedule_title_attempt(id, serial, delay, cx);
            self.persist_and_notify(cx);
            return;
        }
        let regenerate = request.first_message.is_none();
        match result {
            Ok(title)
                if title != super::threads_ui::THREAD_TITLE_GENERAL
                    && title != request.expected =>
            {
                chat.update(cx, |view, cx| {
                    view.thread_title = title;
                    cx.notify();
                });
            }
            Ok(_) if regenerate => self.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice(
                    "Title kept. The current title already fits this thread.",
                    false,
                    cx,
                )
            }),
            Err(error) if regenerate => {
                let reason = match error {
                    SuggestionError::InvalidReply => {
                        "The selected model returned an unreadable title.".to_owned()
                    }
                    SuggestionError::TimedOut => "The selected model timed out.".to_owned(),
                    other => other.to_string(),
                };
                self.meetings_hub.update(cx, |hub, cx| {
                    hub.show_notice(
                        format!("Couldn’t regenerate the thread title. {reason}"),
                        true,
                        cx,
                    )
                });
            }
            _ => {}
        }
        self.persist_and_notify(cx);
    }
}

#[cfg(test)]
mod tests {
    fn manual(serial: u64, expected: &str) -> super::PendingTitle {
        super::PendingTitle::new(serial, expected.to_owned(), None)
    }

    #[gpui::test]
    fn initial_title_failures_preserve_the_retry_budget_across_restore(
        cx: &mut gpui::TestAppContext,
    ) {
        use super::{message_key, Message, PendingTitle, SuggestionError};
        let (workspace, cx, _, id) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |ws, _, cx| {
            let chat = ws.threads.iter().find(|t| t.id == id).unwrap().chat.clone();
            let first = Message::user("Look at this");
            let key = message_key(&first);
            let expected = chat.read(cx).thread_title.clone();
            chat.update(cx, |chat, _| chat.messages = vec![first]);
            let request = PendingTitle::new(1, expected.clone(), Some(key));
            assert!(request
                .retry_after(&SuggestionError::UsageReserved)
                .is_none());
            ws.title_requests.insert(id.clone(), request);
            for (serial, delay) in [(1, 2), (0, 4)] {
                assert_eq!(
                    ws.title_requests[&id]
                        .retry_after(&SuggestionError::TimedOut)
                        .unwrap()
                        .as_secs(),
                    delay
                );
                ws.finish_thread_title(&id, serial, Err(SuggestionError::TimedOut), cx);
                let saved = serde_json::to_vec(&ws.snapshot(cx)).unwrap();
                let restored: super::PersistedWorkspaceState =
                    serde_json::from_slice(&saved).unwrap();
                ws.title_requests = restored.title_requests;
                assert!(ws.title_requests[&id].retry_at_ms > 0);
            }
            ws.finish_thread_title(&id, 0, Err(SuggestionError::InvalidReply), cx);
            assert!(ws.title_requests.is_empty());
            assert_eq!(chat.read(cx).thread_title, expected);
        });
    }

    #[gpui::test]
    fn a_project_can_generate_local_titles_and_a_late_model_reply_preserves_manual_renames(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, id) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |this, _, cx| {
            let chat = this
                .threads
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .chat
                .clone();
            this.settings_project_scope = chat.read(cx).settings_project_path().to_owned();
            this.choose_title_model("local", cx);
            let saved = this.snapshot(cx);
            assert_eq!(
                saved.project_defaults[&this.settings_project_scope]
                    .text_generation_model
                    .as_deref(),
                Some("local")
            );
            assert!(saved.text_generation_model.is_none());
            chat.update(cx, |view, _| {
                view.messages = vec![crate::views::chat::Message::user("Fix the project picker")]
            });
            this.regenerate_title_by_id(&id, cx);
            assert_eq!(chat.read(cx).thread_title, "Fix the project picker");
            this.title_requests
                .insert(id.clone(), manual(2, "Fix the project picker"));
            this.finish_thread_title(&id, 1, Ok("Stale".into()), cx);
            assert_eq!(this.title_requests[&id].serial, 2);
            chat.update(cx, |view, _| view.thread_title = "My title".into());
            this.finish_thread_title(&id, 2, Ok("Late".into()), cx);
            assert_eq!(chat.read(cx).thread_title, "My title");
            this.choose_title_model("inherit", cx);
            assert!(this.snapshot(cx).project_defaults.is_empty());
        });
    }
    #[gpui::test]
    fn a_manual_title_request_reports_a_failure_and_an_unchanged_result(
        cx: &mut gpui::TestAppContext,
    ) {
        let (workspace, cx, _, id) = super::super::tests::thread_menu_workspace(cx);
        workspace.update_in(cx, |this, _, cx| {
            let chat = this
                .threads
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .chat
                .clone();
            let title = chat.read(cx).thread_title.clone();
            for (serial, result, warning) in [
                (
                    1,
                    Err(bruhs_core::prompt_suggestion::SuggestionError::InvalidReply),
                    true,
                ),
                (2, Ok(title.clone()), false),
            ] {
                this.title_requests
                    .insert(id.clone(), manual(serial, &title));
                this.finish_thread_title(&id, serial, result, cx);
                assert_eq!(chat.read(cx).thread_title, title);
                assert!(!this.title_requests.contains_key(&id));
                let notice = this.meetings_hub.read(cx).notice.as_ref().unwrap();
                assert_eq!(notice.warning, warning);
                assert!(notice.message.contains(if warning {
                    "unreadable title"
                } else {
                    "Title kept"
                }));
            }
        });
    }
}
