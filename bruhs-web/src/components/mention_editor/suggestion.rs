use bruhs_models::{
    agent::Role,
    logic::prompt_suggestion::{
        recent_context, PromptLine, PromptSuggestion, SuggestionKey, SuggestionRequest,
    },
};
use leptos::{html, prelude::*, task::spawn_local};

use crate::state::{ActiveThreadId, CurrentModel, IsStreaming, MessagesSignal, TaskModelsSignal};

#[derive(Clone, Copy)]
pub(super) struct Suggestion {
    pub selection: RwSignal<std::ops::Range<usize>>,
    pub focused: RwSignal<bool>,
    pub composing: RwSignal<bool>,
    pub suffix: Memo<String>,
    pub dismiss: Callback<()>,
}

pub(super) fn bind(
    node: NodeRef<html::Textarea>,
    value: ReadSignal<String>,
    blocked: Signal<bool>,
) -> Suggestion {
    let state = RwSignal::new(PromptSuggestion::default());
    let selection = RwSignal::new(0..0);
    let focused = RwSignal::new(false);
    let composing = RwSignal::new(false);
    let thread = expect_context::<ActiveThreadId>().0;
    let model = expect_context::<CurrentModel>().0;
    let streaming = expect_context::<IsStreaming>().0;
    let turns = expect_context::<crate::state::Turns>();
    let messages = expect_context::<MessagesSignal>().0;
    let settings = expect_context::<TaskModelsSignal>().0;
    let successful_turn = expect_context::<crate::state::PromptSuggestionTurn>().0;
    let previous = StoredValue::new((None::<String>, false, String::new()));
    let abort = StoredValue::new_local(None::<web_sys::AbortController>);
    let stop = move || {
        abort.update_value(|abort| {
            if let Some(controller) = abort.take() {
                controller.abort();
            }
        })
    };
    on_cleanup(stop);

    Effect::new(move |_| {
        let current_thread = thread.get();
        let running = streaming.get();
        turns.track();
        let busy = current_thread
            .as_deref()
            .is_some_and(|id| turns.is_busy_untracked(id));
        let settings = settings.get();
        let current_model = settings.suggestion(&model.get()).to_string();
        let (old_thread, was_running, old_model) = previous.get_value();
        previous.set_value((current_thread.clone(), running, current_model.clone()));
        if running
            || busy
            || current_thread != old_thread
            || current_model != old_model
            || current_model.is_empty()
            || !settings.prompt_suggestions
        {
            stop();
            state.update(PromptSuggestion::dismiss);
            return;
        }
        // Only a live successful turn's falling edge generates a prediction.
        // Opening saved history and changing a preference never spends tokens.
        if !was_running || blocked.get_untracked() || !value.get_untracked().is_empty() {
            return;
        }
        let Some(thread) = current_thread else { return };
        let Some((turn, context)) = messages.with_untracked(|messages| {
            let last = messages.last()?;
            if last.role != Role::Assistant
                || last.is_streaming()
                || last.error.is_some()
                || last.text().trim().is_empty()
            {
                return None;
            }
            Some((
                last.id.clone(),
                recent_context(
                    messages
                        .iter()
                        .filter(|message| message.error.is_none())
                        .map(|message| PromptLine {
                            role: message.role,
                            text: message.text(),
                        }),
                ),
            ))
        }) else {
            return;
        };
        let request = SuggestionRequest {
            model: current_model,
            messages: context,
        };
        if successful_turn.get_untracked().as_ref() != Some(&(thread.clone(), turn.clone())) {
            return;
        }
        if !request.is_valid() {
            return;
        }
        let key = SuggestionKey {
            thread,
            turn,
            model: request.model.clone(),
        };
        let mut start = false;
        state.update(|state| start = state.begin(key.clone()));
        if !start {
            return;
        }
        stop();
        let controller = match web_sys::AbortController::new() {
            Ok(controller) => controller,
            Err(error) => {
                log::warn!("Prompt suggestion cancellation unavailable: {error:?}");
                return;
            }
        };
        let signal = controller.signal();
        abort.set_value(Some(controller));
        spawn_local(async move {
            let result = crate::api::prompt_suggestion(&request, &signal).await;
            match result {
                Ok(reply) => {
                    let _ = state.try_update(|state| state.finish(&key, reply));
                }
                Err(crate::api::ApiError::Aborted) => {}
                Err(error) => log::warn!("Prompt suggestion unavailable: {error}"),
            }
        });
    });

    let suffix = Memo::new(move |_| {
        turns.track();
        let busy = thread.with(|thread| {
            thread
                .as_deref()
                .is_some_and(|id| turns.is_busy_untracked(id))
        });
        if !focused.get()
            || composing.get()
            || blocked.get()
            || streaming.get()
            || busy
            || !settings.with(|settings| settings.prompt_suggestions)
        {
            return String::new();
        }
        value.with(|draft| {
            state.with(|state| {
                state
                    .suffix(draft, selection.get())
                    .unwrap_or_default()
                    .to_string()
            })
        })
    });
    Effect::new(move |_| {
        value.track();
        if let Some(node) = node.get() {
            remember_selection(&node, selection);
        }
    });
    Suggestion {
        selection,
        focused,
        composing,
        suffix,
        dismiss: Callback::new(move |_| {
            stop();
            state.update(PromptSuggestion::dismiss);
        }),
    }
}

pub(super) fn remember_selection(
    node: &web_sys::HtmlTextAreaElement,
    selection: RwSignal<std::ops::Range<usize>>,
) {
    let value = node.value();
    let start = super::caret::byte_offset(
        &value,
        usize::try_from(node.selection_start().ok().flatten().unwrap_or(0)).unwrap_or(0),
    );
    let end = super::caret::byte_offset(
        &value,
        usize::try_from(node.selection_end().ok().flatten().unwrap_or(0)).unwrap_or(0),
    );
    selection.set(start..end);
}
