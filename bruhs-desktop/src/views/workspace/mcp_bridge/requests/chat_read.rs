use super::*;
use crate::views::chat::Message;
use bruhs_models::workspace_bridge::{
    ChatMessageRecord, ChatThreadSummary, ChatToolCallRecord, ChatUsageRecord,
};

fn message_record(message: &Message) -> ChatMessageRecord<'_> {
    ChatMessageRecord {
        role: message.role.as_str(),
        content: message.content.as_str().into(),
        model: message.model.as_deref(),
        thinking: message.thinking.as_str().into(),
        usage: message.usage.as_ref().map(|usage| ChatUsageRecord {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_input_tokens: usage.cache_read_input_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            context_tokens: usage.context_tokens,
        }),
        duration_ms: message.duration_ms,
        started_at_ms: message.started_at_ms,
        completed_at_ms: message.completed_at_ms,
        tool_calls: message
            .tool_calls
            .iter()
            .map(|call| ChatToolCallRecord {
                id: &call.id,
                tool: &call.tool,
                input: &call.input,
                output: call.output.as_deref(),
                is_error: call.is_error,
            })
            .collect(),
        stopped: message.state.is_stopped(),
        error: message.state.error(),
    }
}

impl Workspace {
    pub(super) fn read_bridge_chat(
        &self,
        query: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        bridge::read_chat_threads(
            self.threads.iter(),
            |thread| {
                let thread = *thread;
                let chat = thread.chat.read(cx);
                ChatThreadSummary {
                    id: &thread.id,
                    title: &chat.thread_title,
                    project_name: &thread.project_name,
                    updated_at_ms: thread.updated_at_ms.as_millis(),
                    pinned: thread.pinned,
                    settled: thread.settled,
                    streaming: chat.is_streaming,
                }
            },
            query,
            offset,
        )
    }

    pub(super) fn read_bridge_chat_messages(
        &self,
        thread_id: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        // Check before even resolving the ID, so revocation cannot reveal existence.
        bridge::authorize_chat_messages(&self.automation_state.permissions, thread_id, offset)?;
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .ok_or(bridge::CHAT_THREAD_NOT_FOUND)?;
        let chat = thread.chat.read(cx);
        // Borrow only completed history. Never snapshot input, stashes or live provider buffers.
        bridge::read_chat_messages(
            chat.messages
                .iter()
                .filter(|message| !message.state.is_streaming()),
            message_record,
            thread_id,
            offset,
        )
    }
}
