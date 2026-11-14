use super::*;
use bruhs_core::meetings::{GenerationOutcome, GenerationReservation, SummaryUpdate};
use bruhs_models::{Meeting, MeetingSummary, TranscriptSegment};
use std::collections::BTreeMap;

pub(super) fn summary(large: bool) -> MeetingSummary {
    MeetingSummary {
        tldr: if large {
            "Ω".repeat(32768)
        } else {
            "Quoted \"agenda\"\r\n😀".into()
        },
        action_items: Vec::new(),
        decisions: vec!["Keep the complete receipt".into()],
        topics: Vec::new(),
        generated_by: "private fixture".into(),
        generated_at: 42,
    }
}

pub(super) struct Model {
    meetings: BTreeMap<String, Meeting>,
    receipts: BTreeMap<String, SummaryGenerationReceipt>,
    segments: BTreeMap<String, Vec<TranscriptSegment>>,
}

impl Model {
    async fn meeting(&mut self, id: &str, store: &Arc<dyn MeetingStore>) {
        let mut meeting = Meeting::new(
            id,
            f64::from(u32::try_from(self.meetings.len()).expect("meeting count")),
        );
        meeting.id = id.into();
        meeting.created_at = 42;
        meeting.updated_at = 4_000_000_000_000;
        assert_eq!(
            store
                .create_meeting(meeting.clone())
                .await
                .expect("private meeting"),
            meeting
        );
        self.meetings.insert(id.into(), meeting);
        self.segments.insert(id.into(), Vec::new());
    }

    fn request(meeting: &str, request: &str) -> SummaryGenerationRequest {
        SummaryGenerationRequest {
            request_id: request.into(),
            meeting_id: meeting.into(),
            extract_observations: true,
            replace_observations: false,
        }
    }

    pub(super) async fn seed(store: &Arc<dyn MeetingStore>, history: usize, large: bool) -> Self {
        let mut model = Self {
            meetings: BTreeMap::new(),
            receipts: BTreeMap::new(),
            segments: BTreeMap::new(),
        };
        model.meeting("background", store).await;
        for index in 0..history {
            let request = Self::request("background", &format!("background-{index:06}"));
            let expected = SummaryGenerationReceipt {
                request: request.clone(),
                state: SummaryGenerationState::Cancelled,
            };
            assert_eq!(
                store
                    .fence_summary_generation(request)
                    .await
                    .expect("private history receipt"),
                expected
            );
            model
                .receipts
                .insert(expected.request.request_id.clone(), expected);
        }
        for case in Case::ALL {
            let (meeting_id, request_id) = case.identity(0);
            model.meeting(&meeting_id, store).await;
            if let Some(mut state) = case.seed_state() {
                let request = Self::request(&meeting_id, &request_id);
                assert!(matches!(
                    store
                        .begin_summary_generation(request.clone())
                        .await
                        .expect("seed receipt"),
                    GenerationReservation::Started
                ));
                match &mut state {
                    SummaryGenerationState::Failed => {
                        store
                            .finish_summary_generation(
                                &meeting_id,
                                &request_id,
                                GenerationOutcome::Failed,
                            )
                            .await
                            .expect("seed failed");
                    }
                    SummaryGenerationState::Cancelled => {
                        store
                            .finish_summary_generation(
                                &meeting_id,
                                &request_id,
                                GenerationOutcome::Cancelled,
                            )
                            .await
                            .expect("seed cancelled");
                    }
                    SummaryGenerationState::Succeeded { summary: saved } => {
                        *saved = summary(large);
                        let segment = TranscriptSegment {
                            id: format!("{meeting_id}-segment"),
                            meeting_id: meeting_id.clone(),
                            speaker_id: Some("me".into()),
                            start_ms: 0,
                            end_ms: 10,
                            text: "Complete Ω 😀".into(),
                            confidence: 1.0,
                            partial: false,
                            speaker_confidence: None,
                        };
                        store
                            .append_segment(segment.clone())
                            .await
                            .expect("seed complete transcript");
                        let meeting = model.meetings.get_mut(&meeting_id).expect("seeded meeting");
                        let updated = store
                            .commit_summary_generation(
                                &request_id,
                                SummaryUpdate {
                                    expected: meeting.clone(),
                                    people_versions: BTreeMap::new(),
                                    transcript: vec![segment.clone()],
                                    summary: saved.clone(),
                                    observations: Default::default(),
                                    replace_observations: false,
                                },
                            )
                            .await
                            .expect("seed committed summary without provider");
                        meeting.summary = Some(saved.clone());
                        meeting.updated_at += 1;
                        assert_eq!(updated, *meeting);
                        model.segments.insert(meeting_id.clone(), vec![segment]);
                    }
                    SummaryGenerationState::Pending => {}
                }
                model
                    .receipts
                    .insert(request_id, SummaryGenerationReceipt { request, state });
            }
        }
        model.verify(store).await;
        model
    }

    pub(super) async fn prepare(
        &mut self,
        case: Case,
        index: usize,
        store: &Arc<dyn MeetingStore>,
    ) {
        if matches!(case, Case::CancelPending | Case::FencePending) {
            let (meeting, id) = case.identity(index);
            let request = Self::request(&meeting, &id);
            assert!(matches!(
                store
                    .begin_summary_generation(request.clone())
                    .await
                    .expect("fresh pending receipt"),
                GenerationReservation::Started
            ));
            self.receipts.insert(
                id,
                SummaryGenerationReceipt {
                    request,
                    state: SummaryGenerationState::Pending,
                },
            );
        }
    }

    pub(super) fn apply(&mut self, case: Case, index: usize) -> (u16, Value) {
        if let Some((status, message)) = case.error() {
            return (status, json!({"error":message}));
        }
        let (meeting, id) = case.identity(index);
        if matches!(case, Case::FenceFresh) {
            let request = Self::request(&meeting, &id);
            self.receipts.insert(
                id.clone(),
                SummaryGenerationReceipt {
                    request,
                    state: SummaryGenerationState::Cancelled,
                },
            );
        } else if matches!(case, Case::CancelPending | Case::FencePending) {
            self.receipts
                .get_mut(&id)
                .expect("fresh pending identity")
                .state = SummaryGenerationState::Cancelled;
        }
        let response = match case {
            Case::PendingNone => Value::Null,
            Case::ReplaySucceeded => match &self.receipts[&id].state {
                SummaryGenerationState::Succeeded { summary } => json!(summary),
                _ => unreachable!("successful replay seeded"),
            },
            _ => json!(self.receipts[&id]),
        };
        (200, response)
    }

    pub(super) async fn verify_subject(
        &self,
        case: Case,
        index: usize,
        store: &Arc<dyn MeetingStore>,
    ) {
        let (meeting, id) = case.identity(index);
        assert_eq!(
            store
                .get_meeting(&meeting)
                .await
                .expect("unchanged subject meeting")
                .as_ref(),
            self.meetings.get(&meeting)
        );
        assert_eq!(
            store
                .summary_generation(&meeting, &id)
                .await
                .expect("subject receipt")
                .as_ref(),
            self.receipts.get(&id)
        );
        let pending = self
            .receipts
            .values()
            .find(|row| {
                row.request.meeting_id == meeting
                    && matches!(row.state, SummaryGenerationState::Pending)
            })
            .cloned();
        assert_eq!(
            store
                .pending_summary_generation(&meeting)
                .await
                .expect("pending receipt"),
            pending
        );
    }

    pub(super) async fn verify(&self, store: &Arc<dyn MeetingStore>) {
        let mut meetings: Vec<_> = self.meetings.values().cloned().collect();
        meetings.sort_by(|a, b| {
            b.started_at_ms
                .partial_cmp(&a.started_at_ms)
                .expect("finite start")
        });
        assert_eq!(
            store
                .list_meetings()
                .await
                .expect("complete ordered meetings"),
            meetings
        );
        assert!(store
            .list_people()
            .await
            .expect("no fixture people")
            .is_empty());
        for (meeting, segments) in &self.segments {
            assert_eq!(
                store
                    .list_segments(meeting)
                    .await
                    .expect("complete retained transcript"),
                *segments
            );
            assert!(store
                .removed_segment_ids(meeting)
                .await
                .expect("no removals")
                .is_empty());
            assert!(store
                .list_local_focus(meeting)
                .await
                .expect("no local context")
                .is_empty());
            let pending = self
                .receipts
                .values()
                .find(|row| {
                    row.request.meeting_id == *meeting
                        && matches!(row.state, SummaryGenerationState::Pending)
                })
                .cloned();
            assert_eq!(
                store
                    .pending_summary_generation(meeting)
                    .await
                    .expect("all pending receipts"),
                pending
            );
        }
        for (id, receipt) in &self.receipts {
            assert_eq!(
                store
                    .summary_generation(&receipt.request.meeting_id, id)
                    .await
                    .expect("complete receipt")
                    .as_ref(),
                Some(receipt)
            );
        }
    }

    pub(super) fn fingerprint(&self) -> String {
        fingerprint(
            &json!({"meetings":self.meetings,"receipts":self.receipts,"segments":self.segments,"people":[],"removals":[],"local_focus":[]}),
        )
    }

    pub(super) fn receipts(&self) -> usize {
        self.receipts.len()
    }
}
