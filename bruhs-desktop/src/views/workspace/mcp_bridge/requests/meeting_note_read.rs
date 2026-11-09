use super::read_page::read_page;
use super::*;

impl Workspace {
    fn require_meeting_notes_read(&self) -> Result<(), String> {
        if !self
            .automation_state
            .permissions
            .allows(AgentDomain::Meetings, AgentAccess::Read)
        {
            return Err("Read access is not granted for this domain.".into());
        }
        Ok(())
    }

    pub(super) fn read_bridge_meeting_notes(
        &self,
        query: &str,
        offset: usize,
    ) -> Result<Value, String> {
        self.require_meeting_notes_read()?;
        if query.len() > 512 || offset > 100_000 {
            return Err("Query or offset is too large.".into());
        }
        let query = query.to_lowercase();
        #[derive(Serialize)]
        struct Identity<'a> {
            id: &'a str,
            title: &'a str,
            updated_at: i64,
        }
        let records = self
            .meeting_notes
            .iter()
            .filter(|note| {
                query.is_empty()
                    || note.id.to_lowercase().contains(&query)
                    || note.title.to_lowercase().contains(&query)
            })
            .map(|note| Identity {
                id: &note.id,
                title: &note.title,
                updated_at: note.updated_at,
            });
        let mut result = read_page(records, offset)?;
        result["source"] = "workspace_local_notes".into();
        Ok(result)
    }

    pub(super) fn read_bridge_meeting_note(&self, id: &str) -> Result<Value, String> {
        self.require_meeting_notes_read()?;
        if id.trim().is_empty() || id.len() > 128 {
            return Err("Invalid meeting note ID.".into());
        }
        let mut matching = self.meeting_notes.iter().filter(|note| note.id == id);
        let note = matching
            .next()
            .ok_or("Meeting note is not present in this workspace.")?;
        if matching.next().is_some() {
            return Err("Meeting note identity is ambiguous.".into());
        }
        let mut result = read_page(std::iter::once(note), 0)?;
        result["source"] = "workspace_local_notes".into();
        Ok(result)
    }
}
