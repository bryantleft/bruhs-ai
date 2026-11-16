//! `bruhs_read_meetings` and `bruhs_read_meeting`: meetings loaded in the
//! app's workspace cache. Neither returns audio paths, and an unloaded
//! transcript is an error, never a fabricated empty one.
use super::page::{read_matching_page, read_page};
use super::{
    require_read, LowercaseQuery, MeetingReadSection, MAX_ID_BYTES, MAX_OFFSET, MAX_QUERY_BYTES,
};
use crate::{
    AgentDomain, AgentPermissionPolicy, Meeting, MeetingStatus, TranscriptSegment,
    TranscriptValidationError,
};
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
struct MeetingIdentity<'a> {
    id: &'a str,
    title: &'a str,
    status: MeetingStatus,
    started_at_ms: f64,
    ended_at_ms: Option<f64>,
    updated_at: i64,
    summary_available: bool,
}

impl<'a> From<&'a Meeting> for MeetingIdentity<'a> {
    fn from(meeting: &'a Meeting) -> Self {
        Self {
            id: &meeting.id,
            title: &meeting.title,
            status: meeting.status,
            started_at_ms: meeting.started_at_ms,
            ended_at_ms: meeting.ended_at_ms,
            updated_at: meeting.updated_at,
            summary_available: meeting.summary.is_some(),
        }
    }
}

/// Identity summaries. `query` matches ID and title only, never summaries,
/// transcripts or audio paths. The caller has checked Meetings Read.
pub fn read_meetings<'a>(
    meetings: impl Iterator<Item = &'a Meeting>,
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    if query.len() > MAX_QUERY_BYTES {
        return Err("Query is too large.".into());
    }
    let query = LowercaseQuery::new(query);
    let mut matcher = query.matcher();
    let mut result = read_matching_page(
        meetings,
        query.is_empty(),
        |meeting| matcher.matches_fields(&[&meeting.id, &meeting.title]),
        MeetingIdentity::from,
        offset,
    )?;
    result["source"] = "workspace_cache".into();
    Ok(result)
}

/// One meeting's summary, or a page of its loaded transcript. Checks
/// Meetings Read before resolving the ID. `transcript` returns the loaded
/// segments of a meeting and `validate` checks them (identity, ownership and
/// bounds) without copying; an unloaded or invalid transcript fails.
pub fn read_meeting<'a>(
    policy: &AgentPermissionPolicy,
    meetings: impl Iterator<Item = &'a Meeting>,
    transcript: impl FnOnce(&str) -> Option<&'a [TranscriptSegment]>,
    validate: impl FnOnce(&str) -> Result<(), TranscriptValidationError>,
    id: &str,
    section: MeetingReadSection,
    offset: usize,
) -> Result<Value, String> {
    // Revoke before ID resolution; Write permission is not Read permission.
    require_read(policy, AgentDomain::Meetings)?;
    if id.trim().is_empty()
        || id.len() > MAX_ID_BYTES
        || offset > MAX_OFFSET
        || (section == MeetingReadSection::Summary && offset != 0)
    {
        return Err("Invalid meeting ID or offset; summary requires offset 0.".into());
    }
    let mut matching = meetings.filter(|meeting| meeting.id == id);
    let meeting = matching.next().ok_or(
        "Meeting is not loaded in this workspace. Open or refresh Meetings in the app first.",
    )?;
    if matching.next().is_some() {
        return Err("Meeting identity is ambiguous in the workspace cache.".into());
    }
    let mut result = match section {
        MeetingReadSection::Summary => {
            #[derive(Serialize)]
            struct Summary<'a> {
                meeting: MeetingIdentity<'a>,
                speakers: &'a [crate::Speaker],
                summary: &'a Option<crate::MeetingSummary>,
            }
            read_page(
                std::iter::once(Summary {
                    meeting: meeting.into(),
                    speakers: &meeting.speakers,
                    summary: &meeting.summary,
                }),
                0,
            )?
        }
        MeetingReadSection::Transcript => {
            let segments = transcript(id).ok_or("Transcript is not loaded in this workspace. Open the meeting in the app first; no empty transcript is claimed.")?;
            // Validate borrowed records without cloning transcript text or changing
            // the live cache. Unknown/foreign/corrupt data never becomes a read receipt.
            validate(id).map_err(|error| error.to_string())?;
            read_page(segments.iter(), offset)?
        }
    };
    result["meeting_id"] = id.into();
    result["section"] =
        serde_json::to_value(section).map_err(|_| "Cannot encode meeting section.")?;
    result["source"] = "workspace_cache".into();
    result["meeting_updated_at"] = meeting.updated_at.into();
    Ok(result)
}
