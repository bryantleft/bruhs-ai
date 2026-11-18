//! Rooms: the rooms, their transcripts and the runs a post queues. A room
//! run is an ordinary run whose trigger names the room and the message it
//! answers; the routing rules and copy live in `bruhs_models::crew::room`.

use super::{
    check_capacity, crew, decode, encode, get, new_run, queue_capacity, records, required,
    runs_where, save, save_run, valid_id, ReactiveError,
};
use bruhs_models::{
    crew::{
        room::{
            check_room, handoff_failed_line, handoff_stopped_line, left_line, recipients,
            room_next_hop, room_order, room_prompt, skipped_line, stopped_line, Room, RoomAuthor,
            RoomMember, RoomMessage, RoomState, FORMER_BRUH, ROOM_CONTEXT_MESSAGES,
        },
        BruhStatus, STOPPED_LINE,
    },
    reactive::{
        OutboxItem, ReactiveAgent, ReactiveRun, RunOutcome, RunStatus, RunTrigger, HISTORY_LIMIT,
    },
    weekly_reports::managed_preset_id,
    EpochMs,
};
use rusqlite::{params, Connection};
use std::collections::HashMap;

const REMOVED: &str = "Removed from the room.";
const DELETED: &str = "The room was deleted.";

pub(in crate::reactive) fn initialize(connection: &Connection) -> Result<(), ReactiveError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS reactive_rooms(id TEXT PRIMARY KEY,payload TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS reactive_room_messages(id TEXT PRIMARY KEY,room_id TEXT NOT NULL,seq INTEGER NOT NULL,result_of TEXT UNIQUE,payload TEXT NOT NULL,UNIQUE(room_id,seq));",
    )?;
    Ok(())
}

/// What a message carries beyond its author and text. `result_of` is the
/// run whose one terminal message this is.
#[derive(Debug, Default)]
pub(in crate::reactive) struct Extra {
    pub(in crate::reactive) run_id: Option<String>,
    pub(in crate::reactive) outbox_id: Option<String>,
    pub(in crate::reactive) failed: bool,
    pub(in crate::reactive) result_of: Option<String>,
}

/// Create a room, or replace one. A member taken out loses its live runs in
/// this room; a name or lead change stops nothing.
pub(in crate::reactive) fn save_room(
    connection: &Connection,
    room: &Room,
    create: bool,
) -> Result<(), ReactiveError> {
    valid_id(&room.id)?;
    let old: Option<Room> = get(connection, "reactive_rooms", &room.id)?;
    match (&old, create) {
        (Some(_), true) => {
            return Err(ReactiveError::Conflict(
                "A room with this ID already exists.".into(),
            ))
        }
        (None, false) => return Err(ReactiveError::NotFound("The room was not found.".into())),
        _ => {}
    }
    let agents = records::<ReactiveAgent>(connection, "reactive_agents")?;
    let eligible: Vec<&str> = agents
        .iter()
        .map(|agent| agent.id.as_str())
        .filter(|id| managed_preset_id(id).is_none())
        .collect();
    check_room(room, &eligible).map_err(|reason| ReactiveError::Invalid(reason.into()))?;
    if create {
        check_capacity(connection, "reactive_rooms", &room.id)?;
    }
    if let Some(old) = old {
        for gone in old.members.iter().filter(|id| !room.members.contains(id)) {
            cancel(connection, &room.id, Some(gone), REMOVED)?;
        }
    }
    save(connection, "reactive_rooms", &room.id, room)
}

/// Stop the room's live runs, then drop its messages and the room.
pub(in crate::reactive) fn delete_room(
    connection: &Connection,
    id: &str,
) -> Result<(), ReactiveError> {
    required::<Room>(connection, "reactive_rooms", id, "room")?;
    cancel(connection, id, None, DELETED)?;
    connection.execute("DELETE FROM reactive_room_messages WHERE room_id=?1", [id])?;
    connection.execute("DELETE FROM reactive_rooms WHERE id=?1", [id])?;
    Ok(())
}

/// Cancel the live runs in a room, for one member or all of them.
fn cancel(
    connection: &Connection,
    room_id: &str,
    agent_id: Option<&str>,
    reason: &str,
) -> Result<(), ReactiveError> {
    for mut run in runs_where(
        connection,
        "json_extract(payload,'$.trigger.room_id')=?1 AND status IN ('awaiting_review','queued','running')",
        Some(room_id),
    )? {
        if agent_id.is_some_and(|id| id != run.agent_id) {
            continue;
        }
        run.status = RunStatus::Cancelled;
        run.updated_at = EpochMs::now();
        run.error = Some(reason.into());
        save_run(connection, &run)?;
    }
    Ok(())
}

/// A deleted bruh leaves every room it was in: it is no longer a member or
/// the lead, and each room it was in says so.
pub(in crate::reactive) fn remove_bruh(
    connection: &Connection,
    agent_id: &str,
    name: &str,
) -> Result<(), ReactiveError> {
    for mut room in records::<Room>(connection, "reactive_rooms")? {
        let member = room.members.iter().any(|id| id == agent_id);
        let lead = room.lead.as_deref() == Some(agent_id);
        if !member && !lead {
            continue;
        }
        room.members.retain(|id| id != agent_id);
        if lead {
            room.lead = None;
        }
        save(connection, "reactive_rooms", &room.id, &room)?;
        if member {
            append(
                connection,
                &room.id,
                RoomAuthor::System,
                left_line(name),
                Extra::default(),
            )?;
        }
    }
    Ok(())
}

/// A room run asked the owner something (a question or an approval): the
/// room shows it as a card beside the bruh's other lines.
pub(in crate::reactive) fn ask_opened(
    connection: &Connection,
    run: &ReactiveRun,
    item: &OutboxItem,
) -> Result<(), ReactiveError> {
    let Some(room_id) = run.trigger.as_ref().and_then(RunTrigger::room_id) else {
        return Ok(());
    };
    let text = if item.message.trim().is_empty() {
        item.title.clone()
    } else {
        item.message.clone()
    };
    append(
        connection,
        room_id,
        RoomAuthor::Bruh {
            agent_id: run.agent_id.clone(),
        },
        text,
        Extra {
            run_id: Some(run.id.clone()),
            outbox_id: Some(item.id.clone()),
            ..Extra::default()
        },
    )?;
    Ok(())
}

/// The trigger for the run a late answer queues: a room run's own room,
/// marked resumed so it keeps the owner's answer as its prompt. The answer
/// is the owner's input, so the hop count starts again.
pub(in crate::reactive) fn resume_trigger(original: &ReactiveRun) -> Option<RunTrigger> {
    match &original.trigger {
        Some(RunTrigger::Room {
            room_id,
            message_id,
            ..
        }) => Some(RunTrigger::Room {
            room_id: room_id.clone(),
            message_id: message_id.clone(),
            hop: 0,
            resumed: true,
        }),
        _ => None,
    }
}

/// [`resume_trigger`] while the room still has the bruh as a member. A bruh
/// that left the room, or a room that is gone, hears the answer in its own
/// chat instead.
pub(in crate::reactive) fn resume_trigger_for(
    connection: &Connection,
    original: &ReactiveRun,
) -> Result<Option<RunTrigger>, ReactiveError> {
    let Some(trigger) = resume_trigger(original) else {
        return Ok(None);
    };
    let Some(room_id) = trigger.room_id() else {
        return Ok(None);
    };
    let member = get::<Room>(connection, "reactive_rooms", room_id)?
        .is_some_and(|room| room.members.contains(&original.agent_id));
    Ok(member.then_some(trigger))
}

/// The owner posts in a room: the message is kept, and the bruhs it reaches
/// get a run each. A full queue fails the post.
pub(in crate::reactive) fn post_owner(
    connection: &Connection,
    room_id: &str,
    text: &str,
) -> Result<(), ReactiveError> {
    super::text(text, "Room message", true)?;
    let room: Room = required(connection, "reactive_rooms", room_id, "room")?;
    let message = append(
        connection,
        room_id,
        RoomAuthor::Owner,
        text.to_owned(),
        Extra::default(),
    )?
    .ok_or_else(|| ReactiveError::NotFound("The room was not found.".into()))?;
    let crew = members(connection, &room)?;
    let to = recipients(text, &room, &names(&crew), &RoomAuthor::Owner);
    hand_to(connection, &room, &message, &to, 0)
}

/// Add a line at the end of a room. `None` when the room is gone or the run
/// in `extra.result_of` already has its message.
pub(in crate::reactive) fn append(
    connection: &Connection,
    room_id: &str,
    author: RoomAuthor,
    text: String,
    extra: Extra,
) -> Result<Option<RoomMessage>, ReactiveError> {
    let room: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM reactive_rooms WHERE id=?1)",
        [room_id],
        |row| row.get(0),
    )?;
    if !room {
        return Ok(None);
    }
    if let Some(run_id) = &extra.result_of {
        let written: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM reactive_room_messages WHERE result_of=?1)",
            [run_id],
            |row| row.get(0),
        )?;
        if written {
            return Ok(None);
        }
    }
    let seq: i64 = connection.query_row(
        "SELECT COALESCE(MAX(seq),0)+1 FROM reactive_room_messages WHERE room_id=?1",
        [room_id],
        |row| row.get(0),
    )?;
    let message = RoomMessage {
        id: uuid::Uuid::new_v4().to_string(),
        room_id: room_id.to_owned(),
        seq: u64::try_from(seq).map_err(|_| {
            ReactiveError::Storage("A saved room message number is malformed.".into())
        })?,
        author,
        text,
        run_id: extra.run_id,
        outbox_id: extra.outbox_id,
        failed: extra.failed,
        created_at: EpochMs::now(),
    };
    connection.execute(
        "INSERT INTO reactive_room_messages(id,room_id,seq,result_of,payload) VALUES(?1,?2,?3,?4,?5)",
        params![message.id, room_id, seq, extra.result_of, encode(&message)?],
    )?;
    Ok(Some(message))
}

/// The room's members that still exist, in member order.
fn members(connection: &Connection, room: &Room) -> Result<Vec<ReactiveAgent>, ReactiveError> {
    let mut crew = Vec::new();
    for id in &room.members {
        if let Some(agent) = get::<ReactiveAgent>(connection, "reactive_agents", id)? {
            crew.push(agent);
        }
    }
    Ok(crew)
}

fn names(crew: &[ReactiveAgent]) -> Vec<RoomMember<'_>> {
    crew.iter()
        .map(|agent| RoomMember {
            agent_id: &agent.id,
            name: &agent.name,
        })
        .collect()
}

/// The room's newest lines plus the trigger, oldest first: enough for one
/// envelope.
fn prompt_history(
    connection: &Connection,
    room_id: &str,
    trigger_id: &str,
) -> Result<Vec<RoomMessage>, ReactiveError> {
    let limit = i64::try_from(ROOM_CONTEXT_MESSAGES)
        .map_err(|_| ReactiveError::Invalid("Room context size is invalid.".into()))?;
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_room_messages WHERE room_id=?1 AND id<>?2 ORDER BY seq DESC LIMIT ?3",
    )?;
    let rows = statement.query_map(params![room_id, trigger_id, limit], |row| {
        row.get::<_, String>(0)
    })?;
    let mut history = rows
        .map(|row| decode::<RoomMessage>(row?))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(trigger) = get::<RoomMessage>(connection, "reactive_room_messages", trigger_id)? {
        history.push(trigger);
    }
    history.sort_by_key(|message| message.seq);
    Ok(history)
}

/// The run `agent_id` has queued in `room_id` and not yet started, unless
/// the owner's decision resumed it (that one keeps its own prompt).
fn queued_run(
    connection: &Connection,
    agent_id: &str,
    room_id: &str,
) -> Result<Option<ReactiveRun>, ReactiveError> {
    let mut statement = connection.prepare(
        "SELECT payload FROM reactive_runs WHERE agent_id=?1 AND status='queued' AND json_extract(payload,'$.trigger.room_id')=?2 AND COALESCE(json_extract(payload,'$.trigger.resumed'),0)=0 ORDER BY created,rowid LIMIT 1",
    )?;
    let mut rows =
        statement.query_map(params![agent_id, room_id], |row| row.get::<_, String>(0))?;
    rows.next().map(|row| decode(row?)).transpose()
}

/// Hand `message` to each of `agent_ids` that is a member. A paused or off
/// bruh gets a System line instead. A bruh with a run queued in this room
/// already keeps that one run, now answering `message`: an owner post
/// restarts its hop count, a bruh's line never lowers it.
pub(in crate::reactive) fn hand_to(
    connection: &Connection,
    room: &Room,
    message: &RoomMessage,
    agent_ids: &[String],
    hop: u32,
) -> Result<(), ReactiveError> {
    let crew = members(connection, room)?;
    let names = names(&crew);
    let history = prompt_history(connection, &room.id, &message.id)?;
    for id in agent_ids {
        let Some(agent) = crew.iter().find(|agent| agent.id == *id) else {
            continue;
        };
        let skipped = if !agent.enabled {
            Some(BruhStatus::Off)
        } else if crew::is_paused(connection, &agent.id)? {
            Some(BruhStatus::Paused)
        } else {
            None
        };
        if let Some(status) = skipped {
            append(
                connection,
                &room.id,
                RoomAuthor::System,
                skipped_line(&agent.name, status),
                Extra::default(),
            )?;
            continue;
        }
        let prompt = room_prompt(room, &names, &agent.name, &history, &message.id);
        if let Some(mut run) = queued_run(connection, &agent.id, &room.id)? {
            if let Some(RunTrigger::Room {
                message_id,
                hop: queued_hop,
                ..
            }) = run.trigger.as_mut()
            {
                *message_id = message.id.clone();
                *queued_hop = if message.author == RoomAuthor::Owner {
                    0
                } else {
                    (*queued_hop).max(hop)
                };
            }
            run.prompt = prompt;
            run.updated_at = EpochMs::now();
            save_run(connection, &run)?;
            continue;
        }
        queue_capacity(connection)?;
        let mut run = new_run(connection, agent, prompt, None, None, RunStatus::Queued, 0)?;
        run.trigger = Some(RunTrigger::Room {
            room_id: room.id.clone(),
            message_id: message.id.clone(),
            hop,
            resumed: false,
        });
        save_run(connection, &run)?;
    }
    Ok(())
}

/// A room run ended: post its one message, and pass a reply's @mentions
/// on. A repeat save of the run writes nothing.
pub(in crate::reactive) fn on_run_saved(
    connection: &Connection,
    run: &ReactiveRun,
) -> Result<(), ReactiveError> {
    let Some(RunTrigger::Room { room_id, hop, .. }) = &run.trigger else {
        return Ok(());
    };
    let me = || RoomAuthor::Bruh {
        agent_id: run.agent_id.clone(),
    };
    let (author, text, failed) = match run.status {
        RunStatus::Completed => (me(), reply_text(run), false),
        RunStatus::Failed | RunStatus::Interrupted => (me(), failure_text(run), true),
        RunStatus::Cancelled => {
            // However it was cancelled, a late answer must not resume it.
            crew::withdraw_run_approvals(connection, &run.id)?;
            let name = get::<ReactiveAgent>(connection, "reactive_agents", &run.agent_id)?
                .map_or_else(|| FORMER_BRUH.to_owned(), |agent| agent.name);
            (
                RoomAuthor::System,
                stopped_line(&name, run.error.as_deref()),
                false,
            )
        }
        _ => return Ok(()),
    };
    if run.status == RunStatus::Completed && question_only(connection, run)? {
        return Ok(());
    }
    let extra = Extra {
        run_id: Some(run.id.clone()),
        failed,
        result_of: Some(run.id.clone()),
        ..Extra::default()
    };
    let Some(message) = append(connection, room_id, author, text, extra)? else {
        return Ok(());
    };
    if run.status != RunStatus::Completed {
        return Ok(());
    }
    let Some(room) = get::<Room>(connection, "reactive_rooms", room_id)? else {
        return Ok(());
    };
    let crew = members(connection, &room)?;
    let to = recipients(&message.text, &room, &names(&crew), &message.author);
    if to.is_empty() {
        return Ok(());
    }
    let Some(next) = room_next_hop(*hop) else {
        append(
            connection,
            room_id,
            RoomAuthor::System,
            handoff_stopped_line(),
            Extra::default(),
        )?;
        return Ok(());
    };
    for id in &to {
        match hand_to(connection, &room, &message, std::slice::from_ref(id), next) {
            Ok(()) => {}
            Err(ReactiveError::Conflict(error) | ReactiveError::Invalid(error)) => {
                let name = crew
                    .iter()
                    .find(|agent| agent.id == *id)
                    .map_or(FORMER_BRUH, |agent| agent.name.as_str());
                append(
                    connection,
                    room_id,
                    RoomAuthor::System,
                    handoff_failed_line(name, &error),
                    Extra::default(),
                )?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn said(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_owned())
}

/// What a finished room run says: its reply or, when it wrote none, what its
/// report tells the owner (a room run's update gets no card of its own).
fn reply_text(run: &ReactiveRun) -> String {
    run.result
        .as_deref()
        .and_then(said)
        .or_else(|| {
            run.report
                .as_ref()
                .filter(|report| report.outcome != RunOutcome::Silent)
                .and_then(|report| said(&report.message).or_else(|| said(&report.title)))
        })
        .unwrap_or_else(|| STOPPED_LINE.to_owned())
}

/// The run wrote no reply, only a question, and the room already shows that
/// question as its card: saying it again would repeat it.
fn question_only(connection: &Connection, run: &ReactiveRun) -> Result<bool, ReactiveError> {
    if run.result.as_deref().and_then(said).is_some()
        || run
            .report
            .as_ref()
            .is_none_or(|report| report.outcome != RunOutcome::Ask)
    {
        return Ok(false);
    }
    // The run's question is keyed by the run's id; its approvals are not.
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM reactive_room_messages m JOIN reactive_outbox o ON json_extract(m.payload,'$.outbox_id')=o.id WHERE o.run_id=?1)",
        [&run.id],
        |row| row.get(0),
    )?)
}

/// What a failed room run says. A room run can't be retried (the owner
/// posts again), so an error that says to retry reads as stopped.
fn failure_text(run: &ReactiveRun) -> String {
    run.error
        .as_deref()
        .and_then(said)
        .filter(|error| {
            let error = error.to_lowercase();
            !error.contains("retry") && !error.contains("retried")
        })
        .unwrap_or_else(|| STOPPED_LINE.to_owned())
}

/// The envelope a room run starts with: the room as it reads now. A run
/// whose room, membership or trigger message is gone keeps the prompt it
/// was queued with.
pub(in crate::reactive) fn prompt_for(
    connection: &Connection,
    run: &ReactiveRun,
) -> Result<String, ReactiveError> {
    let Some(RunTrigger::Room {
        room_id,
        message_id,
        ..
    }) = &run.trigger
    else {
        return Ok(run.prompt.clone());
    };
    let Some(room) = get::<Room>(connection, "reactive_rooms", room_id)? else {
        return Ok(run.prompt.clone());
    };
    let crew = members(connection, &room)?;
    let Some(me) = crew.iter().find(|agent| agent.id == run.agent_id) else {
        return Ok(run.prompt.clone());
    };
    let history = prompt_history(connection, room_id, message_id)?;
    if !history.iter().any(|message| message.id == *message_id) {
        return Ok(run.prompt.clone());
    }
    Ok(room_prompt(
        &room,
        &names(&crew),
        &me.name,
        &history,
        message_id,
    ))
}

/// Every room with its last message number and the members with a queued
/// or running run in it, in rail order.
pub(in crate::reactive) fn room_states(
    connection: &Connection,
) -> Result<Vec<RoomState>, ReactiveError> {
    let mut statement = connection
        .prepare("SELECT room_id,MAX(seq) FROM reactive_room_messages GROUP BY room_id")?;
    let last: HashMap<String, i64> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut statement = connection.prepare(
        "SELECT DISTINCT json_extract(payload,'$.trigger.room_id'),agent_id FROM reactive_runs WHERE status IN ('queued','running') AND json_extract(payload,'$.trigger.room_id') IS NOT NULL",
    )?;
    let busy: Vec<(String, String)> = statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<_, _>>()?;
    let mut states: Vec<RoomState> = records::<Room>(connection, "reactive_rooms")?
        .into_iter()
        .map(|room| RoomState {
            last_seq: last
                .get(&room.id)
                .and_then(|seq| u64::try_from(*seq).ok())
                .unwrap_or_default(),
            working: room
                .members
                .iter()
                .filter(|id| busy.iter().any(|(r, a)| *r == room.id && a == *id))
                .cloned()
                .collect(),
            room,
        })
        .collect();
    room_order(&mut states);
    Ok(states)
}

/// One room's messages, newest first, a page at a time. `before` is the id
/// of the last message on the previous page.
pub(in crate::reactive) fn messages(
    connection: &Connection,
    room_id: &str,
    before: Option<&str>,
) -> Result<Vec<RoomMessage>, ReactiveError> {
    let limit = i64::try_from(HISTORY_LIMIT)
        .map_err(|_| ReactiveError::Invalid("History page size is invalid.".into()))?;
    let mut statement = connection.prepare("SELECT payload FROM reactive_room_messages WHERE room_id=?1 AND (?2 IS NULL OR rowid<(SELECT rowid FROM reactive_room_messages WHERE id=?2)) ORDER BY rowid DESC LIMIT ?3")?;
    let rows = statement.query_map(params![room_id, before, limit], |row| {
        row.get::<_, String>(0)
    })?;
    rows.map(|row| decode(row?)).collect()
}
