use super::*;
use crate::views::chat::{DesktopToolCall, Message};

use super::read_page as page;

#[derive(Serialize)]
struct ThreadSummary<'a> {
    id: &'a str,
    title: &'a str,
    project_name: &'a str,
    updated_at_ms: i64,
    pinned: bool,
    settled: bool,
    streaming: bool,
}

#[derive(Serialize)]
struct MessageRecord<'a> {
    role: &'a str,
    content: &'a str,
    model: &'a Option<String>,
    thinking: &'a str,
    usage: &'a Option<bruhs_core::UsageInfo>,
    duration_ms: Option<u64>,
    started_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
    tool_calls: &'a [DesktopToolCall],
    stopped: bool,
    error: Option<&'a str>,
}

impl<'a> From<&'a Message> for MessageRecord<'a> {
    fn from(message: &'a Message) -> Self {
        Self {
            role: message.role.as_str(),
            content: &message.content,
            model: &message.model,
            thinking: &message.thinking,
            usage: &message.usage,
            duration_ms: message.duration_ms,
            started_at_ms: message.started_at_ms,
            completed_at_ms: message.completed_at_ms,
            tool_calls: &message.tool_calls,
            stopped: message.state.is_stopped(),
            error: message.state.error(),
        }
    }
}

impl Workspace {
    pub(super) fn read_bridge_chat(
        &self,
        query: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        if query.len() > 512 {
            return Err("Query is too large.".into());
        }
        let query = query.to_lowercase();
        let summaries = self.threads.iter().filter_map(|thread| {
            let chat = thread.chat.read(cx);
            if !query.is_empty()
                && ![
                    thread.id.as_str(),
                    chat.thread_title.as_str(),
                    thread.project_name.as_str(),
                ]
                .iter()
                .any(|field| field.to_lowercase().contains(&query))
            {
                return None;
            }
            Some(ThreadSummary {
                id: &thread.id,
                title: &chat.thread_title,
                project_name: &thread.project_name,
                updated_at_ms: thread.updated_at_ms.as_millis(),
                pinned: thread.pinned,
                settled: thread.settled,
                streaming: chat.is_streaming,
            })
        });
        page::read_page(summaries, offset)
    }

    pub(super) fn read_bridge_chat_messages(
        &self,
        thread_id: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        // Check before even resolving the ID, so revocation cannot reveal existence.
        if !self
            .automation_state
            .permissions
            .allows(AgentDomain::Chat, AgentAccess::Read)
        {
            return Err("Read access is not granted for this domain.".into());
        }
        if thread_id.trim().is_empty() || thread_id.len() > 128 || offset > 100_000 {
            return Err("Invalid thread ID or offset.".into());
        }
        let thread = self
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .ok_or("Chat thread was not found in the current workspace.")?;
        let chat = thread.chat.read(cx);
        // Borrow only completed history. Never snapshot input, stashes or live provider buffers.
        let messages = chat
            .messages
            .iter()
            .filter(|message| !message.state.is_streaming())
            .map(MessageRecord::from);
        let mut result = page::read_page(messages, offset)?;
        result["thread_id"] = thread_id.into();
        Ok(result)
    }
}
