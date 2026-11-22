//! Conversations reached through a host-owned scheduled task.
use super::*;
use bruhs_models::ids::ThreadId;
use bruhs_models::reactive::{
    LinkedThreadMessage, LinkedThreadPage, LinkedThreadReply, ScheduledThreadLink,
};

pub(in crate::reactive) fn inventory(
    db: &Connection,
    links: bool,
) -> Result<Vec<ScheduledTaskSummary>, ReactiveError> {
    let mut tasks = schedule_summaries(db, None)?;
    if links {
        for task in &mut tasks {
            if let Some((choice, _)) = task_conversations::binding(db, &task.id)? {
                task.thread = Some(ScheduledThreadLink {
                    id: choice.thread_id,
                    title: choice.title,
                });
            }
        }
    }
    Ok(tasks)
}

fn bound(
    db: &Connection,
    task: &str,
    thread: &ThreadId,
) -> Result<(TaskThreadChoice, String), ReactiveError> {
    let sub: Subscription = required(db, "reactive_subscriptions", task, "scheduled task")?;
    let source: EventSource = required(db, "reactive_sources", &sub.source_id, "trigger")?;
    let binding = task_conversations::binding(db, task)?
        .map(|(c, _)| c)
        .filter(|c| {
            c.thread_id == *thread
                && c.agent_id == sub.agent_id
                && source.kind.is_scheduled_task()
                && sub.task_conversation.has_destination()
        })
        .ok_or_else(|| {
            ReactiveError::Conflict("This task no longer points to that conversation.".into())
        })?;
    let agent: ReactiveAgent = required(db, "reactive_agents", &sub.agent_id, "agent")?;
    let matches_project: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM task_conversations WHERE thread_id=?1 AND project=?2)",
        params![thread.as_str(), agent.project_path],
        |r| r.get(0),
    )?;
    if !matches_project {
        return Err(ReactiveError::Conflict(
            "The conversation moved to another project.".into(),
        ));
    }
    Ok((binding, agent.project_path))
}

// Work backward from the newest message; a cursor identifies the start of the
// previously displayed page, even if a provider posts a new reply meanwhile.
struct Page<'a> {
    skip: Option<&'a str>,
    messages: Vec<LinkedThreadMessage>,
    bytes: usize,
}
impl Page<'_> {
    fn push(&mut self, key: String, assistant: bool, text: String) -> bool {
        // A single provider response can exceed the transport limit. Stable byte
        // offsets let the reader page through it without losing UTF-8 text.
        const MAX_CHUNK: usize = 128 * 1024;
        if text.len() <= MAX_CHUNK {
            return self.push_chunk(key, assistant, &text);
        }
        let mut end = text.len();
        while end > 0 {
            let mut start = end.saturating_sub(MAX_CHUNK);
            while !text.is_char_boundary(start) {
                start += 1;
            }
            if self.push_chunk(format!("{key}:{start}"), assistant, &text[start..end]) {
                return true;
            }
            end = start;
        }
        false
    }

    fn push_chunk(&mut self, key: String, assistant: bool, text: &str) -> bool {
        if let Some(before) = self.skip {
            if before == key {
                self.skip = None;
            }
            return false;
        }
        if !self.messages.is_empty()
            && (self.messages.len() == 40 || self.bytes + text.len() > 256 * 1024)
        {
            return true;
        }
        self.bytes += text.len();
        self.messages.push(LinkedThreadMessage {
            key,
            assistant,
            text: text.into(),
        });
        false
    }
}

pub(in crate::reactive) fn read(
    db: &Connection,
    task: &str,
    thread: &ThreadId,
    before: Option<&str>,
) -> Result<LinkedThreadPage, ReactiveError> {
    let tx = db.unchecked_transaction()?;
    let (choice, project) = bound(&tx, task, thread)?;
    let (title, delivered, desktop) = if choice.desktop_model.is_some() {
        let store: String = tx.query_row(
            "SELECT identity FROM desktop_task_store WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        desktop_tasks::linked_messages(thread, &project, &store)?
    } else {
        (choice.title, 0, Vec::new())
    };
    let mut page = Page {
        skip: before,
        messages: Vec::new(),
        bytes: 0,
    };
    let mut more = false;
    {
        let mut q = tx.prepare("SELECT r.payload FROM reactive_runs r WHERE r.agent_id=?1 AND json_extract(r.payload,'$.thread_id')=?2
            AND (?3=0 OR r.id NOT IN (SELECT json_extract(payload,'$.id') FROM desktop_task_turns WHERE thread_id=?2 AND seq<=?3))
            ORDER BY r.rowid DESC")?;
        let mut rows = q.query(params![choice.agent_id, thread.as_str(), delivered])?;
        while let Some(row) = rows.next()? {
            let run: ReactiveRun = decode(row.get(0)?)?;
            let result = run
                .result
                .or(run.error)
                .unwrap_or_else(|| run_status(run.status).to_owned());
            let owner = run.owner_message.unwrap_or_else(|| {
                if run.trigger == Some(RunTrigger::Chat) {
                    run.prompt
                } else {
                    "Scheduled run".into()
                }
            });
            if page.push(format!("a{}", run.id), true, result)
                || page.push(format!("u{}", run.id), false, owner)
            {
                more = true;
                break;
            }
        }
    }
    if !more {
        for msg in desktop.into_iter().rev() {
            if page.push(msg.key, msg.assistant, msg.text) {
                more = true;
                break;
            }
        }
    }
    if page.skip.is_some() {
        return Err(ReactiveError::Conflict(
            "The conversation changed. Refresh its history.".into(),
        ));
    }
    page.messages.reverse();
    let before = more.then(|| page.messages[0].key.clone());
    Ok(LinkedThreadPage {
        thread: ScheduledThreadLink {
            id: thread.clone(),
            title,
        },
        messages: page.messages,
        before,
    })
}

pub(in crate::reactive) fn send(
    db: &mut Connection,
    task: &str,
    thread: &ThreadId,
    reply: LinkedThreadReply,
) -> Result<(), ReactiveError> {
    transaction(db, |db| {
        let (choice, project) = bound(db, task, thread)?;
        if choice.desktop_model.is_some() {
            desktop_tasks::linked_agent(db, &project, thread)?;
        }
        reports::guard_managed_identity(db, task)?;
        task_conversations::reply_once(db, task, thread, &reply.message, Some(reply.request_id))
    })
}
