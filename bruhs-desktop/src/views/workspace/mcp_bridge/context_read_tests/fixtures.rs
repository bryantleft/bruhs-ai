//! Synthetic context records; construction and entity mounting are outside timers.
use super::{Workspace, CHAT_ID, MEETING_ID, NOTE_ID, PERSON_ID};
use bruhs_models::*;
use gpui::{Context, Window};

pub(super) fn body(kind: &str, index: usize) -> String {
    format!("{kind} {index}: ΟΣ Café İ K 👩🏽‍💻 \"quoted\" \\ body\nline ").repeat(2)
}

pub(super) fn populate(
    w: &mut Workspace,
    group: &str,
    records: usize,
    identities: usize,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) {
    w.meeting_notes.clear();
    w.relationships.clear();
    w.meetings_hub.update(cx, |hub, _| {
        hub.meetings.clear();
        hub.segments.clear();
        hub.people.clear();
    });
    for domain in [
        AgentDomain::Meetings,
        AgentDomain::People,
        AgentDomain::Chat,
    ] {
        w.automation_state
            .permissions
            .set(domain, AgentAccess::Read, true);
    }
    if group == "meetings" {
        w.meetings_hub.update(cx, |hub, _| {
            for index in 0..identities {
                let now = i64::try_from(index).expect("fixture timestamp");
                let mut meeting = Meeting::new(format!("Meeting ΟΣ {index}"), 1.0);
                meeting.id = if index == identities - 1 {
                    MEETING_ID.into()
                } else {
                    format!("meeting-{index}")
                };
                meeting.created_at = now;
                meeting.updated_at = now;
                if index == identities - 1 {
                    meeting.audio_path = Some("/synthetic/unshared-secret.wav".into());
                    meeting.summary = Some(MeetingSummary {
                        tldr: body("Summary", 0),
                        action_items: vec![],
                        decisions: vec!["Retain context".into()],
                        topics: vec![],
                        generated_by: "fixture".into(),
                        generated_at: 42,
                    });
                    meeting.speakers = vec![Speaker {
                        id: "speaker-context".into(),
                        label: "Speaker ΟΣ".into(),
                        person_id: None,
                        color: Some("pandan".into()),
                    }];
                }
                hub.meetings.push(meeting);
            }
            hub.segments.insert(
                MEETING_ID.into(),
                (0..records)
                    .map(|index| {
                        let start = u32::try_from(index).expect("fixture timestamp");
                        TranscriptSegment {
                            id: format!("segment-{index}"),
                            meeting_id: MEETING_ID.into(),
                            speaker_id: Some("speaker-context".into()),
                            start_ms: start,
                            end_ms: start + 1,
                            text: body("Words", index),
                            confidence: 0.9,
                            partial: index == records - 1,
                            speaker_confidence: Some(0.75),
                        }
                    })
                    .collect(),
            );
        });
        w.meeting_notes = (0..identities)
            .map(|index| {
                let now = i64::try_from(index).expect("fixture timestamp");
                MeetingNote {
                    id: if index == identities - 1 {
                        NOTE_ID.into()
                    } else {
                        format!("note-{index}")
                    },
                    title: format!("Note ΟΣ {index}"),
                    body: if index == identities - 1 {
                        body("Note", 0)
                    } else {
                        "unshared-secret".into()
                    },
                    created_at: now,
                    updated_at: now,
                    recording_id: Some("synthetic-recording".into()),
                }
            })
            .collect();
    } else if group == "people" {
        w.meetings_hub.update(cx, |hub, _| {
            for index in 0..identities {
                let now = i64::try_from(index).expect("fixture timestamp");
                let mut person = PersonProfile::new(format!("Person ΟΣ {index}"));
                person.id = if index == identities - 1 {
                    PERSON_ID.into()
                } else {
                    format!("person-{index}")
                };
                person.created_at = now;
                person.updated_at = now;
                person.aliases = vec![format!("Alias {index}")];
                let mut profile = RelationshipProfile::new(&person.id, 7);
                if index == identities - 1 {
                    person.voice_embedding = Some(vec![0.125; 512]);
                    person.observations = (0..records)
                        .map(|n| PersonObservation {
                            at_meeting_id: format!("meeting-{n}"),
                            kind: ObservationKind::Quote,
                            text: body("Observation", n),
                            created_at: i64::try_from(n).expect("fixture timestamp"),
                        })
                        .collect();
                    profile.company = "Synthetic company".into();
                    profile.email = "person@example.invalid".into();
                    profile.notes = body("Relationship profile", 0);
                    profile.last_contacted_at_ms = Some(6);
                    profile.items = (0..records)
                        .map(|n| {
                            let now = i64::try_from(n).expect("fixture timestamp");
                            let mut item = RelationshipItem::new(
                                RelationshipItemKind::Reminder,
                                body("Relationship", n),
                                now,
                            );
                            item.id = format!("item-{n}");
                            item.due_at_ms = Some(now + 100);
                            item.completed_at_ms = n.is_multiple_of(2).then_some(now + 200);
                            item.source_record_id = Some(format!("source-{n}"));
                            item
                        })
                        .collect();
                }
                hub.people.push(person.into());
                w.relationships.push(profile);
            }
        });
    } else if group == "chat" {
        w.threads.clear();
        for index in 0..identities {
            let mut thread = Workspace::create_thread_with_title(
                "Fixture project".into(),
                String::new(),
                "fixture-model".into(),
                w.permission_mode,
                &format!("Thread ΟΣ {index}"),
                window,
                cx,
            );
            thread.id = if index == identities - 1 {
                CHAT_ID.into()
            } else {
                format!("thread-{index}")
            };
            thread.chat.update(cx, |chat, _| {
                chat.thread_id = Some(thread.id.clone());
                if index == identities - 1 {
                    use crate::views::chat::{DesktopToolCall, Message, TurnState};
                    chat.messages = (0..records)
                        .map(|n| {
                            let now = i64::try_from(n).expect("fixture timestamp");
                            let mut message = Message::user(body("Completed", n));
                            message.started_at_ms = Some(now);
                            message.completed_at_ms = Some(now + 1);
                            message.checkpoint = Some("unshared-secret-checkpoint".into());
                            message
                        })
                        .collect();
                    let mut last = Message::assistant_streaming(33, Some("fixture-model".into()));
                    last.content = body("Completed", records - 1);
                    last.thinking = "Stored synthetic reasoning".into();
                    last.completed_at_ms = Some(66);
                    last.duration_ms = Some(33);
                    last.state = TurnState::Failed {
                        error: "Stored synthetic interruption".into(),
                    };
                    last.tool_calls = vec![DesktopToolCall {
                        id: "stored-tool".into(),
                        tool: "read".into(),
                        input: serde_json::json!({"path":"synthetic"}),
                        output: Some("Stored result ΟΣ".into()),
                        is_error: false,
                    }];
                    chat.messages[records - 1] = last;
                    for position in [1, records / 2, records + 2] {
                        let mut streaming = Message::assistant_streaming(99, None);
                        streaming.content = "unshared-secret-stream".into();
                        chat.messages.insert(position, streaming);
                    }
                }
            });
            w.threads.push(thread);
        }
    } else {
        unreachable!()
    }
}
