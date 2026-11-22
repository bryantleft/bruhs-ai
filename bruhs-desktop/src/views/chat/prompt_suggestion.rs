use bruhs_models::logic::prompt_suggestion::{
    recent_context, PromptLine, SuggestionKey, SuggestionRequest,
};

use super::*;

struct AbortOnDrop(tokio::task::AbortHandle);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl ChatView {
    fn suggestion_blocked(&self, cx: &App) -> bool {
        !self.task_models.prompt_suggestions
            || self.task_models.suggestion(&self.model_id).is_empty()
            || self.is_streaming
            || self.placing
            || self.queue_edit.is_some()
            || !self.text_attachments.is_empty()
            || !self.file_offers.is_empty()
            || !self.context_chips.is_empty()
            || self.pending_composer.is_some()
            || self.composer_hint_popover_open(cx)
            || self
                .thread_id
                .as_deref()
                .is_some_and(|id| self.prompt_queue.has_for(id))
    }

    pub(super) fn hide_blocked_prompt_suggestion(&mut self, cx: &mut Context<'_, Self>) {
        if self.suggestion_blocked(cx) {
            self.input_state
                .update(cx, |input, cx| input.set_inline_completion_text(None, cx));
        }
    }

    pub(super) fn sync_prompt_suggestion(&mut self, cx: &mut Context<'_, Self>) {
        let suffix = if self.suggestion_blocked(cx) {
            None
        } else {
            let input = self.input_state.read(cx);
            self.prompt_suggestion
                .suffix(&input.value(), input.selected_range())
                .map(str::to_string)
        };
        self.input_state
            .update(cx, |input, cx| input.set_inline_completion_text(suffix, cx));
    }

    pub(super) fn dismiss_prompt_suggestion(&mut self, cx: &mut Context<'_, Self>) {
        self.prompt_suggestion.dismiss();
        self.suggestion_task = None;
        self.input_state
            .update(cx, |input, cx| input.set_inline_completion_text(None, cx));
    }

    pub(super) fn accept_prompt_suggestion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if self.suggestion_blocked(cx) || !self.input_state.focus_handle(cx).is_focused(window) {
            return false;
        }
        let input = self.input_state.read(cx);
        let Some(suffix) = self
            .prompt_suggestion
            .suffix(&input.value(), input.selected_range())
        else {
            return false;
        };
        if input.inline_completion_text() != Some(suffix) {
            return false;
        }
        let suffix = suffix.to_string();
        self.prompt_suggestion.dismiss();
        self.suggestion_task = None;
        self.input_state.update(cx, |input, cx| {
            // insert records an ordinary undoable edit and leaves the caret
            // at the end. Accepting never invokes the send action.
            input.insert(suffix, window, cx);
            input.set_inline_completion_text(None, cx);
        });
        cx.notify();
        true
    }

    pub(super) fn request_prompt_suggestion(&mut self, cx: &mut Context<'_, Self>) {
        if self.suggestion_blocked(cx) || !self.input_state.read(cx).value().is_empty() {
            return;
        }
        let Some(last) = self.messages.last() else {
            return;
        };
        if last.role != MessageRole::Assistant
            || last.state != TurnState::Finished
            || last.content.trim().is_empty()
        {
            return;
        }
        let Some(thread) = self.thread_id.clone() else {
            return;
        };
        let request = SuggestionRequest {
            model: self.task_models.suggestion(&self.model_id).to_string(),
            messages: recent_context(
                self.messages
                    .iter()
                    .filter(|message| {
                        message.role != MessageRole::Tool && message.state == TurnState::Finished
                    })
                    .map(|message| PromptLine {
                        role: message.role,
                        text: message.content.clone(),
                    }),
            ),
        };
        if !request.is_valid() {
            return;
        }
        let key = SuggestionKey {
            thread,
            turn: self.turn_generation.to_string(),
            model: request.model.clone(),
        };
        if !self.prompt_suggestion.begin(key.clone()) {
            return;
        }
        let project = std::path::PathBuf::from(&self.project_path);
        let task = cx
            .global::<DesktopState>()
            .runtime
            .spawn(async move { bruhs_core::prompt_suggestion::suggest_in_project(request, &project).await });
        let cancel = AbortOnDrop(task.abort_handle());
        self.suggestion_task = Some(cx.spawn(async move |this, cx| {
            let _cancel = cancel;
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(reply)) => this.prompt_suggestion.finish(&key, reply),
                    Ok(Err(error)) => tracing::debug!("Prompt suggestion unavailable: {error}"),
                    Err(error) if error.is_cancelled() => {}
                    Err(error) => tracing::warn!("Prompt suggestion worker failed: {error}"),
                }
                this.sync_prompt_suggestion(cx);
                cx.notify();
            });
        }));
    }
}
