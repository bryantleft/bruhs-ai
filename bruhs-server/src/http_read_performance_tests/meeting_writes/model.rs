use super::*;
use bruhs_models::{
    ActionItem, LocalFocus, LocalFocusObservation, Meeting, MeetingEvent, MeetingStatus,
    MeetingSummary, ObservationKind, PersonObservation, PersonProfile, Speaker, TranscriptSegment,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::RangeInclusive;

const VERSION: i64 = 4_000_000_000_000;

pub(super) fn focus(meeting_id: &str, id: &str, observed_at_ms: i64) -> LocalFocusObservation {
    LocalFocusObservation {
        id: id.into(),
        meeting_id: meeting_id.into(),
        observed_at_ms,
        focus: LocalFocus::Available {
            app_name: "Fixture Ω".into(),
            bundle_id: Some("test.fixture".into()),
        },
    }
}

fn meeting(id: &str, start: u32, shape: &str, references: usize) -> Meeting {
    let mut meeting = Meeting::new("Meeting Ω", f64::from(start));
    meeting.id = id.into();
    meeting.created_at = 42;
    meeting.updated_at = VERSION;
    meeting.language = Some("en".into());
    if matches!(
        shape,
        "repeated-assignees" | "unique-assignees" | "large-person" | "repeated-large-person"
    ) {
        meeting.speakers[1].person_id = Some("p000000".into());
    }
    let action_items = if matches!(
        shape,
        "repeated-assignees" | "unique-assignees" | "repeated-large-person"
    ) {
        (0..references)
            .map(|index| ActionItem {
                text: format!("Action {index:06} 😀"),
                assignee_person_id: Some(if shape == "unique-assignees" {
                    format!("p{index:06}")
                } else {
                    "p000000".into()
                }),
                due_hint: Some("next week".into()),
            })
            .collect()
    } else {
        Vec::new()
    };
    meeting.summary = Some(MeetingSummary {
        tldr: if shape == "large-summary" {
            "Ω".repeat(32768)
        } else {
            "summary \"agenda\"\n😀".into()
        },
        action_items,
        decisions: vec!["preserve full state".into()],
        topics: Vec::new(),
        generated_by: "fixture".into(),
        generated_at: 42,
    });
    meeting
}

pub(super) struct Model {
    pub(super) meetings: BTreeMap<String, Meeting>,
    people: BTreeMap<String, PersonProfile>,
    segments: BTreeMap<String, Vec<TranscriptSegment>>,
    removals: BTreeMap<String, BTreeSet<String>>,
    focus: BTreeMap<String, Vec<LocalFocusObservation>>,
    canonical_ids: BTreeMap<String, String>,
    canonical_version: Option<i64>,
}

impl Model {
    pub(super) fn reference_counts(&self) -> (usize, usize) {
        let meeting = &self.meetings["meeting"];
        let references: Vec<_> = meeting
            .speakers
            .iter()
            .filter_map(|speaker| speaker.person_id.as_ref())
            .chain(meeting.summary.iter().flat_map(|summary| {
                summary
                    .action_items
                    .iter()
                    .filter_map(|item| item.assignee_person_id.as_ref())
            }))
            .collect();
        (
            references.len(),
            references.into_iter().collect::<BTreeSet<_>>().len(),
        )
    }

    pub(super) async fn seed(
        case: Case,
        shape: &str,
        references: usize,
        iterations: usize,
        store: &Arc<dyn MeetingStore>,
    ) -> Self {
        assert!(
            chrono::Utc::now().timestamp_millis() < VERSION,
            "fixture version must stay ahead of real clock"
        );
        let mut people = BTreeMap::new();
        let count = if shape == "unique-assignees" {
            references
        } else {
            1
        };
        for index in 0..count {
            let mut person = PersonProfile::new(format!("Contact {index:06} Ω"));
            person.id = format!("p{index:06}");
            person.created_at = 42;
            person.updated_at = VERSION;
            person.aliases = vec![format!("Alias {index:06} 😀")];
            person.observations = vec![PersonObservation {
                at_meeting_id: "meeting".into(),
                kind: ObservationKind::Quote,
                text: if matches!(shape, "large-person" | "repeated-large-person") {
                    "Ω".repeat(32768)
                } else {
                    "quoted \"agenda\"\r\n😀".into()
                },
                created_at: 42,
            }];
            assert_eq!(
                store
                    .upsert_person(person.clone())
                    .await
                    .expect("seed linked person"),
                person
            );
            people.insert(person.id.clone(), person);
        }
        let mut meetings = BTreeMap::from([(
            "meeting".into(),
            meeting("meeting", 1000, shape, references),
        )]);
        if matches!(case, Case::Delete) {
            for index in 0..iterations {
                let id = format!("delete-{index:06}");
                meetings.insert(
                    id.clone(),
                    meeting(
                        &id,
                        2000 + u32::try_from(index).expect("fixture index"),
                        shape,
                        references,
                    ),
                );
            }
        }
        let mut model = Self {
            meetings,
            people,
            segments: BTreeMap::new(),
            removals: BTreeMap::new(),
            focus: BTreeMap::new(),
            canonical_ids: BTreeMap::new(),
            canonical_version: None,
        };
        for (id, meeting) in &model.meetings {
            assert_eq!(
                store
                    .create_meeting(meeting.clone())
                    .await
                    .expect("seed private meeting"),
                *meeting
            );
            let chunks = if id == "meeting" && matches!(case, Case::Remove) {
                iterations
            } else {
                2
            };
            let mut segments = Vec::new();
            for index in 0..chunks {
                let start = u32::try_from(index).expect("fixture index") * 10;
                let segment = TranscriptSegment {
                    id: if id == "meeting" {
                        format!("chunk-{index:06}")
                    } else {
                        format!("{id}-chunk-{index:06}")
                    },
                    meeting_id: id.clone(),
                    speaker_id: Some("them".into()),
                    start_ms: start,
                    end_ms: start + 9,
                    text: "full retained transcript 😀".into(),
                    confidence: 0.875,
                    partial: false,
                    speaker_confidence: Some(0.75),
                };
                store
                    .append_segment(segment.clone())
                    .await
                    .expect("seed transcript segment");
                segments.push(segment);
            }
            store
                .remove_segment(id, "removed")
                .await
                .expect("seed removed chunk");
            let sample = focus(id, if id == "meeting" { "focus-seed" } else { id }, 1);
            store
                .append_local_focus(sample.clone())
                .await
                .expect("seed local context");
            model.focus.insert(id.clone(), vec![sample]);
            model.segments.insert(id.clone(), segments);
            model
                .removals
                .insert(id.clone(), BTreeSet::from(["removed".into()]));
        }
        model.verify(store).await;
        model
    }

    pub(super) async fn verify(&self, store: &Arc<dyn MeetingStore>) {
        let mut expected: Vec<_> = self.meetings.values().cloned().collect();
        expected.sort_by(|a, b| {
            b.started_at_ms
                .partial_cmp(&a.started_at_ms)
                .expect("finite fixture starts")
        });
        assert_eq!(
            store
                .list_meetings()
                .await
                .expect("complete committed meetings"),
            expected
        );
        assert_eq!(
            store.list_people().await.expect("all linked contacts"),
            self.people.values().cloned().collect::<Vec<_>>()
        );
        for (id, segments) in &self.segments {
            assert_eq!(
                store.list_segments(id).await.expect("retained transcript"),
                *segments
            );
            assert_eq!(
                store
                    .list_local_focus(id)
                    .await
                    .expect("retained local context"),
                self.focus[id]
            );
            if self.meetings.contains_key(id) {
                assert_eq!(
                    store
                        .removed_segment_ids(id)
                        .await
                        .expect("saved chunk removals"),
                    self.removals[id].iter().cloned().collect::<Vec<_>>()
                );
                for removed in &self.removals[id] {
                    assert!(store
                        .segment_removed(id, removed)
                        .await
                        .expect("saved tombstone"));
                }
            } else {
                assert!(
                    matches!(store.removed_segment_ids(id).await,Err(bruhs_core::MeetingStoreError::MeetingNotFound{id:missing}) if &missing==id),
                    "deleted meeting remains missing"
                );
                assert!(!store
                    .segment_removed(id, "removed")
                    .await
                    .expect("removed child tombstone"));
            }
        }
    }

    fn check_uuid(id: &str) {
        let bytes = id.as_bytes();
        assert_eq!(bytes.len(), 36, "new UUID length");
        for (index, byte) in bytes.iter().enumerate() {
            if [8, 13, 18, 23].contains(&index) {
                assert_eq!(*byte, b'-');
            } else {
                assert!(byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase());
            }
        }
        assert_eq!(bytes[14], b'4');
        assert!(matches!(bytes[19], b'8' | b'9' | b'a' | b'b'));
    }

    pub(super) fn apply(
        &mut self,
        case: Case,
        index: usize,
        backend: &str,
        request: &cases::Request,
        actual: Option<Value>,
        clock: RangeInclusive<i64>,
    ) -> (Option<Value>, Option<MeetingEvent>, Vec<Value>) {
        if let Some(error) = request.error {
            let expected = Some(json!({"error":error}));
            assert_eq!(actual, expected, "{} full error response", case.name());
            return (expected, None, Vec::new());
        }
        let mut event = None;
        let mut witnesses = Vec::new();
        let expected = match case {
            Case::Create => {
                let value = actual.as_ref().expect("new meeting JSON");
                let id = value["id"].as_str().expect("new meeting id");
                Self::check_uuid(id);
                assert!(!self.meetings.contains_key(id), "fresh unique id");
                let ordinal = u32::try_from(index).expect("fixture index");
                let created = Meeting {
                    id: id.into(),
                    title: "Created Ω".into(),
                    started_at_ms: f64::from(10000 + ordinal),
                    ended_at_ms: None,
                    status: MeetingStatus::Live,
                    calendar_event_id: Some("fixture-calendar".into()),
                    audio_path: None,
                    language: None,
                    speakers: vec![
                        Speaker {
                            id: "me".into(),
                            label: "Me".into(),
                            person_id: None,
                            color: None,
                        },
                        Speaker {
                            id: "them".into(),
                            label: "Them".into(),
                            person_id: None,
                            color: None,
                        },
                    ],
                    summary: None,
                    thread_id: None,
                    skip_observations: false,
                    created_at: i64::from(10000 + ordinal),
                    updated_at: i64::from(10000 + ordinal),
                };
                let canonical = format!("created-{index:06}");
                witnesses.push(json!({"kind":"created_uuid","actual":id,"canonical":canonical}));
                self.canonical_ids.insert(id.into(), canonical);
                self.segments.insert(id.into(), Vec::new());
                self.removals.insert(id.into(), BTreeSet::new());
                self.focus.insert(id.into(), Vec::new());
                let response = Some(json!(created));
                self.meetings.insert(id.into(), created);
                response
            }
            Case::Delete => {
                let id = format!("delete-{index:06}");
                assert!(self.meetings.remove(&id).is_some());
                self.segments.get_mut(&id).expect("seeded child").clear();
                self.removals.get_mut(&id).expect("seeded child").clear();
                self.focus.get_mut(&id).expect("seeded child").clear();
                None
            }
            Case::Remove | Case::RemoveRepeat => {
                let id = if matches!(case, Case::Remove) {
                    format!("chunk-{index:06}")
                } else {
                    "removed".into()
                };
                self.segments
                    .get_mut("meeting")
                    .expect("subject transcript")
                    .retain(|row| row.id != id);
                self.removals
                    .get_mut("meeting")
                    .expect("subject removals")
                    .insert(id.clone());
                event = Some(MeetingEvent::SegmentRemoved {
                    meeting_id: "meeting".into(),
                    segment_id: id,
                });
                None
            }
            Case::Removals => Some(json!(self.removals["meeting"].iter().collect::<Vec<_>>())),
            Case::FocusList => Some(json!(self.focus["meeting"])),
            Case::FocusAppend | Case::FocusRetry => {
                if matches!(case, Case::FocusAppend) {
                    let sample: LocalFocusObservation =
                        serde_json::from_value(request.body.clone().expect("focus body"))
                            .expect("independent sample");
                    self.focus
                        .get_mut("meeting")
                        .expect("subject focus")
                        .push(sample);
                }
                None
            }
            Case::Patch
            | Case::PatchEmpty
            | Case::Stop
            | Case::Speaker
            | Case::Link
            | Case::LinkNew
            | Case::Skip => {
                let person_name = self.people["p000000"].display_name.clone();
                let meeting = self.meetings.get_mut("meeting").expect("subject meeting");
                match case {
                    Case::Patch => meeting.title = format!("Renamed {index:06} Ω"),
                    Case::PatchEmpty => {}
                    Case::Stop => {
                        meeting.status = MeetingStatus::Ended;
                        meeting.ended_at_ms = Some(f64::from(
                            20000 + u32::try_from(index).expect("fixture index"),
                        ));
                    }
                    Case::Speaker => meeting.speakers[1].label = format!("Label {index:06} 😀"),
                    Case::Link => {
                        meeting.speakers[1].label = person_name;
                        meeting.speakers[1].person_id = Some("p000000".into());
                        event = Some(MeetingEvent::PeopleUpdated);
                    }
                    Case::LinkNew => {
                        meeting.speakers.push(Speaker {
                            id: format!("linked-{index:06}"),
                            label: person_name,
                            person_id: Some("p000000".into()),
                            color: None,
                        });
                        event = Some(MeetingEvent::PeopleUpdated);
                    }
                    Case::Skip => meeting.skip_observations = index.is_multiple_of(2),
                    _ => unreachable!("declared successful meeting edit"),
                }
                if matches!(case, Case::Skip) && backend == "memory" {
                    let stamp = actual.as_ref().expect("skip JSON")["updated_at"]
                        .as_i64()
                        .expect("real clock stamp");
                    assert!(
                        clock.contains(&stamp),
                        "skip update timestamp enclosed by request"
                    );
                    let canonical = VERSION + i64::try_from(index).expect("fixture index") + 1;
                    witnesses.push(json!({"kind":"memory_skip_updated_at","actual":stamp,"not_before":clock.start(),"not_after":clock.end(),"canonical":canonical}));
                    meeting.updated_at = stamp;
                    self.canonical_version = Some(canonical);
                } else {
                    meeting.updated_at += 1;
                }
                Some(json!(meeting))
            }
            _ => unreachable!("error cases returned earlier"),
        };
        assert_eq!(actual, expected, "{} entire ordered response", case.name());
        let mut canonical = expected;
        if matches!(case, Case::Create) {
            let value = canonical.as_mut().expect("new meeting JSON");
            let id = value["id"].as_str().expect("new meeting id").to_owned();
            value["id"] = json!(self.canonical_ids[&id]);
        }
        if matches!(case, Case::Skip) && backend == "memory" {
            canonical.as_mut().expect("skip JSON")["updated_at"] =
                json!(self.canonical_version.expect("checked normalized version"));
        }
        (canonical, event, witnesses)
    }

    fn canonical_id(&self, id: &str) -> String {
        self.canonical_ids
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.into())
    }

    pub(super) fn canonical_state(&self) -> Value {
        let meetings: BTreeMap<_, _> = self
            .meetings
            .iter()
            .map(|(id, meeting)| {
                let mut value = json!(meeting);
                value["id"] = json!(self.canonical_id(id));
                if id == "meeting" {
                    if let Some(version) = self.canonical_version {
                        value["updated_at"] = json!(version);
                    }
                }
                (self.canonical_id(id), value)
            })
            .collect();
        let segments: BTreeMap<_, _> = self
            .segments
            .iter()
            .map(|(id, rows)| {
                let values: Vec<_> = rows
                    .iter()
                    .map(|row| {
                        let mut value = json!(row);
                        value["meeting_id"] = json!(self.canonical_id(id));
                        value
                    })
                    .collect();
                (self.canonical_id(id), values)
            })
            .collect();
        let removals: BTreeMap<_, _> = self
            .removals
            .iter()
            .map(|(id, rows)| (self.canonical_id(id), rows))
            .collect();
        let focus: BTreeMap<_, _> = self
            .focus
            .iter()
            .map(|(id, rows)| {
                let values: Vec<_> = rows
                    .iter()
                    .map(|row| {
                        let mut value = json!(row);
                        value["meeting_id"] = json!(self.canonical_id(id));
                        value
                    })
                    .collect();
                (self.canonical_id(id), values)
            })
            .collect();
        json!({"meetings":meetings,"people":self.people,"segments":segments,"removals":removals,"focus":focus})
    }
}
