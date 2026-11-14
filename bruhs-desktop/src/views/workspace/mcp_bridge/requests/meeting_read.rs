use super::read_page::{read_matching_page, read_page};
use super::*;
use bruhs_core::workspace_bridge::MeetingReadSection;
use bruhs_models::{Meeting, MeetingStatus};

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

impl Workspace {
    pub(super) fn read_bridge_meetings(
        &self,
        query: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        if query.len() > 512 {
            return Err("Query is too large.".into());
        }
        let query = LowercaseQuery::new(query);
        let mut matcher = query.matcher();
        let hub = self.meetings_hub.read(cx);
        // Search identity only: neither summaries nor audio paths affect discovery.
        let mut result = read_matching_page(
            hub.meetings.iter(),
            query.is_empty(),
            |meeting| matcher.matches_fields(&[&meeting.id, &meeting.title]),
            MeetingIdentity::from,
            offset,
        )?;
        result["source"] = "workspace_cache".into();
        Ok(result)
    }

    pub(super) fn read_bridge_meeting(
        &self,
        id: &str,
        section: MeetingReadSection,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        // Revoke before ID resolution; Write permission is not Read permission.
        if !self
            .automation_state
            .permissions
            .allows(AgentDomain::Meetings, AgentAccess::Read)
        {
            return Err("Read access is not granted for this domain.".into());
        }
        if id.trim().is_empty()
            || id.len() > 128
            || offset > 100_000
            || (section == MeetingReadSection::Summary && offset != 0)
        {
            return Err("Invalid meeting ID or offset; summary requires offset 0.".into());
        }
        let hub = self.meetings_hub.read(cx);
        let mut matching = hub.meetings.iter().filter(|meeting| meeting.id == id);
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
                    speakers: &'a [bruhs_models::Speaker],
                    summary: &'a Option<bruhs_models::MeetingSummary>,
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
                let segments = hub.segments.get(id).ok_or("Transcript is not loaded in this workspace. Open the meeting in the app first; no empty transcript is claimed.")?;
                // Validate borrowed records without cloning transcript text or changing
                // the live cache. Unknown/foreign/corrupt data never becomes a read receipt.
                hub.segments
                    .validate(id)
                    .map_err(|error| error.to_string())?;
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
}
