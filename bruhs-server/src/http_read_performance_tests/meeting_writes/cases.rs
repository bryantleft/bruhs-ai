use super::*;

#[derive(Clone, Copy)]
pub(super) enum Case {
    Create,
    CreateEmpty,
    Patch,
    PatchEmpty,
    PatchMissing,
    Stop,
    StopMissing,
    Speaker,
    SpeakerMissing,
    SpeakerMeetingMissing,
    Link,
    LinkNew,
    LinkPersonMissing,
    LinkMeetingMissing,
    Skip,
    SkipMissing,
    Delete,
    DeleteMissing,
    Remove,
    RemoveRepeat,
    RemoveMissing,
    RemoveInvalid,
    Removals,
    RemovalsMissing,
    FocusList,
    FocusDenied,
    FocusAppend,
    FocusRetry,
    FocusMismatch,
    FocusMissing,
    FocusAppendDenied,
}

pub(super) struct Request {
    pub(super) method: Method,
    pub(super) path: String,
    pub(super) body: Option<Value>,
    pub(super) native: bool,
    pub(super) status: u16,
    pub(super) error: Option<&'static str>,
}

impl Case {
    pub(super) const ALL: [Self; 31] = [
        Self::Create,
        Self::CreateEmpty,
        Self::Patch,
        Self::PatchEmpty,
        Self::PatchMissing,
        Self::Stop,
        Self::StopMissing,
        Self::Speaker,
        Self::SpeakerMissing,
        Self::SpeakerMeetingMissing,
        Self::Link,
        Self::LinkNew,
        Self::LinkPersonMissing,
        Self::LinkMeetingMissing,
        Self::Skip,
        Self::SkipMissing,
        Self::Delete,
        Self::DeleteMissing,
        Self::Remove,
        Self::RemoveRepeat,
        Self::RemoveMissing,
        Self::RemoveInvalid,
        Self::Removals,
        Self::RemovalsMissing,
        Self::FocusList,
        Self::FocusDenied,
        Self::FocusAppend,
        Self::FocusRetry,
        Self::FocusMismatch,
        Self::FocusMissing,
        Self::FocusAppendDenied,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Create => "create-fresh",
            Self::CreateEmpty => "create-empty",
            Self::Patch => "patch-title",
            Self::PatchEmpty => "patch-empty",
            Self::PatchMissing => "patch-missing",
            Self::Stop => "stop",
            Self::StopMissing => "stop-missing",
            Self::Speaker => "speaker-label",
            Self::SpeakerMissing => "speaker-missing",
            Self::SpeakerMeetingMissing => "speaker-meeting-missing",
            Self::Link => "link-existing-speaker",
            Self::LinkNew => "link-new-speaker",
            Self::LinkPersonMissing => "link-person-missing",
            Self::LinkMeetingMissing => "link-meeting-missing",
            Self::Skip => "skip-toggle",
            Self::SkipMissing => "skip-missing",
            Self::Delete => "delete-fresh",
            Self::DeleteMissing => "delete-missing",
            Self::Remove => "remove-fresh",
            Self::RemoveRepeat => "remove-retry",
            Self::RemoveMissing => "remove-missing",
            Self::RemoveInvalid => "remove-invalid",
            Self::Removals => "removals-list",
            Self::RemovalsMissing => "removals-missing",
            Self::FocusList => "focus-list",
            Self::FocusDenied => "focus-read-denied",
            Self::FocusAppend => "focus-append",
            Self::FocusRetry => "focus-retry",
            Self::FocusMismatch => "focus-mismatch",
            Self::FocusMissing => "focus-missing",
            Self::FocusAppendDenied => "focus-write-denied",
        }
    }

    pub(super) fn action(self) -> &'static str {
        match self {
            Self::Create | Self::CreateEmpty => "POST /api/meetings",
            Self::Patch | Self::PatchEmpty | Self::PatchMissing => "PATCH /api/meetings/{id}",
            Self::Stop | Self::StopMissing => "POST /api/meetings/{id}/stop",
            Self::Speaker | Self::SpeakerMissing | Self::SpeakerMeetingMissing => {
                "PATCH /api/meetings/{id}/speakers/{speaker_id}"
            }
            Self::Link | Self::LinkNew | Self::LinkPersonMissing | Self::LinkMeetingMissing => {
                "POST /api/meetings/{id}/speakers/{speaker_id}/link/{person_id}"
            }
            Self::Skip | Self::SkipMissing => "PATCH /api/meetings/{id}/skip_observations",
            Self::Delete | Self::DeleteMissing => "DELETE /api/meetings/{id}",
            Self::Remove | Self::RemoveRepeat | Self::RemoveMissing | Self::RemoveInvalid => {
                "DELETE /api/meetings/{id}/segments/{segment_id}"
            }
            Self::Removals | Self::RemovalsMissing => "GET /api/meetings/{id}/segment-removals",
            Self::FocusList | Self::FocusDenied => "GET /api/meetings/{id}/local-focus",
            Self::FocusAppend
            | Self::FocusRetry
            | Self::FocusMismatch
            | Self::FocusMissing
            | Self::FocusAppendDenied => "POST /api/meetings/{id}/local-focus",
        }
    }

    pub(super) fn request(self, index: usize) -> Request {
        let mut request = Request {
            method: Method::PATCH,
            path: "/api/meetings/meeting".into(),
            body: None,
            native: false,
            status: 200,
            error: None,
        };
        match self {
            Self::Create | Self::CreateEmpty => {
                request.method = Method::POST;
                request.path = "/api/meetings".into();
                request.body = Some(
                    json!({"title":if matches!(self,Self::Create) { "  Created Ω  " } else { " " },"started_at_ms":10000+index,"calendar_event_id":"fixture-calendar"}),
                );
                if matches!(self, Self::CreateEmpty) {
                    request.status = 400;
                    request.error = Some("title must not be empty");
                }
            }
            Self::Patch | Self::PatchEmpty | Self::PatchMissing => {
                request.body = Some(if matches!(self, Self::PatchEmpty) {
                    json!({})
                } else {
                    json!({"title":format!("Renamed {index:06} Ω")})
                });
                if matches!(self, Self::PatchMissing) {
                    request.path = "/api/meetings/missing".into();
                }
            }
            Self::Stop | Self::StopMissing => {
                request.method = Method::POST;
                request.path = format!(
                    "/api/meetings/{}/stop",
                    if matches!(self, Self::Stop) {
                        "meeting"
                    } else {
                        "missing"
                    }
                );
                request.body = Some(json!({"ended_at_ms":20000+index}));
            }
            Self::Speaker | Self::SpeakerMissing | Self::SpeakerMeetingMissing => {
                request.path = format!(
                    "/api/meetings/{}/speakers/{}",
                    if matches!(self, Self::SpeakerMeetingMissing) {
                        "missing"
                    } else {
                        "meeting"
                    },
                    if matches!(self, Self::SpeakerMissing) {
                        "missing"
                    } else {
                        "them"
                    }
                );
                request.body = Some(json!({"label":format!("Label {index:06} 😀")}));
                if matches!(self, Self::SpeakerMissing) {
                    request.status = 404;
                    request.error = Some("speaker not found: meeting=meeting, speaker=missing");
                }
            }
            Self::Link | Self::LinkNew | Self::LinkPersonMissing | Self::LinkMeetingMissing => {
                request.method = Method::POST;
                request.path = format!(
                    "/api/meetings/{}/speakers/{}/link/{}",
                    if matches!(self, Self::LinkMeetingMissing) {
                        "missing"
                    } else {
                        "meeting"
                    },
                    if matches!(self, Self::LinkNew) {
                        format!("linked-{index:06}")
                    } else {
                        "them".into()
                    },
                    if matches!(self, Self::LinkPersonMissing) {
                        "missing"
                    } else {
                        "p000000"
                    }
                );
                if matches!(self, Self::LinkPersonMissing) {
                    request.status = 404;
                    request.error = Some("person not found: missing");
                }
            }
            Self::Skip | Self::SkipMissing => {
                request.path = format!(
                    "/api/meetings/{}/skip_observations",
                    if matches!(self, Self::Skip) {
                        "meeting"
                    } else {
                        "missing"
                    }
                );
                request.body = Some(json!({"skip":index.is_multiple_of(2)}));
            }
            Self::Delete | Self::DeleteMissing => {
                request.method = Method::DELETE;
                request.path = format!(
                    "/api/meetings/{}",
                    if matches!(self, Self::Delete) {
                        format!("delete-{index:06}")
                    } else {
                        "missing".into()
                    }
                );
                request.status = 204;
            }
            Self::Remove | Self::RemoveRepeat | Self::RemoveMissing | Self::RemoveInvalid => {
                request.method = Method::DELETE;
                request.status = 204;
                request.path = format!(
                    "/api/meetings/{}/segments/{}",
                    if matches!(self, Self::RemoveMissing) {
                        "missing"
                    } else {
                        "meeting"
                    },
                    match self {
                        Self::Remove => format!("chunk-{index:06}"),
                        Self::RemoveInvalid => "%20".into(),
                        _ => "removed".into(),
                    }
                );
                if matches!(self, Self::RemoveInvalid) {
                    request.status = 400;
                    request.error = Some("Transcript chunk identity must be valid.");
                }
            }
            Self::Removals | Self::RemovalsMissing => {
                request.method = Method::GET;
                request.path = format!(
                    "/api/meetings/{}/segment-removals",
                    if matches!(self, Self::Removals) {
                        "meeting"
                    } else {
                        "missing"
                    }
                );
            }
            Self::FocusList | Self::FocusDenied => {
                request.method = Method::GET;
                request.path = "/api/meetings/meeting/local-focus".into();
                request.native = matches!(self, Self::FocusList);
            }
            Self::FocusAppend
            | Self::FocusRetry
            | Self::FocusMismatch
            | Self::FocusMissing
            | Self::FocusAppendDenied => {
                request.method = Method::POST;
                request.status = 204;
                request.native = !matches!(self, Self::FocusAppendDenied);
                request.path = format!(
                    "/api/meetings/{}/local-focus",
                    if matches!(self, Self::FocusMissing) {
                        "missing"
                    } else {
                        "meeting"
                    }
                );
                let sample = if matches!(self, Self::FocusRetry) {
                    model::focus("meeting", "focus-seed", 1)
                } else {
                    model::focus(
                        if matches!(self, Self::FocusMismatch) {
                            "other"
                        } else if matches!(self, Self::FocusMissing) {
                            "missing"
                        } else {
                            "meeting"
                        },
                        &format!("focus-{index:06}"),
                        i64::try_from(index).expect("fixture index") + 10,
                    )
                };
                request.body = Some(json!(sample));
                if matches!(self, Self::FocusMismatch) {
                    request.status = 400;
                    request.error = Some("Local focus meeting mismatch.");
                }
            }
        }
        if matches!(
            self,
            Self::PatchMissing
                | Self::StopMissing
                | Self::SpeakerMeetingMissing
                | Self::LinkMeetingMissing
                | Self::SkipMissing
                | Self::DeleteMissing
                | Self::RemoveMissing
                | Self::RemovalsMissing
                | Self::FocusMissing
        ) {
            request.status = 404;
            request.error = Some("meeting not found: missing");
        }
        if matches!(self, Self::FocusDenied | Self::FocusAppendDenied) {
            request.status = 403;
            request.error = Some("Local context requires the native loopback client.");
        }
        request
    }
}
