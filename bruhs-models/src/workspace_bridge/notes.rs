//! `bruhs_read_meeting_notes` and `bruhs_read_meeting_note`: local text notes,
//! separate from captured meetings and generated summaries.
use super::page::{read_matching_page, read_page};
use super::{require_read, LowercaseQuery, MAX_ID_BYTES, MAX_OFFSET, MAX_QUERY_BYTES};
use crate::{AgentDomain, AgentPermissionPolicy, MeetingNote};
use serde::Serialize;
use serde_json::Value;

/// Note identities. Checks Meetings Read first; `query` matches ID and title.
pub fn read_meeting_notes(
    policy: &AgentPermissionPolicy,
    notes: &[MeetingNote],
    query: &str,
    offset: usize,
) -> Result<Value, String> {
    require_read(policy, AgentDomain::Meetings)?;
    if query.len() > MAX_QUERY_BYTES || offset > MAX_OFFSET {
        return Err("Query or offset is too large.".into());
    }
    let query = LowercaseQuery::new(query);
    let mut matcher = query.matcher();
    #[derive(Serialize)]
    struct Identity<'a> {
        id: &'a str,
        title: &'a str,
        updated_at: i64,
    }
    let mut result = read_matching_page(
        notes.iter(),
        query.is_empty(),
        |note| matcher.matches_fields(&[&note.id, &note.title]),
        |note| Identity {
            id: &note.id,
            title: &note.title,
            updated_at: note.updated_at,
        },
        offset,
    )?;
    result["source"] = "workspace_local_notes".into();
    Ok(result)
}

/// One whole note by exact ID. Checks Meetings Read before resolving it.
pub fn read_meeting_note(
    policy: &AgentPermissionPolicy,
    notes: &[MeetingNote],
    id: &str,
) -> Result<Value, String> {
    require_read(policy, AgentDomain::Meetings)?;
    if id.trim().is_empty() || id.len() > MAX_ID_BYTES {
        return Err("Invalid meeting note ID.".into());
    }
    let mut matching = notes.iter().filter(|note| note.id == id);
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
