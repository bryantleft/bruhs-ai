//! Bind a schedule to an ordinary desktop chat. The desktop remains the sole
//! writer of its workspace. Completed host turns are delivered in commit order.
use super::*;
use bruhs_models::reactive::{DesktopTaskTurn, LinkedThreadMessage};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
struct Workspace<M> {
    #[serde(default)]
    thread_origins: BTreeMap<String, Origin>,
    current_model: Option<String>,
    #[serde(default = "Vec::new")]
    threads: Vec<Chat<M>>,
}

#[derive(Deserialize)]
struct Chat<M> {
    #[serde(skip)]
    delivered: BTreeMap<String, i64>,
    id: String,
    title: String,
    project_path: Option<String>,
    project_settings_root: Option<String>,
    worktree_path: Option<String>,
    #[serde(default)]
    model_id: String,
    session_id: Option<String>,
    session_provider: Option<String>,
    #[serde(default)]
    fresh_session: bool,
    #[serde(default, deserialize_with = "bruhs_models::time::deserialize_legacy")]
    updated_at_ms: EpochMs,
    #[serde(default = "Vec::new")]
    messages: Vec<M>,
}

#[derive(Deserialize)]
struct Origin {
    side_chat: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct Line {
    role: String,
    content: String,
}

fn workspace<M: DeserializeOwned>(only: Option<&ThreadId>) -> Result<Vec<Chat<M>>, ReactiveError> {
    let Some(path) = crate::delivery::writer::desktop_state_file() else {
        return Ok(Vec::new());
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ReactiveError::Storage(format!(
                "Could not read desktop conversations: {error}"
            )))
        }
    };
    let parse_error = |error: serde_json::Error| {
        ReactiveError::Storage(format!(
            "Could not read the saved desktop conversation list: {error}"
        ))
    };
    if let Some(id) = only {
        // The profile can contain many large transcripts. Decode message bodies
        // only for the chat the user bound to the schedule.
        #[derive(Deserialize)]
        struct Records<'a> {
            #[serde(default)]
            desktop_task_cursors: BTreeMap<String, i64>,
            #[serde(default)]
            thread_origins: BTreeMap<String, Origin>,
            current_model: Option<String>,
            #[serde(borrow, default)]
            threads: Vec<&'a serde_json::value::RawValue>,
        }
        #[derive(Deserialize)]
        struct Identity<'a> {
            id: std::borrow::Cow<'a, str>,
        }
        let records = serde_json::from_slice::<Records<'_>>(&bytes).map_err(parse_error)?;
        if records
            .thread_origins
            .get(id.as_str())
            .is_some_and(|o| o.side_chat.is_some())
        {
            return Ok(Vec::new());
        }
        for raw in records.threads {
            if serde_json::from_str::<Identity<'_>>(raw.get())
                .map_err(parse_error)?
                .id
                == id.as_str()
            {
                return serde_json::from_str::<Chat<M>>(raw.get())
                    .map(|mut chat| {
                        chat.delivered = records.desktop_task_cursors;
                        vec![chat.with_defaults(records.current_model.as_deref())]
                    })
                    .map_err(parse_error);
            }
        }
        return Ok(Vec::new());
    }
    serde_json::from_slice::<Workspace<M>>(&bytes)
        .map(|w| {
            w.threads
                .into_iter()
                .filter(|c| {
                    !w.thread_origins
                        .get(&c.id)
                        .is_some_and(|o| o.side_chat.is_some())
                })
                .map(|c| c.with_defaults(w.current_model.as_deref()))
                .collect()
        })
        .map_err(parse_error)
}

impl<M> Chat<M> {
    fn with_defaults(mut self, current_model: Option<&str>) -> Self {
        if self.model_id.trim().is_empty() {
            self.model_id = current_model
                .unwrap_or(bruhs_models::agent::DEFAULT_AGENT_MODEL)
                .to_owned();
        }
        if self.title.trim().is_empty() {
            self.title = "Untitled thread".into();
        }
        self
    }
    fn project(&self) -> Option<&str> {
        self.project_settings_root
            .as_deref()
            .or(self.project_path.as_deref())
    }
    fn checkout(&self) -> TaskCheckout {
        self.worktree_path
            .as_ref()
            .filter(|p| Some(p.as_str()) != self.project())
            .map(|p| TaskCheckout::Existing { path: p.clone() })
            .unwrap_or_default()
    }
    fn session(&self) -> Option<&str> {
        let provider = bruhs_models::agent::AgentProvider::for_model(&self.model_id);
        (!self.fresh_session && self.session_provider.as_deref() == Some(provider.as_str()))
            .then_some(self.session_id.as_deref())
            .flatten()
            .filter(|id| {
                !id.is_empty()
                    && (provider != bruhs_models::agent::AgentProvider::Acp
                        || bruhs_models::agent_extensions::acp_session_matches_model(
                            id,
                            &self.model_id,
                        ))
            })
    }
}

pub(in crate::reactive) fn choices(
    project: &str,
    query: &str,
) -> Result<Vec<TaskThreadChoice>, ReactiveError> {
    let query = query.trim().to_lowercase();
    let mut chats = workspace::<serde::de::IgnoredAny>(None)?;
    chats.retain(|c| {
        c.project() == Some(project)
            && !c.model_id.is_empty()
            && c.title.to_lowercase().contains(&query)
    });
    chats.sort_by_key(|c| std::cmp::Reverse(c.updated_at_ms));
    Ok(chats
        .into_iter()
        .filter_map(|c| {
            let thread_id = ThreadId::try_from(c.id.as_str()).ok()?;
            Some(TaskThreadChoice {
                thread_id,
                agent_id: c.id.clone(),
                checkout: c.checkout(),
                title: c.title,
                desktop_model: Some(c.model_id),
                agent_model: None,
            })
        })
        .take(HISTORY_LIMIT)
        .collect())
}

fn project_chat<M: DeserializeOwned>(
    id: &ThreadId,
    project: &str,
) -> Result<Chat<M>, ReactiveError> {
    let chat = workspace::<M>(Some(id))?
        .into_iter()
        .next()
        .ok_or_else(|| {
            ReactiveError::Conflict(
                "The desktop thread is no longer available as a scheduled destination.".into(),
            )
        })?;
    if chat.project() != Some(project) {
        return Err(ReactiveError::Conflict("The desktop thread belongs to another project. Choose a destination in this task’s project.".into()));
    }
    Ok(chat)
}

pub(super) fn linked_messages(
    id: &ThreadId,
    project: &str,
    store: &str,
) -> Result<(String, i64, Vec<LinkedThreadMessage>), ReactiveError> {
    let chat = project_chat::<Line>(id, project)?;
    let cursor = chat
        .delivered
        .get(&format!("{store}/{id}"))
        .copied()
        .unwrap_or_default();
    let messages = chat
        .messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let assistant = match line.role.as_str() {
                "user" => false,
                "assistant" => true,
                _ => return None,
            };
            Some(LinkedThreadMessage {
                key: format!("d{index}"),
                assistant,
                text: line.content,
            })
        })
        .collect();
    Ok((chat.title, cursor, messages))
}

pub(super) fn linked_agent(
    db: &Connection,
    project: &str,
    id: &ThreadId,
) -> Result<ReactiveAgent, ReactiveError> {
    let chat = project_chat::<serde::de::IgnoredAny>(id, project)?;
    reports::guard_managed_identity(db, id.as_str())?;
    if is_desktop(db, id)? {
        let mut agent: ReactiveAgent = required(db, "reactive_agents", id.as_str(), "agent")?;
        if agent.project_path != project {
            return Err(ReactiveError::Conflict(
                "The conversation moved to another project.".into(),
            ));
        }
        agent.model = chat.model_id;
        return Ok(agent);
    }
    Ok(ReactiveAgent {
        id: id.to_string(),
        name: chat.title,
        project_path: project.into(),
        model: chat.model_id,
        instructions: "Continue the selected desktop conversation.".into(),
        memory: String::new(),
        enabled: true,
    })
}

pub(super) fn prepare_binding(
    db: &Connection,
    setup: &AutomationSetup,
    id: &ThreadId,
) -> Result<(), ReactiveError> {
    let chat = project_chat::<serde::de::IgnoredAny>(id, &setup.agent.project_path)?;
    if setup.agent.model != chat.model_id || setup.subscription.task_checkout != chat.checkout() {
        return Err(ReactiveError::Conflict(
            "The desktop thread’s model or checkout changed. Reopen the task editor.".into(),
        ));
    }
    if !setup.subscription.task_conversation.has_destination() || setup.agent.id != id.as_str() {
        return Err(ReactiveError::Conflict(
            "Choose a persistent destination for this desktop thread.".into(),
        ));
    }
    if is_desktop(db, id)? {
        let mut saved: ReactiveAgent = required(db, "reactive_agents", &setup.agent.id, "agent")?;
        saved.model.clone_from(&chat.model_id);
        if saved != setup.agent {
            return Err(ReactiveError::Conflict(
                "The conversation’s task settings changed. Reopen the task editor.".into(),
            ));
        }
        db.execute(
            "UPDATE task_conversations SET title=?2,checkout=?3 WHERE thread_id=?1",
            params![id.as_str(), chat.title, encode(&chat.checkout())?],
        )?;
        db.execute("UPDATE desktop_task_threads SET last_desktop_session=CASE WHEN model=?2 THEN last_desktop_session ELSE NULL END, model=?2 WHERE thread_id=?1",
            params![id.as_str(),chat.model_id])?;
        return Ok(());
    }
    if get::<ReactiveAgent>(db, "reactive_agents", &setup.agent.id)?.is_some() {
        return Err(ReactiveError::Conflict(
            "Another agent already uses this desktop thread ID.".into(),
        ));
    }
    db.execute("INSERT INTO task_conversations(thread_id,agent_id,project,title,checkout,session_id,stamp,updated) VALUES(?1,?1,?2,?3,?4,?5,?6,?7)",
        params![id.as_str(),setup.agent.project_path,chat.title,encode(&chat.checkout())?,chat.session(),super::task_conversations::stamp(&setup.agent)?,chat.updated_at_ms.as_millis()])?;
    db.execute(
        "INSERT INTO desktop_task_threads(thread_id,model,last_desktop_session) VALUES(?1,?2,?3)",
        params![id.as_str(), chat.model_id, chat.session()],
    )?;
    Ok(())
}

pub(super) fn is_desktop(db: &Connection, id: &ThreadId) -> Result<bool, ReactiveError> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM desktop_task_threads WHERE thread_id=?1)",
        [id.as_str()],
        |r| r.get(0),
    )?)
}

pub(in crate::reactive) fn prepare_turn(
    db: &Connection,
    agent: &mut ReactiveAgent,
    run: &mut ReactiveRun,
) -> Result<String, ReactiveError> {
    let chat = project_chat::<Line>(&run.thread_id, &agent.project_path)?;
    let sub: Subscription = required(
        db,
        "reactive_subscriptions",
        run.subscription_id.as_deref().unwrap_or_default(),
        "task",
    )?;
    let checkout = chat.checkout();
    let changed = agent.model != chat.model_id || run.task_checkout != checkout;
    if changed {
        // Project, source and task instructions still belong to the schedule. The
        // model and checkout follow the existing conversation on every dispatch.
        agent.model.clone_from(&chat.model_id);
        run.task_checkout = checkout;
        db.execute(
            "UPDATE reactive_agents SET payload=json_set(payload,'$.model',?2) WHERE id=?1",
            params![agent.id, agent.model],
        )?;
        db.execute(
            "UPDATE reactive_subscriptions SET payload=json_set(payload,'$.task_checkout',json(?2))
            WHERE id IN (SELECT subscription_id FROM task_destinations WHERE thread_id=?1)",
            params![run.thread_id.as_str(), encode(&run.task_checkout)?],
        )?;
    }
    db.execute(
        "UPDATE task_conversations SET title=?2,checkout=?3 WHERE thread_id=?1",
        params![
            run.thread_id.as_str(),
            chat.title,
            encode(&run.task_checkout)?
        ],
    )?;
    let previous: Option<String> = db.query_row(
        "SELECT last_desktop_session FROM desktop_task_threads WHERE thread_id=?1",
        [run.thread_id.as_str()],
        |row| row.get(0),
    )?;
    if changed || previous.as_deref() != chat.session() {
        run.session_id = chat.session().map(str::to_owned);
        db.execute(
            "UPDATE desktop_task_threads SET last_desktop_session=?2,model=?3 WHERE thread_id=?1",
            params![run.thread_id.as_str(), chat.session(), agent.model],
        )?;
        db.execute(
            "UPDATE task_conversations SET session_id=?2,stamp=?3 WHERE thread_id=?1",
            params![
                run.thread_id.as_str(),
                chat.session(),
                super::task_conversations::stamp(agent)?
            ],
        )?;
    }
    if sub.task_conversation != TaskConversation::Continue
        && !matches!(
            run.trigger,
            Some(RunTrigger::Chat | RunTrigger::Answer { .. })
        )
    {
        run.session_id = None;
        return Ok(String::new());
    }
    if run.session_id.is_some() {
        return Ok(String::new());
    }
    use bruhs_models::agent::Role;
    use bruhs_models::logic::portable::{thread_context, PortableMessage, PortablePart};
    let messages = chat
        .messages
        .into_iter()
        .filter_map(|line| {
            let role = match line.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };
            (!line.content.is_empty()).then(|| PortableMessage {
                role,
                parts: vec![PortablePart::Text(line.content)],
                started_at_ms: None,
                model: None,
            })
        })
        .collect::<Vec<_>>();
    Ok(thread_context(&chat.title, &messages).unwrap_or_default())
}

pub(super) fn activity(db: &Connection) -> Result<BTreeMap<String, i64>, ReactiveError> {
    let mut statement=db.prepare("SELECT t.thread_id, COALESCE((SELECT seq FROM desktop_task_turns s WHERE s.thread_id=t.thread_id ORDER BY seq DESC LIMIT 1),0) FROM desktop_task_threads t")?;
    let rows = statement.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(in crate::reactive) fn turns(
    db: &Connection,
    thread: &ThreadId,
    after: i64,
) -> Result<Vec<DesktopTaskTurn>, ReactiveError> {
    let mut statement=db.prepare("SELECT seq,title,model,payload FROM desktop_task_turns WHERE thread_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3")?;
    let mut rows = statement.query(params![
        thread.as_str(),
        after,
        i64::try_from(HISTORY_LIMIT).unwrap_or(i64::MAX)
    ])?;
    let mut turns = Vec::new();
    while let Some(row) = rows.next()? {
        turns.push(DesktopTaskTurn {
            sequence: row.get(0)?,
            title: row.get(1)?,
            model: row.get(2)?,
            run: decode(row.get(3)?)?,
        });
    }
    Ok(turns)
}
