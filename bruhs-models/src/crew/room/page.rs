//! The rules and copy a room's page shares on every surface: its transcript
//! lines, the rail's order and status, the `@` list and the settings form.

use super::{check_room, join_names, Room, RoomAuthor, RoomMessage, RoomState, FORMER_BRUH};
use crate::crew::{rooms_ready, service_notice, BruhState};
use crate::reactive::{OutboxItem, ReactiveCommand, ReactiveRun, RunStatus};
use crate::EpochMs;

/// The rail's action and the new room's heading.
pub const NEW_ROOM: &str = "New room";
/// The settings heading.
pub const ROOM_SETTINGS: &str = "Room settings";
/// The lead picker's empty choice.
pub const NO_LEAD: &str = "No lead";
/// Under a room that has members but no lead.
pub const NO_LEAD_HINT: &str = "No lead. Write @ and a name to hand a bruh work.";
/// A room with no lines yet.
pub const EMPTY_ROOM: &str = "Nothing here yet. Write @ and a name to hand a bruh work.";
/// A room with no members, in place of the composer.
pub const NO_MEMBERS: &str = "Add a bruh in Room settings to start.";
/// Asked before a room is deleted.
pub const DELETE_ROOM_CONFIRM: &str = "Delete this room and its messages? The bruhs stay.";
/// Shown on an approval card once the owner has decided it.
pub const APPROVAL_DECIDED: &str = "Decided";
/// The room form's member checklist.
pub const ROOM_MEMBERS: &str = "Members";
/// The room form's lead picker.
pub const ROOM_LEAD: &str = "Lead";
/// Saves a new room.
pub const CREATE_ROOM: &str = "Create room";
/// Saves an existing room's settings.
pub const SAVE_ROOM: &str = "Save";
/// Opens the inline delete confirm in Room settings.
pub const DELETE_ROOM: &str = "Delete room";
/// Confirms [`DELETE_ROOM_CONFIRM`].
pub const DELETE_ROOM_YES: &str = "Delete";
/// Backs out of [`DELETE_ROOM_CONFIRM`].
pub const DELETE_ROOM_NO: &str = "Keep";
/// Why New room is off while the crew is empty.
pub const ADD_A_BRUH_FIRST: &str = "Add a bruh first.";

/// Why New room is off, or `None` when it works. An unknown service
/// version counts as not yet, titled plainly with [`NEW_ROOM`]; an older
/// service gives its update notice; an empty crew asks for a bruh first.
#[must_use]
pub fn new_room_block(protocol: Option<u32>, bruhs: usize) -> Option<&'static str> {
    let Some(protocol) = protocol else {
        return Some(NEW_ROOM);
    };
    if !rooms_ready(protocol) {
        return Some(service_notice(protocol, None).map_or(NEW_ROOM, |notice| notice.message()));
    }
    (bruhs == 0).then_some(ADD_A_BRUH_FIRST)
}

/// The line shown while a bruh has a run in the room.
#[must_use]
pub fn working_line(name: &str) -> String {
    format!("{name} is working")
}

/// One line of a room's transcript as drawn.
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::large_enum_variant,
        reason = "cards are a minority of lines; boxing would change the shared API"
    )
)]
#[derive(Debug, Clone, PartialEq)]
pub enum RoomLine {
    Owner {
        message_id: String,
        text: String,
        at: EpochMs,
    },
    Bruh {
        message_id: String,
        agent_id: String,
        text: String,
        failed: bool,
        at: EpochMs,
    },
    System {
        message_id: String,
        text: String,
    },
    /// A question or approval still in the outbox.
    Card {
        message_id: String,
        agent_id: String,
        item: OutboxItem,
    },
    /// A question or approval whose card is gone from the outbox.
    Asked {
        message_id: String,
        agent_id: String,
        summary: String,
    },
    /// A bruh with a queued or running run in this room.
    Working {
        run_id: String,
        agent_id: String,
    },
}

/// A room's transcript: its messages in `seq` order, then one row per bruh
/// still working in it.
#[must_use]
pub fn room_lines(
    messages: &[RoomMessage],
    runs: &[ReactiveRun],
    outbox: &[OutboxItem],
    room_id: &str,
) -> Vec<RoomLine> {
    let mut ordered: Vec<&RoomMessage> = messages.iter().collect();
    ordered.sort_by_key(|message| message.seq);
    let mut lines: Vec<RoomLine> = ordered
        .into_iter()
        .map(|message| {
            let message_id = message.id.clone();
            match &message.author {
                RoomAuthor::Owner => RoomLine::Owner {
                    message_id,
                    text: message.text.clone(),
                    at: message.created_at,
                },
                RoomAuthor::System => RoomLine::System {
                    message_id,
                    text: message.text.clone(),
                },
                RoomAuthor::Bruh { agent_id } => {
                    let agent_id = agent_id.clone();
                    match &message.outbox_id {
                        Some(outbox_id) => match outbox.iter().find(|item| &item.id == outbox_id) {
                            Some(item) => RoomLine::Card {
                                message_id,
                                agent_id,
                                item: item.clone(),
                            },
                            None => RoomLine::Asked {
                                message_id,
                                agent_id,
                                summary: message.text.clone(),
                            },
                        },
                        None => RoomLine::Bruh {
                            message_id,
                            agent_id,
                            text: message.text.clone(),
                            failed: message.failed,
                            at: message.created_at,
                        },
                    }
                }
            }
        })
        .collect();
    let mut working: Vec<&ReactiveRun> = runs
        .iter()
        .filter(|run| {
            run.trigger.as_ref().and_then(|t| t.room_id()) == Some(room_id)
                && matches!(run.status, RunStatus::Queued | RunStatus::Running)
        })
        .collect();
    working.sort_by_key(|run| run.created_at);
    lines.extend(working.into_iter().map(|run| RoomLine::Working {
        run_id: run.id.clone(),
        agent_id: run.agent_id.clone(),
    }));
    lines
}

/// What identifies a line as drawn: a line is redrawn only when this changes.
#[must_use]
pub fn room_line_key(line: &RoomLine) -> String {
    match line {
        RoomLine::Owner { message_id, .. } => format!("owner:{message_id}"),
        RoomLine::Bruh {
            message_id, failed, ..
        } => format!("bruh:{message_id}:{failed}"),
        RoomLine::System { message_id, .. } => format!("system:{message_id}"),
        RoomLine::Card {
            message_id, item, ..
        } => format!(
            "card:{}:{}:{:?}",
            message_id,
            item.resolved_at.is_some(),
            item.approval.as_ref().map(|approval| approval.state)
        ),
        RoomLine::Asked { message_id, .. } => format!("asked:{message_id}"),
        RoomLine::Working { run_id, .. } => format!("working:{run_id}"),
    }
}

/// Puts a page of messages into those shown, replacing repeats by id, in
/// `seq` order.
pub fn merge_messages(shown: &mut Vec<RoomMessage>, page: Vec<RoomMessage>) {
    for message in page {
        match shown.iter_mut().find(|known| known.id == message.id) {
            Some(known) => *known = message,
            None => shown.push(message),
        }
    }
    shown.sort_by_key(|message| message.seq);
}

/// A bruh's current name, or [`FORMER_BRUH`] once it is deleted.
#[must_use]
pub fn author_name(agent_id: &str, bruhs: &[BruhState]) -> String {
    bruhs
        .iter()
        .find(|bruh| bruh.agent_id == agent_id)
        .map_or_else(|| FORMER_BRUH.to_string(), |bruh| bruh.name.clone())
}

/// The rail's order: name without case, then id.
pub fn room_order(rooms: &mut [RoomState]) {
    rooms.sort_by(|a, b| {
        a.room
            .name
            .to_lowercase()
            .cmp(&b.room.name.to_lowercase())
            .then_with(|| a.room.id.cmp(&b.room.id))
    });
}

/// "Ada, Bo and Cy", or "No members".
#[must_use]
pub fn members_line(room: &Room, bruhs: &[BruhState]) -> String {
    let names: Vec<String> = room
        .members
        .iter()
        .map(|id| author_name(id, bruhs))
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    if names.is_empty() {
        "No members".to_string()
    } else {
        join_names(&names)
    }
}

/// The rail's second line: who is working, else who is in the room.
#[must_use]
pub fn room_status_line(state: &RoomState, bruhs: &[BruhState]) -> String {
    let names: Vec<String> = state
        .working
        .iter()
        .map(|id| author_name(id, bruhs))
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    match names.as_slice() {
        [] => members_line(&state.room, bruhs),
        [one] => working_line(one),
        many => format!("{} are working", join_names(many)),
    }
}

/// An `@` being typed at `caret`: the byte index of the `@` and the text
/// after it. The `@` opens at the start or after whitespace or `(`, and the
/// text after it holds no whitespace.
#[must_use]
pub fn mention_query(text: &str, caret: usize) -> Option<(usize, String)> {
    let before = text.get(..caret)?;
    let at = before.rfind(|c: char| c == '@' || c.is_whitespace())?;
    if !before[at..].starts_with('@') {
        return None;
    }
    let opens = before[..at]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || c == '(');
    opens.then(|| (at, before[at + 1..].to_string()))
}

/// The room's members the query names: name starts with it, then name
/// contains it, each in the room's member order.
#[must_use]
pub fn member_hits<'a>(room: &Room, bruhs: &'a [BruhState], query: &str) -> Vec<&'a BruhState> {
    let query = query.to_lowercase();
    let members: Vec<&BruhState> = room
        .members
        .iter()
        .filter_map(|id| bruhs.iter().find(|bruh| &bruh.agent_id == id))
        .collect();
    let prefix = |bruh: &&BruhState| bruh.name.to_lowercase().starts_with(&query);
    let contains = |bruh: &&BruhState| bruh.name.to_lowercase().contains(&query);
    members
        .iter()
        .copied()
        .filter(prefix)
        .chain(
            members
                .iter()
                .copied()
                .filter(|bruh| !prefix(bruh) && contains(bruh)),
        )
        .collect()
}

/// The room settings form.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomForm {
    pub name: String,
    pub members: Vec<String>,
    pub lead: Option<String>,
}

impl RoomForm {
    #[must_use]
    pub fn from_room(room: &Room) -> Self {
        Self {
            name: room.name.clone(),
            members: room.members.clone(),
            lead: room.lead.clone(),
        }
    }

    /// Adds or removes a member; removing the lead clears it.
    pub fn toggle_member(&mut self, agent_id: &str) {
        if let Some(at) = self.members.iter().position(|id| id == agent_id) {
            self.members.remove(at);
            if self.lead.as_deref() == Some(agent_id) {
                self.lead = None;
            }
        } else {
            self.members.push(agent_id.to_string());
        }
    }
}

/// The form a room dialog opens with: blank for a new room, else the room
/// without members who have left the crew (and a lead who left with them),
/// so an untouched form still saves.
#[must_use]
pub fn room_form_for(room: Option<&Room>, crew: &[BruhState]) -> RoomForm {
    let Some(room) = room else {
        return RoomForm::default();
    };
    let mut form = RoomForm::from_room(room);
    form.members
        .retain(|id| crew.iter().any(|bruh| bruh.agent_id == *id));
    if form
        .lead
        .as_ref()
        .is_some_and(|lead| !form.members.contains(lead))
    {
        form.lead = None;
    }
    form
}

/// The lead picker's choices as (agent id, name): [`NO_LEAD`] with an empty
/// id, then each checked member in crew order.
#[must_use]
pub fn lead_choices(form: &RoomForm, crew: &[BruhState]) -> Vec<(String, String)> {
    std::iter::once((String::new(), NO_LEAD.to_string()))
        .chain(
            crew.iter()
                .filter(|bruh| form.members.contains(&bruh.agent_id))
                .map(|bruh| (bruh.agent_id.clone(), bruh.name.clone())),
        )
        .collect()
}

/// The command a saved form sends: create for a new room, update for a
/// changed one, `None` when nothing changed, or the first rule it breaks.
pub fn room_command(
    existing: Option<&Room>,
    id: &str,
    form: &RoomForm,
    eligible: &[&str],
) -> Result<Option<ReactiveCommand>, &'static str> {
    let room = Room {
        id: id.to_string(),
        name: form.name.trim().to_string(),
        members: form.members.clone(),
        lead: form.lead.clone(),
    };
    check_room(&room, eligible)?;
    Ok(match existing {
        Some(current) if *current == room => None,
        Some(_) => Some(ReactiveCommand::UpdateRoom { room }),
        None => Some(ReactiveCommand::CreateRoom { room }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crew::BruhStatus;
    use crate::reactive::{OutboxKind, RunTrigger};

    fn state(id: &str, name: &str) -> BruhState {
        BruhState {
            agent_id: id.into(),
            name: name.into(),
            look: Default::default(),
            access: Default::default(),
            effort: None,
            status: BruhStatus::Asleep,
            next_wake_ms: None,
            unread: 0,
        }
    }

    fn msg(seq: u64, author: RoomAuthor, text: &str) -> RoomMessage {
        RoomMessage {
            id: format!("m{seq}"),
            room_id: "r".into(),
            seq,
            author,
            text: text.into(),
            run_id: None,
            outbox_id: None,
            failed: false,
            created_at: EpochMs::saturating_from_i64(i64::try_from(seq).unwrap()),
        }
    }

    fn bruh(id: &str) -> RoomAuthor {
        RoomAuthor::Bruh {
            agent_id: id.into(),
        }
    }

    fn run(id: &str, agent: &str, status: &str, created: i64, room: &str) -> ReactiveRun {
        let mut run: ReactiveRun = serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": agent, "source_id": null, "subscription_id": null,
            "event_id": null, "status": status, "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": created, "updated_at": created,
            "not_before": created, "result": null, "error": null,
        }))
        .unwrap();
        run.trigger = Some(RunTrigger::Room {
            room_id: room.into(),
            message_id: "m1".into(),
            hop: 0,
            resumed: false,
        });
        run
    }

    fn item(id: &str) -> OutboxItem {
        let mut item: OutboxItem = serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "a", "run_id": "r", "kind": OutboxKind::Ask,
            "title": "t", "message": "m", "created_at": 1,
        }))
        .unwrap();
        item.approval = Some(crate::crew::ApprovalRequest {
            tool: "Bash".into(),
            summary: "Run ls".into(),
            fingerprint: "f".into(),
            state: crate::crew::ApprovalState::Waiting,
            detail: None,
        });
        item
    }

    #[test]
    fn lines_follow_seq_with_cards_working_rows_and_system_lines() {
        let mut waiting = msg(4, bruh("a"), "Run ls?");
        waiting.outbox_id = Some("q1".into());
        let mut gone = msg(5, bruh("a"), "Run pwd?");
        gone.outbox_id = Some("gone".into());
        // Out of order on purpose: lines follow seq.
        let messages = [
            msg(2, bruh("a"), "On it"),
            msg(1, RoomAuthor::Owner, "@Ada go"),
            msg(
                3,
                RoomAuthor::System,
                "Bo is paused, so it did not get this message.",
            ),
            waiting,
            gone,
        ];
        let runs = [
            run("run1", "a", "running", 5, "r"),
            run("run2", "b", "running", 6, "other"),
            run("run3", "b", "completed", 7, "r"),
        ];
        let lines = room_lines(&messages, &runs, &[item("q1")], "r");
        assert!(matches!(&lines[0], RoomLine::Owner { text, .. } if text == "@Ada go"));
        assert!(matches!(&lines[1], RoomLine::Bruh { text, .. } if text == "On it"));
        assert!(matches!(&lines[2], RoomLine::System { .. }));
        assert!(matches!(&lines[3], RoomLine::Card { item, .. } if item.id == "q1"));
        assert!(matches!(&lines[4], RoomLine::Asked { summary, .. } if !summary.is_empty()));
        assert!(
            matches!(lines.last(), Some(RoomLine::Working { agent_id, .. }) if agent_id == "a")
        );
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn lines_name_bruhs_by_their_current_name_or_former_bruh() {
        assert_eq!(
            author_name("a", &[state("a", "Ada Renamed")]),
            "Ada Renamed"
        );
        assert_eq!(author_name("gone", &[state("a", "Ada")]), FORMER_BRUH);
    }

    #[test]
    fn line_keys_change_only_when_a_line_is_redrawn() {
        let line = |failed| RoomLine::Bruh {
            message_id: "m1".into(),
            agent_id: "a".into(),
            text: "hi".into(),
            failed,
            at: EpochMs::saturating_from_i64(1),
        };
        assert_eq!(room_line_key(&line(false)), room_line_key(&line(false)));
        assert_ne!(room_line_key(&line(false)), room_line_key(&line(true)));
        let card = |resolved: bool, state: crate::crew::ApprovalState| {
            let mut open = item("q1");
            if resolved {
                open.resolved_at = Some(EpochMs::saturating_from_i64(9));
            }
            open.approval.as_mut().unwrap().state = state;
            RoomLine::Card {
                message_id: "m4".into(),
                agent_id: "a".into(),
                item: open,
            }
        };
        let waiting = room_line_key(&card(false, crate::crew::ApprovalState::Waiting));
        assert_eq!(
            waiting,
            room_line_key(&card(false, crate::crew::ApprovalState::Waiting))
        );
        assert_ne!(
            waiting,
            room_line_key(&card(true, crate::crew::ApprovalState::Approved))
        );
        let working = RoomLine::Working {
            run_id: "run1".into(),
            agent_id: "a".into(),
        };
        assert_eq!(room_line_key(&working), "working:run1");
    }

    #[test]
    fn a_newer_page_merges_in_seq_order_without_repeats() {
        let mut shown = vec![
            msg(3, RoomAuthor::Owner, "c"),
            msg(4, RoomAuthor::Owner, "d"),
        ];
        let mut edited = msg(4, RoomAuthor::Owner, "d");
        edited.failed = true;
        merge_messages(
            &mut shown,
            vec![
                msg(1, RoomAuthor::Owner, "a"),
                msg(2, RoomAuthor::Owner, "b"),
                edited,
            ],
        );
        let seqs: Vec<u64> = shown.iter().map(|m| m.seq).collect();
        assert_eq!(seqs, [1, 2, 3, 4]);
        assert!(shown[3].failed);
    }

    fn rs(id: &str, name: &str, members: &[&str], working: &[&str]) -> RoomState {
        RoomState {
            room: Room {
                id: id.into(),
                name: name.into(),
                members: members.iter().map(|m| (*m).to_string()).collect(),
                lead: None,
            },
            last_seq: 0,
            working: working.iter().map(|m| (*m).to_string()).collect(),
        }
    }

    #[test]
    fn the_rail_orders_rooms_by_name_and_says_who_is_working() {
        let mut rooms = [rs("1", "beta", &[], &[]), rs("2", "Alpha", &[], &[])];
        room_order(&mut rooms);
        assert_eq!(rooms[0].room.name, "Alpha");
        let bruhs = [state("a", "Ada"), state("b", "Bo"), state("c", "Cy")];
        let room = rs("3", "Launch", &["a", "b", "c"], &[]);
        assert_eq!(members_line(&room.room, &bruhs), "Ada, Bo and Cy");
        assert_eq!(
            members_line(&rs("4", "Empty", &[], &[]).room, &bruhs),
            "No members"
        );
        assert_eq!(room_status_line(&room, &bruhs), "Ada, Bo and Cy");
        let one = rs("3", "Launch", &["a", "b", "c"], &["b"]);
        assert_eq!(room_status_line(&one, &bruhs), "Bo is working");
        let two = rs("3", "Launch", &["a", "b", "c"], &["a", "b"]);
        assert_eq!(room_status_line(&two, &bruhs), "Ada and Bo are working");
    }

    #[test]
    fn at_lists_room_members_prefix_first_then_contains() {
        assert_eq!(mention_query("hi @bo", 6), Some((3, "bo".into())));
        assert_eq!(mention_query("@", 1), Some((0, String::new())));
        assert_eq!(mention_query("mail@bo", 7), None);
        assert_eq!(mention_query("@bo now", 7), None);
        let bruhs = [
            state("a", "Abo"),
            state("b", "Bobby"),
            state("c", "Cy"),
            state("d", "Bob"),
        ];
        let room = Room {
            id: "r".into(),
            name: "R".into(),
            members: vec!["c".into(), "b".into(), "a".into()],
            lead: None,
        };
        let names = |query: &str| -> Vec<&str> {
            member_hits(&room, &bruhs, query)
                .into_iter()
                .map(|b| b.name.as_str())
                .collect()
        };
        assert_eq!(names("bo"), ["Bobby", "Abo"]);
        assert_eq!(names(""), ["Cy", "Bobby", "Abo"]);
    }

    #[test]
    fn new_room_is_off_until_the_service_takes_rooms_and_there_is_a_bruh() {
        use crate::crew::{ServiceNotice, CREW_PROTOCOL};
        assert_eq!(
            new_room_block(Some(1), 2),
            Some(ServiceNotice::ServingOld.message())
        );
        assert_eq!(
            new_room_block(Some(CREW_PROTOCOL), 0),
            Some("Add a bruh first.")
        );
        assert_eq!(new_room_block(Some(CREW_PROTOCOL), 1), None);
        // Unknown: off, titled plainly.
        assert_eq!(new_room_block(None, 1), Some("New room"));
    }

    #[test]
    fn the_room_form_creates_updates_or_does_nothing() {
        let mut form = RoomForm {
            name: "Launch".into(),
            members: vec!["a".into(), "b".into()],
            lead: Some("a".into()),
        };
        form.toggle_member("a");
        assert_eq!(form.lead, None);
        assert_eq!(form.members, ["b"]);
        form.toggle_member("a");
        assert_eq!(form.members, ["b", "a"]);
        let created = room_command(None, "r", &form, &["a", "b"]).unwrap();
        assert!(matches!(created, Some(ReactiveCommand::CreateRoom { .. })));
        let room = Room {
            id: "r".into(),
            name: "Launch".into(),
            members: vec!["b".into()],
            lead: None,
        };
        assert_eq!(
            room_command(Some(&room), "r", &RoomForm::from_room(&room), &["b"]),
            Ok(None)
        );
        let renamed = RoomForm {
            name: "Ship".into(),
            ..RoomForm::from_room(&room)
        };
        assert!(matches!(
            room_command(Some(&room), "r", &renamed, &["b"]),
            Ok(Some(ReactiveCommand::UpdateRoom { .. }))
        ));
        assert_eq!(
            room_command(
                None,
                "r",
                &RoomForm {
                    name: String::new(),
                    ..form
                },
                &["b"]
            ),
            Err("Name the room.")
        );
    }

    #[test]
    fn the_lead_list_follows_the_checked_members() {
        let crew = [state("a", "Ada"), state("b", "Bo"), state("c", "Cy")];
        // Checked out of crew order on purpose: the list keeps crew order.
        let form = RoomForm {
            name: "Launch".into(),
            members: vec!["c".into(), "a".into()],
            lead: None,
        };
        assert_eq!(
            lead_choices(&form, &crew),
            [
                (String::new(), NO_LEAD.to_string()),
                ("a".into(), "Ada".into()),
                ("c".into(), "Cy".into()),
            ]
        );
        assert_eq!(lead_choices(&RoomForm::default(), &crew).len(), 1);
    }

    #[test]
    fn a_room_opens_without_members_who_left_the_crew() {
        let crew = [state("a", "Ada"), state("b", "Bo")];
        assert_eq!(room_form_for(None, &crew), RoomForm::default());
        let room = Room {
            id: "r".into(),
            name: "Launch".into(),
            members: vec!["gone".into(), "b".into()],
            lead: Some("gone".into()),
        };
        let form = room_form_for(Some(&room), &crew);
        assert_eq!(form.members, ["b"]);
        assert_eq!(form.lead, None);
        // What is left saves, where the room as stored would not.
        let eligible = ["a", "b"];
        assert!(room_command(Some(&room), "r", &form, &eligible).is_ok());
        assert!(room_command(Some(&room), "r", &RoomForm::from_room(&room), &eligible).is_err());
        let kept = Room {
            lead: Some("b".into()),
            ..room
        };
        assert_eq!(room_form_for(Some(&kept), &crew).lead.as_deref(), Some("b"));
    }
}
