use super::*;
use bruhs_models::workspace_bridge::PersonReadSection;

impl Workspace {
    pub(super) fn read_bridge_people(
        &self,
        query: &str,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        // Discovery never serializes or scans histories, private notes or fingerprints.
        let hub = self.meetings_hub.read(cx);
        bridge::read_people(hub.people.iter().map(AsRef::as_ref), query, offset)
    }

    pub(super) fn read_bridge_person(
        &self,
        person_id: &str,
        section: PersonReadSection,
        offset: usize,
        cx: &App,
    ) -> Result<Value, String> {
        bridge::read_person(
            &self.automation_state.permissions,
            self.meetings_hub.read(cx).people.iter().map(AsRef::as_ref),
            self.relationships.iter(),
            person_id,
            section,
            offset,
        )
    }
}
