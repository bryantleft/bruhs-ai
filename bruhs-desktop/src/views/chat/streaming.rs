//! A turn's life: submitting, the agent stream task, applying its events, stopping, and permission replies.

use super::*;
use bruhs_models::logic::attachments::{UploadedFile, SEND_WAITS};
use bruhs_models::logic::commands::{classify_command, prepare_command, CommandKind, GoalWrite};
use bruhs_models::logic::compact::{
    before_expiry_due, cache_window_ms, show_return_chip, window_tokens,
};
use bruhs_models::logic::composer_text::{
    composer_edit, exceeds_message_limit_with, paste_name, paste_outcome, ComposerEdit,
    ComposerKey, PasteOutcome, TextAttachment,
};
use bruhs_models::logic::context_chip::{expand_prompt, extra_limit_chars, ContextChip};
use bruhs_models::logic::follow_up::{
    composer_action, provider_can_steer, ComposerAction, FollowUp, STEER_UNAVAILABLE,
};
use bruhs_models::logic::queue::QueuedPrompt;
use bruhs_models::logic::recall::{
    compose_after_rewind, logical_line_edges, recall_step, sent_prompts, RecallAction,
    RecallDirection,
};
use bruhs_models::logic::run_place::{first_send, FirstSend, RememberedWorktree};
use bruhs_models::num::ToFloatLossy;

#[cfg(debug_assertions)]
pub(super) const DESKTOP_SMOKE_LOG_ENV: &str = "BRUHS_DESKTOP_SMOKE_LOG";
#[cfg(debug_assertions)]
pub(super) const DESKTOP_SMOKE_PROMPT_ENV: &str = "BRUHS_DESKTOP_SMOKE_PROMPT";

// --- Braille spinner frames for streaming indicator ---
pub const DESKTOP_SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl AgentUiEvent {
    pub(super) fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error(_))
    }
}

/// The saved session a `provider` turn resumes: only one that provider
/// owns. Another provider's id would be meaningless to its CLI.
pub(super) fn session_for_provider(
    session_id: Option<&str>,
    owner: Option<AgentProvider>,
    provider: AgentProvider,
) -> Option<String> {
    session_id
        .filter(|id| !id.is_empty() && owner == Some(provider))
        .map(str::to_string)
}

/// `prompt` with an earlier conversation ahead of it. A slash command goes
/// out as typed: the CLI reads it only at the start of the prompt.
pub(super) fn prompt_with_import_context(context: Option<&str>, prompt: String) -> String {
    match context {
        Some(context) => bruhs_models::logic::portable::with_import_context(context, &prompt),
        None => prompt,
    }
}

impl ChatView {
    /// `messages` as context for a turn whose session does not hold them.
    pub(super) fn conversation_context(&self, messages: &[Message]) -> Option<String> {
        bruhs_models::logic::portable::thread_context(
            &self.thread_title,
            &crate::views::workspace::portable_messages(messages),
        )
    }

    /// The prompt for a cold retry of `turn` (it resumes a session the
    /// provider may have lost): the conversation before this turn, then
    /// the prompt. `None` when the turn starts fresh or already carries it.
    fn cold_retry_prompt(&self, turn: &AgentTurn) -> Option<String> {
        if turn.session_id.is_none() || turn.fresh_session || self.handoff_pending {
            return None;
        }
        // begin_turn pushed this turn's prompt and the reply placeholder.
        let earlier = &self.messages[..self.messages.len().saturating_sub(2)];
        self.conversation_context(earlier)
            .map(|context| prompt_with_import_context(Some(&context), turn.prompt.clone()))
    }

    /// The provider announced this turn's session: keep it for the next
    /// turn and for restarts. The session now holds any imported transcript.
    pub(super) fn record_session(&mut self, id: String) {
        self.session_id = Some(id);
        self.session_provider = Some(self.turn_provider);
        self.fresh_session = false;
        self.import_context = None;
    }
}

pub(super) fn mark_streaming_message_stopped_like_web(message: &mut Message) {
    if message.state.is_streaming() {
        message.state = TurnState::Stopped;
    }
}

/// Reason sent back to the CLI when the user clicks Deny.
pub(super) const PERMISSION_DENIED_REASON: &str = "Denied by the user in bruhs-desktop";

fn pasted_image_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "paste.jpg",
        ImageFormat::Gif => "paste.gif",
        ImageFormat::Webp => "paste.webp",
        ImageFormat::Png
        | ImageFormat::Svg
        | ImageFormat::Bmp
        | ImageFormat::Tiff
        | ImageFormat::Ico
        | ImageFormat::Pnm => "paste.png",
    }
}

pub(super) fn desktop_composer_key(key: &str, shift: bool) -> Option<ComposerKey> {
    match (key, shift) {
        ("enter", true) => Some(ComposerKey::ShiftEnter),
        ("enter", false) => Some(ComposerKey::Enter),
        ("tab", true) => Some(ComposerKey::ShiftTab),
        ("tab", false) => Some(ComposerKey::Tab),
        ("backspace", false) => Some(ComposerKey::Backspace),
        _ => None,
    }
}

fn splice_text(text: &str, start: usize, end: usize, insert: &str) -> String {
    let start = floor_char_boundary(text, start);
    let end = floor_char_boundary(text, end).max(start);
    let mut out = String::with_capacity(text.len() - (end - start) + insert.len());
    out.push_str(&text[..start]);
    out.push_str(insert);
    out.push_str(&text[end..]);
    out
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Drops the inline prompt when the provider withdraws the request it is
/// showing. A cancellation for some other id (already answered, or a stale
/// turn) leaves the current prompt alone.
pub(super) fn clear_permission_if_matches(
    pending: &mut Option<DesktopPermissionRequest>,
    request_id: &str,
) -> bool {
    if pending
        .as_ref()
        .is_some_and(|request| request.id == request_id)
    {
        *pending = None;
        true
    } else {
        false
    }
}

pub(super) fn desktop_permission_request_from_raw(
    value: &serde_json::Value,
) -> Option<DesktopPermissionRequest> {
    let payload =
        if value.get("type").and_then(serde_json::Value::as_str) == Some("permission_request") {
            value
        } else {
            value.get("data")?
        };
    if payload.get("type").and_then(serde_json::Value::as_str) != Some("permission_request") {
        return None;
    }

    let id = payload
        .get("id")
        .or_else(|| payload.get("request_id"))
        .and_then(serde_json::Value::as_str)?
        .to_string();
    let tool = payload
        .get("tool")
        .and_then(serde_json::Value::as_str)?
        .to_string();
    let input = payload.get("input").cloned().unwrap_or_default();

    Some(DesktopPermissionRequest { id, tool, input })
}

#[cfg(debug_assertions)]
pub fn desktop_smoke_prompt() -> Option<String> {
    let prompt = std::env::var(DESKTOP_SMOKE_PROMPT_ENV).ok()?;
    let prompt = prompt.trim().to_string();
    (!prompt.is_empty()).then_some(prompt)
}

/// `BRUHS_DESKTOP_SMOKE_LOG`: stream agent events to stdout for a launch
/// check to read (a protocol for the smoke script, not logging).
#[cfg(debug_assertions)]
pub(super) fn smoke_log(event: &str, payload: &str) {
    if std::env::var_os(DESKTOP_SMOKE_LOG_ENV).is_none() {
        return;
    }

    println!("BRUHS_DESKTOP_SMOKE {event} {payload}");
}

#[cfg(not(debug_assertions))]
pub(super) fn smoke_log(_event: &str, _payload: &str) {}

/// Run `turn` once `previous_turn` (already aborted) has unwound, so the
/// thread is free again, and forward its events to the chat view.
///
/// A turn that resumed a saved session and failed before the provider
/// announced one is retried once as a fresh session (the saved id is
/// stale: the CLI no longer has it), like the web's cold retry.
///
/// `retry_prompt` replaces the prompt on that retry: the new session has
/// none of the conversation, so it carries it.
pub(super) async fn stream_agent_response(
    app: Arc<bruhs_core::AppState>,
    previous_turn: Option<JoinHandle<()>>,
    turn: Result<AgentTurn, IdError>,
    mut retry_prompt: Option<String>,
    tx: UnboundedSender<AgentUiEvent>,
) {
    if let Some(previous_turn) = previous_turn {
        // Only its completion matters; an aborted task reports `Cancelled`.
        let _ = previous_turn.await;
    }
    let mut turn = match turn {
        Ok(turn) => turn,
        Err(e) => {
            let _ = tx.send(AgentUiEvent::Error(format!("invalid thread id: {e}")));
            return;
        }
    };
    let mut cold_retried = false;
    loop {
        let resumed = turn.session_id.is_some() && !turn.fresh_session;
        let mut stream = match app.send_message(turn.clone()).await {
            Ok(stream) => stream,
            Err(error) => {
                let _ = tx.send(AgentUiEvent::Error(error.to_string()));
                return;
            }
        };
        let mut saw_session = false;
        let Some(message) = forward_turn_events(&mut stream, &tx, &mut saw_session).await else {
            break;
        };
        if resumed && !saw_session && !cold_retried {
            // Dropping the stream frees the thread for the retry.
            drop(stream);
            cold_retried = true;
            let _ = tx.send(AgentUiEvent::SessionRejected);
            turn = turn.fresh();
            if let Some(prompt) = retry_prompt.take() {
                turn.prompt = prompt;
            }
            continue;
        }
        let _ = tx.send(AgentUiEvent::Error(message));
        return;
    }

    let _ = tx.send(AgentUiEvent::Done);
}

/// Forward one attempt's events until it ends. Returns the error that ended
/// it, if one did; `saw_session` records whether the provider announced a
/// session first.
async fn forward_turn_events(
    stream: &mut bruhs_core::OrchestratorEventStream,
    tx: &UnboundedSender<AgentUiEvent>,
    saw_session: &mut bool,
) -> Option<String> {
    while let Some(event) = stream.next().await {
        match event {
            OrchestratorEvent::Query(QueryEvent::SecretRequest(request)) => {
                let _ = tx.send(AgentUiEvent::SecretRequest(request));
            }
            OrchestratorEvent::Query(QueryEvent::SecretResolved { id }) => {
                let _ = tx.send(AgentUiEvent::SecretResolved(id));
            }
            OrchestratorEvent::Query(QueryEvent::ThreadRequest(request)) => {
                let _ = tx.send(AgentUiEvent::ThreadRequest(request));
            }
            OrchestratorEvent::Query(QueryEvent::Text(text)) => {
                let _ = tx.send(AgentUiEvent::Text(text));
            }
            OrchestratorEvent::Query(QueryEvent::Thinking(text)) => {
                let _ = tx.send(AgentUiEvent::Thinking(text));
            }
            OrchestratorEvent::Query(QueryEvent::ToolCall { id, tool, input }) => {
                let _ = tx.send(AgentUiEvent::ToolCall { id, tool, input });
            }
            OrchestratorEvent::Query(QueryEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            }) => {
                let _ = tx.send(AgentUiEvent::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                });
            }
            OrchestratorEvent::Query(QueryEvent::Usage(usage)) => {
                let _ = tx.send(AgentUiEvent::Usage(usage));
            }
            OrchestratorEvent::Query(QueryEvent::Result {
                duration_ms, usage, ..
            }) => {
                let _ = tx.send(AgentUiEvent::Result { duration_ms, usage });
            }
            OrchestratorEvent::Query(QueryEvent::Init { session_id, .. }) => {
                if !session_id.is_empty() {
                    *saw_session = true;
                    let _ = tx.send(AgentUiEvent::Session(session_id));
                }
            }
            OrchestratorEvent::Query(QueryEvent::Error(message)) => {
                return Some(message);
            }
            OrchestratorEvent::Query(QueryEvent::PermissionRequest {
                id, tool, input, ..
            }) => {
                let _ = tx.send(AgentUiEvent::PermissionRequest(DesktopPermissionRequest {
                    id,
                    tool,
                    input,
                }));
            }
            OrchestratorEvent::Query(QueryEvent::PermissionCancelled { id }) => {
                let _ = tx.send(AgentUiEvent::PermissionCancelled(id));
            }
            OrchestratorEvent::Query(QueryEvent::Raw(value)) => {
                if let Some(request) = desktop_permission_request_from_raw(&value) {
                    let _ = tx.send(AgentUiEvent::PermissionRequest(request));
                }
            }
            OrchestratorEvent::BackgroundTaskRegistered(task) => {
                let _ = tx.send(AgentUiEvent::ToolCall {
                    id: format!("background-{}", task.id),
                    tool: "Background task".to_string(),
                    input: serde_json::json!({
                        "command": task.label,
                    }),
                });
            }
        }
    }

    None
}

impl ChatView {
    pub(super) fn start_streaming_spinner(&mut self, cx: &mut Context<'_, Self>) {
        self.spinner_frame = 0;
        self.spinner_task.take();
        self.spinner_task = Some(cx.spawn(async move |chat, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            let keep_running = chat.update(cx, |chat, cx| {
                if !chat.is_streaming {
                    return false;
                }
                chat.spinner_frame = (chat.spinner_frame + 1) % DESKTOP_SPINNER_FRAMES.len();
                cx.notify();
                true
            });
            if !matches!(keep_running, Ok(true)) {
                break;
            }
        }));
    }

    #[cfg(test)]
    pub(crate) fn start_spinner_for_test(&mut self, cx: &mut Context<'_, Self>) {
        self.is_streaming = true;
        self.start_streaming_spinner(cx);
    }

    pub(super) fn send_message(
        &mut self,
        _: &SendMessage,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.send_follow_up(false, window, cx);
    }

    /// Enter queues or steers while a turn runs. Cmd/Ctrl+Enter asks for the
    /// other one. An empty composer interrupts, or resumes a held queue.
    pub(super) fn send_follow_up(
        &mut self,
        opposite: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.queue_edit.is_some() {
            if !opposite {
                self.commit_queue_edit(window, cx);
            }
            return;
        }
        let draft = self.input_state.read(cx).value().to_string();
        if exceeds_message_limit_with(&draft, extra_limit_chars(&self.context_chips)) {
            return;
        }
        let trimmed = draft.trim().to_string();
        // Before the queue and steer paths: a review is never a turn
        // (web: `submit`).
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        if let CommandKind::SecondOpinion(focus) = classify_command(&trimmed, provider) {
            if self.is_streaming {
                cx.emit(ChatViewEvent::Notice(
                    bruhs_models::logic::handoff::SECOND_OPINION_WAIT.to_string(),
                ));
                return;
            }
            self.chip_guard = true;
            self.context_chips.clear();
            self.chip_draft.clear();
            self.write_composer("", window, cx);
            self.run_second_opinion(focus, cx);
            return;
        }
        let typed_empty = trimmed.is_empty();
        let draft_empty = if self.is_streaming {
            typed_empty
        } else {
            typed_empty && self.text_attachments.is_empty() && !self.has_ready_upload()
        };
        let has_queued = self
            .thread_id
            .as_ref()
            .is_some_and(|id| self.prompt_queue.has_for(id));
        let action = composer_action(
            self.is_streaming,
            draft_empty,
            opposite,
            self.queue_held,
            has_queued,
            self.follow_up_now(),
        );
        match action {
            ComposerAction::Steer => {
                self.steer_draft(draft, window, cx);
            }
            ComposerAction::SteerUnavailable => {
                cx.emit(ChatViewEvent::Notice(STEER_UNAVAILABLE.to_string()));
            }
            ComposerAction::Interrupt => self.stop_streaming(cx),
            ComposerAction::Resume => {
                self.queue_held = false;
                self.drain_queue(cx);
            }
            ComposerAction::Queue | ComposerAction::Send => {
                // A file that is still uploading, or a failed one, keeps the
                // draft. Queue and retry do not take the offers.
                if !self.is_streaming && self.uploads_block_send() {
                    cx.emit(ChatViewEvent::Notice(SEND_WAITS.to_string()));
                    return;
                }
                // Steer already returned. A local command consumes the draft
                // here. A command that sends waits for `finish_submit`, which
                // applies the goal when the turn actually starts. `/compact`
                // on Claude is one of those sends.
                if !typed_empty {
                    let prepared =
                        prepare_command(&trimmed, provider, self.goal.as_ref(), self.is_streaming);
                    if !prepared.send {
                        self.store_goal(prepared.write, cx);
                        if let Some(notice) = prepared.notice {
                            cx.emit(ChatViewEvent::Notice(notice));
                        }
                        if prepared.stop {
                            self.stop_streaming(cx);
                        }
                        if prepared.queue {
                            if self.queue_follow_up(&draft, cx) {
                                self.chip_guard = true;
                                self.chip_draft.clear();
                                self.write_composer("", window, cx);
                            }
                        } else {
                            self.chip_guard = true;
                            self.context_chips.clear();
                            self.chip_draft.clear();
                            self.write_composer("", window, cx);
                        }
                        return;
                    }
                }
                if matches!(action, ComposerAction::Queue) {
                    if self.queue_follow_up(&draft, cx) {
                        self.chip_guard = true;
                        self.chip_draft.clear();
                        self.write_composer("", window, cx);
                    }
                    return;
                }
                if typed_empty && self.text_attachments.is_empty() && !self.has_ready_upload() {
                    return;
                }
                // A worktree is still being created for the draft already taken.
                if self.placing {
                    return;
                }
                if self.summarize_then_send(&draft, &trimmed, window, cx) {
                    return;
                }
                // This message goes now. Anything already queued follows it.
                self.queue_held = false;
                let attachments = std::mem::take(&mut self.text_attachments);
                let chips = std::mem::take(&mut self.context_chips);
                let uploads = std::mem::take(&mut self.file_offers)
                    .into_iter()
                    .filter_map(|offer| offer.ready_file())
                    .collect();
                self.chip_guard = true;
                self.chip_draft.clear();
                self.finish_submit(draft, attachments, chips, uploads, cx);
                self.write_composer("", window, cx);
                self.clear_mention_hints();
            }
        }
    }

    pub(super) fn follow_up_now(&self) -> FollowUp {
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        FollowUp {
            wants_steer: self.follow_up_steer,
            can_steer: provider_can_steer(provider),
        }
    }

    /// Write the draft into the Claude turn that is already streaming here.
    fn steer_draft(&mut self, draft: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.uploads_block_send() {
            cx.emit(ChatViewEvent::Notice(SEND_WAITS.to_string()));
            return;
        }
        let Some(thread_id) = self.thread_id.clone() else {
            cx.emit(ChatViewEvent::Notice(STEER_UNAVAILABLE.to_string()));
            return;
        };
        let chips = std::mem::take(&mut self.context_chips);
        let attachments = std::mem::take(&mut self.text_attachments);
        let offers = std::mem::take(&mut self.file_offers);
        let uploads: Vec<UploadedFile> = offers
            .iter()
            .filter_map(|offer| offer.ready_file())
            .collect();
        let expanded = expand_prompt(&draft, &chips);
        let visible = super::files::visible_with_files(&expanded, &attachments, &uploads);
        if visible.trim().is_empty() {
            self.context_chips = chips;
            self.text_attachments = attachments;
            self.file_offers = offers;
            return;
        }
        let paste_files = bruhs_core::write_paste_files(&attachments);
        let prompt =
            super::files::turn_prompt(&visible, &paste_files, &uploads, self.goal.as_ref());
        let index = self.messages.len().saturating_sub(1);
        self.messages
            .insert(index, Message::user_with_files(visible.clone(), uploads));
        self.chip_guard = true;
        self.chip_draft.clear();
        self.write_composer("", window, cx);
        self.clear_mention_hints();
        self.scroll_messages_to_bottom();
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();

        let app = Arc::clone(&cx.global::<DesktopState>().app);
        let job = spawn_tokio(cx, async move {
            match ThreadId::try_from(thread_id.as_str()) {
                Ok(id) => app.steer(&id, &prompt).await,
                Err(error) => Err(bruhs_core::SteerError::Transport(error.to_string())),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                let Err(error) = result else {
                    return;
                };
                cx.emit(ChatViewEvent::Notice(error.to_string()));
                if this
                    .messages
                    .get(index)
                    .is_some_and(|message| message.content == visible)
                {
                    this.messages.remove(index);
                }
                if this.input_state.read(cx).value().trim().is_empty() {
                    this.text_attachments = attachments;
                    this.file_offers = offers;
                    this.pending_chips = Some(chips);
                    this.pending_composer = Some(draft);
                }
                cx.emit(ChatViewEvent::MessagesChanged);
                cx.notify();
            });
        })
        .detach();
    }

    /// Enter on a stale Claude thread summarizes, then sends the draft.
    fn summarize_then_send(
        &mut self,
        draft: &str,
        trimmed: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.return_chip_visible()
            || self.compact_full
            || !self.text_attachments.is_empty()
            || !self.file_offers.is_empty()
        {
            return false;
        }
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        if classify_command(trimmed, provider) == CommandKind::Compact {
            return false;
        }
        self.queue_held = true;
        let queued = self.queue_follow_up(draft, cx);
        if queued {
            self.chip_guard = true;
            self.chip_draft.clear();
            self.write_composer("", window, cx);
            self.compact_full = false;
            self.finish_submit(
                "/compact".to_string(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                cx,
            );
            self.clear_mention_hints();
        }
        self.queue_held = false;
        queued
    }

    /// Tokens in the context window after the latest turn that reported usage.
    pub(super) fn latest_context_tokens(&self) -> Option<u64> {
        self.messages.iter().rev().find_map(|message| {
            message.usage.as_ref().map(|usage| {
                window_tokens(
                    usage.context_tokens,
                    usage.input_tokens,
                    usage.cache_read_input_tokens,
                    usage.cache_creation_input_tokens,
                )
            })
        })
    }

    fn idle_for_ms(&self) -> f64 {
        let now = EpochMs::now().as_millis();
        let Some(then) = self
            .messages
            .iter()
            .rev()
            .find_map(|message| message.completed_at_ms.or(message.started_at_ms))
        else {
            return 0.0;
        };
        now.saturating_sub(then).to_f64_lossy()
    }

    pub(super) fn return_chip_visible(&self) -> bool {
        if self.is_streaming || !self.effort_chip_visible() {
            return false;
        }
        let Some(tokens) = self.latest_context_tokens() else {
            return false;
        };
        show_return_chip(
            self.compact_idle,
            tokens,
            self.idle_for_ms(),
            cache_window_ms(bruhs_core::claude_uses_api_key()),
        )
    }

    /// Compact this thread in the lead time before its prompt cache expires.
    /// The draft stays in the composer.
    pub(crate) fn compact_idle_if_due(&mut self, cx: &mut Context<'_, Self>) {
        if self.is_streaming || self.placing || self.queue_held || !self.effort_chip_visible() {
            return;
        }
        if self
            .thread_id
            .as_deref()
            .is_some_and(|id| self.prompt_queue.has_for(id))
        {
            return;
        }
        let Some(tokens) = self.latest_context_tokens() else {
            return;
        };
        let window = cache_window_ms(bruhs_core::claude_uses_api_key());
        if !before_expiry_due(self.compact_idle, tokens, self.idle_for_ms(), window) {
            return;
        }
        self.finish_submit(
            "/compact".to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            cx,
        );
    }

    pub(super) fn queue_follow_up(&mut self, text: &str, cx: &mut Context<'_, Self>) -> bool {
        let Some(thread_id) = self.thread_id.clone() else {
            return false;
        };
        let at_ms = EpochMs::now().as_millis().to_f64_lossy();
        let chips = std::mem::take(&mut self.context_chips);
        if self
            .prompt_queue
            .push_with_chips(&thread_id, text, &chips, at_ms)
            .is_none()
        {
            self.context_chips = chips;
            return false;
        }
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();
        true
    }

    /// Send the oldest queued prompt once the thread is idle and not held.
    /// Called after the stream task has finished, so it can start the next one.
    pub(super) fn drain_queue(&mut self, cx: &mut Context<'_, Self>) {
        if self.is_streaming || self.queue_held || self.placing {
            return;
        }
        let Some(thread_id) = self.thread_id.clone() else {
            return;
        };
        let Some(next) = self.prompt_queue.next_for(&thread_id) else {
            self.drop_queue_edit_if_removed(None);
            return;
        };
        self.drop_queue_edit_if_removed(Some(&next.id));
        let text = next.text;
        let chips = next.chips;
        self.finish_submit(text, Vec::new(), chips, Vec::new(), cx);
        cx.emit(ChatViewEvent::MessagesChanged);
    }

    /// Take a message another thread's agent sent here: run it now when the
    /// thread is free, otherwise queue it behind what already waits. Returns
    /// whether its turn started. A held queue stays held.
    pub fn accept_thread_message(&mut self, prompt: &str, cx: &mut Context<'_, Self>) -> bool {
        let thread_id = self
            .thread_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let free = !(self.is_streaming
            || self.placing
            || self.queue_held
            || self.prompt_queue.has_for(&thread_id));
        if free {
            self.submit_prompt(prompt.to_string(), cx);
            // A turn that could not start (a local command, a missing
            // project) leaves the thread idle.
            return self.is_streaming || self.placing;
        }
        let at_ms = EpochMs::now().as_millis().to_f64_lossy();
        self.prompt_queue.push(&thread_id, prompt, at_ms);
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();
        false
    }

    /// How the message `prompt`, sent here by another thread's agent that is
    /// waiting, ended. `None` while it is still queued or running.
    pub fn thread_message_answer(&self, prompt: &str) -> Option<ThreadMessageAnswer> {
        let prompt = prompt.trim();
        let queued = self.thread_id.as_deref().is_some_and(|id| {
            self.prompt_queue
                .for_thread(id)
                .iter()
                .any(|item| item.text.trim() == prompt)
        });
        if queued || self.placing {
            return None;
        }
        let Some(asked) = self.messages.iter().rposition(|message| {
            message.role == MessageRole::User && message.content.trim() == prompt
        }) else {
            return Some(ThreadMessageAnswer::Removed);
        };
        let reply = self.messages[asked + 1..]
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::Assistant);
        match reply.map(|reply| (&reply.state, reply)) {
            Some((TurnState::Streaming, _)) => None,
            None if self.is_streaming => None,
            Some((TurnState::Finished, reply)) => {
                Some(ThreadMessageAnswer::Replied(reply.content.clone()))
            }
            Some((TurnState::Stopped, _)) => Some(ThreadMessageAnswer::Stopped),
            Some((TurnState::Failed { error }, _)) => {
                Some(ThreadMessageAnswer::Failed(error.clone()))
            }
            None => Some(ThreadMessageAnswer::Failed(
                "its turn ended without a reply".to_string(),
            )),
        }
    }

    fn schedule_drain(&mut self, cx: &mut Context<'_, Self>) {
        if self.is_streaming || self.queue_held || self.placing {
            return;
        }
        let Some(thread_id) = self.thread_id.as_deref() else {
            return;
        };
        if !self.prompt_queue.has_for(thread_id) {
            return;
        }
        cx.spawn(async move |entity, cx| {
            let _ = entity.update(cx, |view, cx| view.drain_queue(cx));
        })
        .detach();
    }

    pub(super) fn begin_queue_edit_latest(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(thread_id) = self.thread_id.clone() else {
            return;
        };
        let Some(item) = self.prompt_queue.latest_for(&thread_id) else {
            return;
        };
        self.begin_queue_edit(item, window, cx);
    }

    pub(super) fn begin_queue_edit(
        &mut self,
        item: QueuedPrompt,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .queue_edit
            .as_ref()
            .is_some_and(|edit| edit.id == item.id)
        {
            return;
        }
        if let Some(edit) = self.queue_edit.clone() {
            let prior_chips = self.queue_prior_chips.clone();
            self.install_chips(
                &edit.prior_draft,
                prior_chips,
                edit.prior_draft.len(),
                window,
                cx,
            );
        }
        let prior_draft = self.input_state.read(cx).value().to_string();
        let prior_chips = self.context_chips.clone();
        self.queue_edit = Some(QueueEdit {
            id: item.id,
            prior_draft,
        });
        self.queue_prior_chips = prior_chips;
        let chips = item.chips.clone();
        let len = item.text.len();
        self.install_chips(&item.text, chips, len, window, cx);
        cx.notify();
    }

    pub(super) fn commit_queue_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(edit) = self.queue_edit.clone() else {
            return;
        };
        let text = self.input_state.read(cx).value().to_string();
        let chips = self.context_chips.clone();
        if !self.prompt_queue.set_body(&edit.id, &text, chips) {
            return;
        }
        self.queue_edit = None;
        let prior_chips = std::mem::take(&mut self.queue_prior_chips);
        let prior = edit.prior_draft;
        self.install_chips(&prior, prior_chips, prior.len(), window, cx);
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();
    }

    pub(super) fn cancel_queue_edit(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(edit) = self.queue_edit.take() else {
            return;
        };
        let prior_chips = std::mem::take(&mut self.queue_prior_chips);
        let prior = edit.prior_draft;
        self.install_chips(&prior, prior_chips, prior.len(), window, cx);
        cx.notify();
    }

    /// `removed` is a row that just left the queue. `None` means every row is gone.
    pub(super) fn drop_queue_edit_if_removed(&mut self, removed: Option<&str>) {
        let Some(edit) = self.queue_edit.clone() else {
            return;
        };
        let gone = match removed {
            None => true,
            Some(id) => edit.id == id || !self.prompt_queue.contains(&edit.id),
        };
        if !gone {
            return;
        }
        if !edit.prior_draft.is_empty() {
            self.pending_composer = Some(edit.prior_draft);
            self.pending_chips = Some(self.queue_prior_chips.clone());
        }
        self.queue_edit = None;
    }

    pub(super) fn write_composer(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let cursor = text.len();
        self.input_state.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx);
            input.set_selected_range(cursor..cursor, cx);
        });
        self.composer_hints_suppressed_value = None;
        self.refresh_mention_hints(cx);
    }

    /// Lists, quotes, and code blocks. `true` means the draft changed.
    pub(super) fn apply_structure_key(
        &mut self,
        key: ComposerKey,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.queue_edit.is_some()
            || self.model_picker_open
            || self.composer_hint_popover_open(cx)
            || !self.input_state.focus_handle(cx).is_focused(window)
        {
            return false;
        }
        let (current, start, end) = {
            let input = self.input_state.read(cx);
            let range = input.selected_range();
            (input.value().to_string(), range.start, range.end)
        };
        let ComposerEdit::Apply {
            text,
            start,
            end,
            caret,
        } = composer_edit(&current, start, end, key)
        else {
            return false;
        };
        let next = splice_text(&current, start, end, &text);
        self.input_state.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            input.set_selected_range(caret..caret, cx);
        });
        self.recall = None;
        self.composer_hints_suppressed_value = None;
        self.refresh_mention_hints(cx);
        cx.notify();
        true
    }

    /// `true` consumes the paste. A normal insert returns false so the input
    /// pastes once. Cmd/Ctrl+Shift+V inserts here.
    pub(super) fn handle_composer_paste(
        &mut self,
        force_inline: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.input_state.focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let mut attached = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::ExternalPaths(paths) => {
                    self.enqueue_paths(paths.paths(), cx);
                    attached = true;
                }
                ClipboardEntry::Image(image) => {
                    self.enqueue_named_bytes(
                        pasted_image_name(image.format).to_string(),
                        image.bytes.clone(),
                        cx,
                    );
                    attached = true;
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if attached {
            return true;
        }
        let Some(text) = item.text() else {
            return false;
        };
        let draft = self.input_state.read(cx).value().to_string();
        match paste_outcome(&draft, &text, force_inline) {
            PasteOutcome::Attach => {
                let name = paste_name(self.text_attachments.iter().map(|item| item.name.as_str()));
                self.text_attachments.push(TextAttachment { name, text });
                cx.notify();
                true
            }
            PasteOutcome::Insert if force_inline => {
                self.insert_composer_text(&text, window, cx);
                true
            }
            PasteOutcome::Insert => false,
        }
    }

    fn insert_composer_text(
        &mut self,
        insert: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let (current, range_start, range_end) = {
            let input = self.input_state.read(cx);
            let range = input.selected_range();
            (input.value().to_string(), range.start, range.end)
        };
        let start = floor_char_boundary(&current, range_start);
        let next = splice_text(&current, start, range_end, insert);
        let caret = start + insert.len();
        self.input_state.update(cx, |input, cx| {
            input.set_value(next, window, cx);
            input.set_selected_range(caret..caret, cx);
        });
        self.recall = None;
        self.composer_hints_suppressed_value = None;
        self.refresh_mention_hints(cx);
        cx.notify();
    }

    /// ArrowUp / ArrowDown through prompts already sent in this thread.
    /// `true` means the key replaces or clears the composer.
    pub(super) fn recall_history(
        &mut self,
        older: bool,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.queue_edit.is_some() || !self.input_state.focus_handle(cx).is_focused(window) {
            return false;
        }
        let (value, range) = {
            let input = self.input_state.read(cx);
            (input.value().to_string(), input.selected_range())
        };
        let Some((on_first, on_last)) = logical_line_edges(&value, range.start, range.end) else {
            return false;
        };
        let prompts = sent_prompts(self.messages.iter().enumerate().map(|(index, message)| {
            (
                index.to_string(),
                matches!(message.role, MessageRole::User),
                message.content.as_str(),
            )
        }));
        let outcome = recall_step(
            &prompts,
            self.recall.as_ref(),
            &value,
            !self.text_attachments.is_empty() || !self.file_offers.is_empty(),
            if older {
                RecallDirection::Older
            } else {
                RecallDirection::Newer
            },
            if older { on_first } else { on_last },
        );
        let claimed = match &outcome.action {
            RecallAction::Place(text) => {
                // The same text means the walk is already at the oldest
                // prompt. Consume the key and leave the caret where it is.
                if text != &value {
                    self.write_composer(text, window, cx);
                }
                true
            }
            RecallAction::Clear => {
                self.write_composer("", window, cx);
                true
            }
            RecallAction::Pass => false,
        };
        self.recall = outcome.cursor;
        claimed
    }

    /// Open the Edit from here confirm on a user message.
    pub(super) fn request_rewind_confirm(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        if self.is_streaming {
            return;
        }
        let is_user = self
            .messages
            .get(index)
            .is_some_and(|message| matches!(message.role, MessageRole::User));
        if !is_user {
            return;
        }
        self.rewind_confirm = Some(index);
        cx.notify();
    }

    pub(super) fn cancel_rewind_confirm(&mut self, cx: &mut Context<'_, Self>) {
        self.rewind_confirm = None;
        cx.notify();
    }

    /// Rewind to before `index`. `draft` is the unsent composer text that
    /// stays above the restored prompt. A queued edit passes its prior draft.
    pub(crate) fn confirm_rewind(&mut self, index: usize, cx: &mut Context<'_, Self>) {
        let draft = self.queue_edit.as_ref().map_or_else(
            || self.input_state.read(cx).value().to_string(),
            |edit| edit.prior_draft.clone(),
        );
        if self.rewind_at(index, &draft) {
            cx.emit(ChatViewEvent::MessagesChanged);
            cx.notify();
        }
    }

    /// Drop the user message at `index` and everything after it. The next
    /// turn starts a new provider session. Files stay as they are.
    pub(super) fn rewind_at(&mut self, index: usize, draft: &str) -> bool {
        if self.is_streaming {
            return false;
        }
        let Some(message) = self.messages.get(index) else {
            return false;
        };
        if !matches!(message.role, MessageRole::User) {
            return false;
        }
        let prompt = message.content.clone();
        self.messages.truncate(index);
        self.forget_message_expansions();
        self.recall = None;
        self.rewind_confirm = None;
        self.queue_edit = None;
        self.fresh_session = true;
        self.session_id = None;
        self.session_provider = None;
        self.pending_composer = Some(compose_after_rewind(draft, &prompt));
        self.focus_pending_composer = true;
        true
    }

    pub fn restore_prompt_queue(&mut self, items: Vec<QueuedPrompt>) {
        self.prompt_queue = PromptQueue::from_items(items);
        self.queue_held = !self.prompt_queue.is_empty();
        self.queue_edit = None;
        self.pending_composer = None;
    }

    pub fn submit_prompt(&mut self, content: String, cx: &mut Context<'_, Self>) {
        self.finish_submit(content, Vec::new(), Vec::new(), Vec::new(), cx);
    }

    /// `attachments` are written beside this turn. A queued drain passes its
    /// own chips and leaves paste chips and file offers in the composer.
    fn finish_submit(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.is_streaming || self.placing {
            return;
        }
        // Idle, including a drained queue. A local command updates the goal
        // and does not start a turn, so a newer draft stays in the composer.
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        let prepared = prepare_command(content.trim(), provider, self.goal.as_ref(), false);
        self.store_goal(prepared.write, cx);
        if let Some(notice) = prepared.notice {
            cx.emit(ChatViewEvent::Notice(notice));
        }
        if !prepared.send {
            return;
        }
        self.place_then_send(content, attachments, chips, uploads, cx);
    }

    /// Claude `--effort` for this send. Codex, and Default, omit the flag.
    fn claude_send_effort(&self) -> Option<String> {
        if !self.effort_chip_visible() {
            return None;
        }
        // The thread's role sets the effort when it names one.
        self.role_profile
            .as_ref()
            .and_then(|role| role.effort.as_deref())
            .or(self.claude_effort.as_deref())
            .and_then(bruhs_models::logic::run_place::claude_effort)
            .map(str::to_string)
    }

    /// Lock the checkout, reuse a worktree, or create one before the agent runs.
    fn place_then_send(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        cx: &mut Context<'_, Self>,
    ) {
        let thread_id = self
            .thread_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let project = self.project_scoped.then_some(self.project_path.as_str());
        let decision = first_send(
            self.run_locked,
            self.run_choice,
            project,
            &self.base_branch,
            self.git_current_branch.as_deref(),
            self.previous_worktree.as_ref(),
            &thread_id,
        );
        match decision {
            Err(message) => {
                self.fail_place(content, attachments, chips, uploads, message, cx);
            }
            Ok(FirstSend::Stay) => {
                self.lock_place(None, None, cx);
                self.launch_turn(content, attachments, chips, uploads, cx);
            }
            Ok(FirstSend::UsePath { path, branch }) => {
                self.lock_place(Some(path), Some(branch), cx);
                self.launch_turn(content, attachments, chips, uploads, cx);
            }
            Ok(FirstSend::Create { base, branch }) => {
                self.begin_worktree(content, attachments, chips, uploads, base, branch, cx);
            }
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the draft and the worktree base and branch stay separate until create finishes"
    )]
    fn begin_worktree(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        base: String,
        branch: String,
        cx: &mut Context<'_, Self>,
    ) {
        let repo = std::path::PathBuf::from(&self.project_path);
        let dest = match bruhs_core::git::worktree_dest(&repo, &branch) {
            Ok(dest) => dest,
            Err(error) => {
                self.fail_place(content, attachments, chips, uploads, &error.to_string(), cx);
                return;
            }
        };
        self.placing = true;
        let create = spawn_tokio(cx, {
            let branch = branch.clone();
            async move {
                bruhs_core::git::add_worktree(&repo, &base, &dest, &branch)
                    .await
                    .map(|()| dest)
            }
        });
        let project_label = self.project_name.clone();
        self.place_task = Some(cx.spawn(async move |this, cx| {
            let created = create.await;
            let _closed = this.update(cx, |this, cx| {
                this.placing = false;
                match created {
                    Ok(dest) => {
                        let path = dest.to_string_lossy().into_owned();
                        let remembered = RememberedWorktree {
                            path: path.clone(),
                            branch: branch.clone(),
                            label: project_label,
                        };
                        this.previous_worktree = Some(remembered);
                        this.emit_run_prefs(cx);
                        this.lock_place(Some(path), Some(branch), cx);
                        this.launch_turn(content, attachments, chips, uploads, cx);
                    }
                    Err(error) => {
                        this.fail_place(
                            content,
                            attachments,
                            chips,
                            uploads,
                            &error.to_string(),
                            cx,
                        );
                    }
                }
            });
        }));
        cx.notify();
    }

    /// Remember that this thread has a place. A worktree also becomes the cwd.
    fn lock_place(
        &mut self,
        worktree: Option<String>,
        branch: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.run_locked = true;
        if let Some(path) = worktree {
            self.worktree_path = Some(path.clone());
            self.project_path = path.clone();
            self.project_scoped = true;
            if let Some(branch) = branch.clone() {
                self.git_current_branch = Some(branch);
            }
            cx.emit(ChatViewEvent::ThreadPlaced {
                project_path: path,
                branch,
            });
            self.refresh_git_branches(cx);
        }
    }

    /// The draft is consumed and the assistant shows why no turn started.
    fn fail_place(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        error: &str,
        cx: &mut Context<'_, Self>,
    ) {
        let expanded = expand_prompt(&content, &chips);
        let visible = super::files::visible_with_files(&expanded, &attachments, &uploads);
        let title_prompt = if content.trim().is_empty() {
            visible.as_str()
        } else {
            content.trim()
        };
        self.thread_title = thread_title_after_submit_like_web(&self.thread_title, title_prompt);
        let model_id = self.model_id.clone();
        self.messages
            .push(Message::user_with_files(visible, uploads));
        self.messages
            .push(Message::assistant_failed(Some(model_id.clone()), error));
        self.scroll_messages_to_bottom();
        cx.emit(ChatViewEvent::TurnStarted {
            model_id,
            branch: self.git_current_branch.clone(),
        });
        cx.notify();
    }

    /// Hand a placed send to the provider. The caller has already locked the place.
    ///
    /// A bruhs worktree is snapshotted first. A failed snapshot still sends,
    /// with no checkpoint, so chatting is not blocked. The project checkout
    /// is never snapshotted.
    fn launch_turn(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        cx: &mut Context<'_, Self>,
    ) {
        // Core refuses a send while the thread's previous turn is still
        // attached. A stopped turn keeps its aborted handle, so the new turn
        // waits for it to unwind (and release the thread) before sending.
        let previous_turn = self.agent_task.take();
        if let Some(task) = &previous_turn {
            task.abort();
        }
        let Some(dir) = self.checkpoint_dir() else {
            self.finish_launch(
                content,
                attachments,
                chips,
                uploads,
                None,
                previous_turn,
                cx,
            );
            return;
        };
        self.placing = true;
        let snap = spawn_tokio(cx, async move {
            bruhs_core::git::snapshot_worktree(&dir).await.ok()
        });
        cx.spawn(async move |this, cx| {
            let tree = snap.await;
            let _ = this.update(cx, |this, cx| {
                this.placing = false;
                this.finish_launch(
                    content,
                    attachments,
                    chips,
                    uploads,
                    tree,
                    previous_turn,
                    cx,
                );
            });
        })
        .detach();
        cx.notify();
    }

    /// Worktree path that may be snapshotted. A checkout, or a path outside
    /// the bruhs worktrees root, sends with no checkpoint.
    fn checkpoint_dir(&self) -> Option<std::path::PathBuf> {
        let path = self
            .worktree_path
            .as_deref()
            .filter(|path| !path.trim().is_empty())?;
        bruhs_core::git::ensure_bruhs_worktree(std::path::Path::new(path)).ok()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the draft, its checkpoint, and the previous turn stay separate until the provider starts"
    )]
    fn finish_launch(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        checkpoint: Option<String>,
        previous_turn: Option<JoinHandle<()>>,
        cx: &mut Context<'_, Self>,
    ) {
        let turn = self.begin_turn(content, attachments, chips, uploads, checkpoint, cx);
        // Cleared only once a turn is actually handed to the provider. A test
        // that drops the turn keeps the flag so the next real send stays fresh.
        if turn.as_ref().is_ok_and(|turn| turn.fresh_session) {
            self.fresh_session = false;
        }
        let retry_prompt = turn
            .as_ref()
            .ok()
            .and_then(|turn| self.cold_retry_prompt(turn));
        let desktop_state = cx.global::<DesktopState>();
        let app = Arc::clone(&desktop_state.app);
        let runtime = Arc::clone(&desktop_state.runtime);
        let (tx, rx) = unbounded_channel();
        let agent_task = runtime.spawn(stream_agent_response(
            app,
            previous_turn,
            turn,
            retry_prompt,
            tx,
        ));
        self.attach_turn(agent_task, rx, cx);
        cx.notify();
    }

    /// The UI half of a submit: the user message, the streaming placeholder,
    /// the spinner. Returns the turn to send.
    pub(super) fn begin_turn(
        &mut self,
        content: String,
        attachments: Vec<TextAttachment>,
        chips: Vec<ContextChip>,
        uploads: Vec<UploadedFile>,
        checkpoint: Option<String>,
        cx: &mut Context<'_, Self>,
    ) -> Result<AgentTurn, IdError> {
        let expanded = expand_prompt(&content, &chips);
        let visible = super::files::visible_with_files(&expanded, &attachments, &uploads);
        let title_prompt = if content.trim().is_empty() {
            visible.as_str()
        } else {
            content.trim()
        };
        let thread_id = self
            .thread_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        self.thread_title = thread_title_after_submit_like_web(&self.thread_title, title_prompt);
        let project_path = self.project_path.clone();
        let project_scoped = self.project_scoped;
        // The thread's role can narrow the access mode, never widen it.
        let permission_mode = bruhs_models::roles::narrow_access(
            self.permission_mode,
            self.role_profile.as_ref().and_then(|role| role.access),
        );
        let instructions = self
            .role_profile
            .as_ref()
            .map(|role| role.instructions.clone())
            .filter(|text| !text.trim().is_empty());
        let model_id = self.model_id.clone();
        let paste_files = bruhs_core::write_paste_files(&attachments);
        let provider = provider_for_model_id(&model_id);
        // A handoff turn asks for a brief; when it ends, a fresh thread opens
        // with the brief as its draft (web: `handoff_place`).
        let prompt = match classify_command(content.trim(), provider) {
            CommandKind::Handoff(next_goal) => {
                self.handoff_pending = true;
                bruhs_models::logic::handoff::handoff_prompt(&next_goal)
            }
            _ => {
                self.handoff_pending = false;
                super::files::turn_prompt(&visible, &paste_files, &uploads, self.goal.as_ref())
            }
        };
        // An imported transcript rides along until a session holds it. So
        // does the conversation so far when the saved session belongs to the
        // other provider (the model changed), which cannot resume it.
        let other_provider = !self.fresh_session
            && self.session_id.is_some()
            && self.session_provider != Some(provider);
        let carried = if self.handoff_pending {
            None
        } else if other_provider {
            self.conversation_context(&self.messages)
        } else {
            self.import_context.clone()
        };
        let prompt = prompt_with_import_context(carried.as_deref(), prompt);
        self.turn_provider = provider;

        let mut user = Message::user_with_files(visible, uploads);
        user.checkpoint = checkpoint.filter(|tree| !tree.is_empty());
        self.messages.push(user);
        self.compact_full = false;
        self.is_streaming = true;
        self.start_streaming_spinner(cx);
        self.stream_started_at = Some(Instant::now());
        self.pending_permission = None;
        cx.emit(ChatViewEvent::TurnStarted {
            model_id: model_id.clone(),
            branch: self.git_current_branch.clone(),
        });

        // Add streaming assistant placeholder
        self.messages.push(Message::assistant_streaming(
            EpochMs::now().as_millis(),
            Some(model_id.clone()),
        ));
        self.scroll_messages_to_bottom();

        let send_effort = self.claude_send_effort();
        self.effort = send_effort.clone();
        let autocompact = self
            .effort_chip_visible()
            .then_some(self.claude_autocompact.as_deref())
            .flatten();
        ThreadId::try_from(thread_id).map(|thread_id| {
            let turn = AgentTurn::new(
                thread_id,
                prompt,
                project_path,
                model_id.as_str(),
                provider_for_model_id(&model_id),
                permission_mode,
            )
            .effort(send_effort.as_deref())
            .autocompact(autocompact)
            .thread_access(self.thread_access)
            .instructions(instructions)
            // A chat with no project needs no coding plugins (web: no `project_path`).
            .lean(!project_scoped);
            if self.fresh_session {
                turn.fresh()
            } else {
                turn.resume(session_for_provider(
                    self.session_id.as_deref(),
                    self.session_provider,
                    provider,
                ))
            }
        })
    }

    /// Route `rx` (the events of the turn `agent_task` is sending) into this
    /// view. Only the newest turn's events are applied: the previous
    /// receiver is dropped (cancelled) here, and each event is checked
    /// against the generation it was received for.
    pub(super) fn attach_turn(
        &mut self,
        agent_task: JoinHandle<()>,
        mut rx: UnboundedReceiver<AgentUiEvent>,
        cx: &mut Context<'_, Self>,
    ) {
        self.turn_generation = self.turn_generation.wrapping_add(1);
        let generation = self.turn_generation;
        self.agent_task = Some(agent_task);
        self.stream_task = Some(cx.spawn(async move |entity, cx| {
            while let Some(event) = rx.recv().await {
                let done = event.is_terminal();
                let applied = entity.update(cx, |view, cx| {
                    if view.turn_generation != generation {
                        return false;
                    }
                    let event = match event {
                        AgentUiEvent::ThreadRequest(request) => {
                            if view.thread_id.as_deref() == Some(request.thread_id.as_str()) {
                                cx.emit(ChatViewEvent::ThreadRequest(request));
                            }
                            return true;
                        }
                        event => event,
                    };
                    let saved = (view.session_id.clone(), view.import_context.is_some());
                    view.apply_agent_event(event);
                    if saved != (view.session_id.clone(), view.import_context.is_some()) {
                        // Saved now, so a crash mid-turn keeps the session.
                        cx.emit(ChatViewEvent::MessagesChanged);
                    }
                    if done {
                        cx.emit(ChatViewEvent::TurnFinished);
                        if let Some(brief) = view.take_handoff_brief() {
                            cx.emit(ChatViewEvent::Handoff { brief });
                        }
                        view.schedule_drain(cx);
                    }
                    cx.notify();
                    true
                });
                if done || !matches!(applied, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Stop listening to the current turn and abort its sender. The aborted
    /// handle stays in `agent_task` for the next submit to wait on.
    pub(super) fn abandon_turn(&mut self) {
        self.pending_secret = None;
        self.secret_prompt = None;
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.stream_task = None;
        if let Some(task) = &self.agent_task {
            task.abort();
        }
    }

    #[cfg(test)]
    pub(super) fn start_turn_for_test(
        &mut self,
        content: &str,
        agent_task: JoinHandle<()>,
        rx: UnboundedReceiver<AgentUiEvent>,
        cx: &mut Context<'_, Self>,
    ) {
        let previous_turn = self.agent_task.take();
        if let Some(task) = previous_turn {
            task.abort();
        }
        // The test drives the stream itself; the turn is never sent.
        drop(self.begin_turn(
            content.to_string(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
            cx,
        ));
        self.attach_turn(agent_task, rx, cx);
        cx.notify();
    }

    /// Keep the goal only when it changes, so a no-op command does not save.
    pub(super) fn store_goal(&mut self, write: GoalWrite, cx: &mut Context<'_, Self>) {
        let next = match write {
            GoalWrite::Keep => return,
            GoalWrite::Set(goal) => Some(goal),
            GoalWrite::Remove => None,
        };
        if self.goal == next {
            return;
        }
        self.goal = next;
        cx.emit(ChatViewEvent::MessagesChanged);
        cx.notify();
    }

    /// `/` menu picks pass the bare name. The editor clears that token.
    /// A Claude `/compact` sends, or queues while a turn is running, and
    /// leaves the draft in the composer.
    pub(super) fn apply_local_command(&mut self, command: &str, cx: &mut Context<'_, Self>) {
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        if let CommandKind::SecondOpinion(focus) = classify_command(command, provider) {
            if self.is_streaming {
                cx.emit(ChatViewEvent::Notice(
                    bruhs_models::logic::handoff::SECOND_OPINION_WAIT.to_string(),
                ));
            } else {
                self.run_second_opinion(focus, cx);
            }
            return;
        }
        let prepared = prepare_command(command, provider, self.goal.as_ref(), self.is_streaming);
        self.store_goal(prepared.write, cx);
        if let Some(notice) = prepared.notice {
            cx.emit(ChatViewEvent::Notice(notice));
        }
        if prepared.stop {
            self.stop_streaming(cx);
        }
        if prepared.queue {
            if let Some(thread_id) = self.thread_id.clone() {
                let at_ms = EpochMs::now().as_millis().to_f64_lossy();
                self.prompt_queue.push(&thread_id, command, at_ms);
                cx.emit(ChatViewEvent::MessagesChanged);
                cx.notify();
            }
            return;
        }
        if prepared.send {
            self.finish_submit(command.to_string(), Vec::new(), Vec::new(), Vec::new(), cx);
        }
    }

    pub(super) fn stop_streaming(&mut self, cx: &mut Context<'_, Self>) {
        if !self.is_streaming {
            return;
        }

        // Hold before the stream is marked idle, so its Done cannot drain.
        self.queue_held = true;
        // A stopped handoff has a cut-off brief: open nothing.
        self.handoff_pending = false;
        self.abandon_turn();
        if let Some(thread_id) = self.thread_id.as_deref() {
            cx.global::<DesktopState>()
                .app
                .registry()
                .mark_stream_killed(thread_id);
        }
        if let Some(message) = self.active_assistant_message_mut() {
            mark_streaming_message_stopped_like_web(message);
        }
        self.is_streaming = false;
        self.spinner_task.take();
        self.stream_started_at = None;
        self.pending_permission = None;
        self.scroll_messages_to_bottom();
        smoke_log("done", "stopped");
        cx.emit(ChatViewEvent::TurnFinished);
        cx.notify();
    }

    pub(super) fn active_assistant_message_mut(&mut self) -> Option<&mut Message> {
        self.messages
            .iter_mut()
            .rev()
            .find(|message| matches!(message.role, MessageRole::Assistant))
    }

    pub(super) fn apply_agent_event(&mut self, event: AgentUiEvent) {
        match event {
            AgentUiEvent::SecretRequest(request) => {
                if self.thread_id.as_deref() == Some(request.thread_id.as_str()) {
                    self.pending_secret = Some(request);
                }
            }
            AgentUiEvent::SecretResolved(id) => {
                if self
                    .pending_secret
                    .as_ref()
                    .is_some_and(|request| request.id == id)
                {
                    self.pending_secret = None;
                    self.secret_prompt = None;
                }
            }
            // Answered by the workspace before events reach this reducer.
            AgentUiEvent::ThreadRequest(_) => {}
            AgentUiEvent::Text(text) => {
                if let Some(message) = self.active_assistant_message_mut() {
                    message.content.push_str(&text);
                }
                smoke_log("text", &text);
            }
            AgentUiEvent::Thinking(text) => {
                if let Some(message) = self.active_assistant_message_mut() {
                    message.thinking.push_str(&text);
                }
            }
            AgentUiEvent::ToolCall { id, tool, input } => {
                let summary = desktop_tool_input_summary(&tool, &input);
                smoke_log("tool_call", &format!("{tool} {summary}"));
                if let Some(message) = self.active_assistant_message_mut() {
                    message.tool_calls.push(DesktopToolCall {
                        id,
                        tool,
                        input,
                        output: None,
                        is_error: false,
                    });
                }
            }
            AgentUiEvent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                smoke_log(
                    if is_error {
                        "tool_error"
                    } else {
                        "tool_result"
                    },
                    &tool_use_id,
                );
                if let Some(message) = self.active_assistant_message_mut() {
                    if let Some(tool_call) = message
                        .tool_calls
                        .iter_mut()
                        .find(|tool_call| tool_call.id == tool_use_id)
                    {
                        tool_call.output = Some(content);
                        tool_call.is_error = is_error;
                    } else {
                        message.tool_calls.push(DesktopToolCall {
                            id: tool_use_id,
                            tool: "Tool result".to_string(),
                            input: serde_json::Value::Null,
                            output: Some(content),
                            is_error,
                        });
                    }
                }
            }
            AgentUiEvent::Usage(usage) => {
                if let Some(message) = self.active_assistant_message_mut() {
                    message.usage = Some(keep_window(usage, message.usage.as_ref()));
                }
            }
            AgentUiEvent::Result { duration_ms, usage } => {
                if let Some(message) = self.active_assistant_message_mut() {
                    message.duration_ms = Some(duration_ms);
                    message.usage = Some(keep_window(usage, message.usage.as_ref()));
                    message.completed_at_ms = message
                        .started_at_ms
                        .map(|started_at_ms| {
                            started_at_ms
                                .saturating_add(i64::try_from(duration_ms).unwrap_or(i64::MAX))
                        })
                        .or_else(|| Some(EpochMs::now().as_millis()));
                }
            }
            AgentUiEvent::PermissionRequest(request) => {
                smoke_log("permission_request", &request.tool);
                self.pending_permission = Some(request);
            }
            AgentUiEvent::PermissionCancelled(id) => {
                if clear_permission_if_matches(&mut self.pending_permission, &id) {
                    smoke_log("permission_cancelled", &id);
                }
            }
            AgentUiEvent::Session(id) => self.record_session(id),
            AgentUiEvent::SessionRejected => {
                self.session_id = None;
                self.session_provider = None;
            }
            AgentUiEvent::Error(message) => {
                self.pending_secret = None;
                self.secret_prompt = None;
                self.agent_task = None;
                if let Some(active_message) = self.active_assistant_message_mut() {
                    active_message.completed_at_ms = Some(EpochMs::now().as_millis());
                    active_message.content = format!("Agent error: {message}");
                    active_message.state = TurnState::Failed {
                        error: message.clone(),
                    };
                }
                self.is_streaming = false;
                self.stream_started_at = None;
                self.pending_permission = None;
                smoke_log("error", &message);
            }
            AgentUiEvent::Done => {
                self.pending_secret = None;
                self.secret_prompt = None;
                self.agent_task = None;
                if let Some(message) = self.active_assistant_message_mut() {
                    if message.state.is_streaming() {
                        message.state = TurnState::Finished;
                        if message.content.is_empty() {
                            message.content = "Agent completed without text output.".to_string();
                        }
                        if message.completed_at_ms.is_none() {
                            message.completed_at_ms = Some(EpochMs::now().as_millis());
                        }
                    }
                }
                self.is_streaming = false;
                self.stream_started_at = None;
                self.pending_permission = None;
                smoke_log("done", "");
            }
        }
        self.scroll_messages_to_bottom();
    }

    pub(super) fn respond_to_permission(&mut self, approved: bool, cx: &mut Context<'_, Self>) {
        let Some(request) = self.pending_permission.take() else {
            cx.notify();
            return;
        };
        smoke_log(
            if approved {
                "permission_approved"
            } else {
                "permission_denied"
            },
            &request.id,
        );
        // Same path as the web's `/api/agent/approve`: core resolves the
        // stdio `can_use_tool` request and the CLI continues the turn.
        if let Some(desktop_state) = cx.try_global::<DesktopState>() {
            let app = Arc::clone(&desktop_state.app);
            let runtime = Arc::clone(&desktop_state.runtime);
            let request_id = request.id;
            let delivery = runtime.spawn(async move {
                let decision = if approved {
                    PermissionDecision::Allow
                } else {
                    PermissionDecision::Deny {
                        reason: Some(PERMISSION_DENIED_REASON.to_string()),
                    }
                };
                app.resolve_permission(&request_id, decision)
                    .await
                    .map_err(|err| format!("permission {request_id} not delivered: {err}"))
            });
            cx.spawn(async move |this, cx| {
                let failure = match delivery.await {
                    Ok(Ok(())) => return,
                    Ok(Err(message)) => message,
                    Err(join) => format!("permission reply did not finish: {join}"),
                };
                tracing::warn!("{failure}");
                // The chat may have closed; then nobody is waiting on it.
                let _closed = this.update(cx, |_, cx| {
                    cx.emit(ChatViewEvent::Notice(format!(
                        "The agent did not get your answer: {failure}"
                    )));
                });
            })
            .detach();
        }
        cx.notify();
    }
}

impl ChatView {
    /// Put `text` in the composer, focused, on the next render (which has
    /// the window the editor needs).
    pub(crate) fn seed_composer(&mut self, text: String) {
        self.pending_composer = Some(text);
        self.focus_pending_composer = true;
    }

    /// The finished `/handoff` reply, once. A failed or empty reply opens
    /// nothing.
    fn take_handoff_brief(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.handoff_pending) {
            return None;
        }
        let reply = self.messages.last()?;
        (matches!(reply.role, MessageRole::Assistant)
            && reply.state == TurnState::Finished
            && !reply.content.trim().is_empty())
        .then(|| reply.content.clone())
    }

    /// `/second-opinion`: the other provider reviews the last answer
    /// read-only; its review becomes the composer draft (web:
    /// `run_second_opinion`).
    pub(super) fn run_second_opinion(&mut self, focus: String, cx: &mut Context<'_, Self>) {
        use bruhs_models::logic::handoff::{
            provider_name, review_holds_up, reviewer_for, second_opinion_agrees,
            second_opinion_followup, SECOND_OPINION_NOTHING,
        };
        let Some((task, answer)) = last_exchange(&self.messages) else {
            cx.emit(ChatViewEvent::Notice(SECOND_OPINION_NOTHING.to_string()));
            return;
        };
        let Some(runtime) = cx
            .try_global::<DesktopState>()
            .map(|state| Arc::clone(&state.runtime))
        else {
            return;
        };
        let provider = provider_for_model_id(&self.model_id);
        let reviewer = reviewer_for(provider);
        cx.emit(ChatViewEvent::Notice(format!(
            "Asking {} for a second opinion…",
            provider_name(reviewer)
        )));
        let request = bruhs_core::second_opinion::ReviewRequest {
            thread_provider: provider,
            project_path: self.project_path.clone(),
            task,
            answer,
            focus,
            // The user's model for this reviewer, if they chose one.
            reviewer_model: self.task_models.review(reviewer).map(str::to_string),
        };
        let review = runtime.spawn(bruhs_core::second_opinion::second_opinion(request));
        cx.spawn(async move |this, cx| {
            let outcome = match review.await {
                // Nothing to fix: a follow-up turn would cost tokens for nothing.
                Ok(Ok(review)) if review_holds_up(&review.text) => {
                    Err(second_opinion_agrees(review.reviewer))
                }
                Ok(Ok(review)) => Ok(second_opinion_followup(review.reviewer, &review.text)),
                Ok(Err(error)) => Err(error.to_string()),
                Err(join) => Err(format!("The review did not finish: {join}")),
            };
            let _closed = this.update(cx, |view, cx| {
                match outcome {
                    // Never over a draft typed while the review ran.
                    Ok(draft) if view.input_state.read(cx).value().trim().is_empty() => {
                        view.seed_composer(draft);
                    }
                    Ok(_) => cx.emit(ChatViewEvent::Notice(
                        "The second opinion came back while the composer was in use; run it again."
                            .to_string(),
                    )),
                    Err(message) => cx.emit(ChatViewEvent::Notice(message)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// The last assistant answer with text, and the user message before it.
fn last_exchange(messages: &[Message]) -> Option<(String, String)> {
    let at = messages.iter().rposition(|message| {
        matches!(message.role, MessageRole::Assistant) && !message.content.trim().is_empty()
    })?;
    let task = messages[..at]
        .iter()
        .rev()
        .find(|message| matches!(message.role, MessageRole::User))
        .map(|message| message.content.clone())
        .unwrap_or_default();
    Some((task, messages[at].content.clone()))
}

/// `usage` as the message's usage. A turn sum carries no window size, so the
/// window last reported (`prior`) is kept.
fn keep_window(mut usage: UsageInfo, prior: Option<&UsageInfo>) -> UsageInfo {
    usage.context_tokens = usage
        .context_tokens
        .or_else(|| prior.and_then(|prior| prior.context_tokens));
    usage
}

#[cfg(test)]
mod window_tests {
    use super::{keep_window, last_exchange, Message, UsageInfo};

    /// Web: `second_opinion_reviews_the_last_answer_with_text_and_its_question`.
    #[test]
    fn second_opinion_reviews_the_last_answer_with_text_and_its_question() {
        let answer = |text: &str| {
            let mut message = Message::assistant_streaming(1, None);
            message.content = text.to_string();
            message
        };
        assert_eq!(last_exchange(&[]), None);
        let rows = vec![
            Message::user("first"),
            answer("old answer"),
            Message::user("is add right?"),
            answer("yes"),
            Message::assistant_streaming(3, None),
        ];
        assert_eq!(
            last_exchange(&rows),
            Some(("is add right?".to_string(), "yes".to_string()))
        );
    }

    /// Claude's `result.usage` sums the turn's requests; the window the last
    /// request reported must survive it (web: `a_turn_sum_keeps_the_last_reported_window`).
    #[test]
    fn a_turn_sum_keeps_the_last_reported_window() {
        let request = UsageInfo {
            cache_read_input_tokens: 47_261,
            context_tokens: Some(49_191),
            ..UsageInfo::default()
        };
        let total = UsageInfo {
            cache_read_input_tokens: 74_003,
            ..UsageInfo::default()
        };
        let kept = keep_window(total, Some(&request));
        assert_eq!(kept.cache_read_input_tokens, 74_003);
        assert_eq!(kept.context_tokens, Some(49_191));
        let fresh = keep_window(request, None);
        assert_eq!(fresh.context_tokens, Some(49_191));
    }
}
