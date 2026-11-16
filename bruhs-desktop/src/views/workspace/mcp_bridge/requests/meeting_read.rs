use super::*;
use bruhs_models::workspace_bridge::MeetingReadSection;

impl Workspace {
    pub(super) fn read_bridge_meetings(
        &self,
        query: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        // Search identity only: neither summaries nor audio paths affect discovery.
        bridge::read_meetings(self.meetings_hub.read(cx).meetings.iter(), query, offset)
    }

    pub(super) fn read_bridge_meeting(
        &self,
        id: &str,
        section: MeetingReadSection,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        let hub = self.meetings_hub.read(cx);
        bridge::read_meeting(
            &self.automation_state.permissions,
            hub.meetings.iter(),
            |id| hub.segments.get(id).map(Vec::as_slice),
            // Validates borrowed records without cloning transcript text or
            // changing the live cache; the result is cached per meeting.
            |id| hub.segments.validate(id),
            id,
            section,
            offset,
        )
    }
}
