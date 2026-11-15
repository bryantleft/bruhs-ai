use super::*;
use crate::meetings::person_changes::{self, next_version, MergeLinks};
use bruhs_models::{ActionItemRef, Meeting, MeetingSpeakerRef, TranscriptSegment};

pub(super) fn meeting(connection: &Connection, id: &str) -> MeetingStoreResult<Option<Meeting>> {
    let value: Option<Meeting> = get(connection, "meetings", id)?;
    if value.as_ref().is_some_and(|value| value.id != id) {
        return Err(MeetingStoreError::PersistenceFailed(
            "Saved meeting identity does not match its key.".into(),
        ));
    }
    Ok(value)
}

pub(super) fn save_meeting(connection: &Connection, value: &Meeting) -> MeetingStoreResult<()> {
    if value.id.is_empty()
        || !value.started_at_ms.is_finite()
        || value.ended_at_ms.is_some_and(|v| !v.is_finite())
    {
        return Err(MeetingStoreError::InvalidInput(
            "Meeting identity and times must be valid.".into(),
        ));
    }
    let mut speakers = std::collections::HashSet::new();
    if value
        .speakers
        .iter()
        .any(|speaker| speaker.id.is_empty() || !speakers.insert(&speaker.id))
    {
        return Err(MeetingStoreError::InvalidInput(
            "Meeting speaker identities must be unique.".into(),
        ));
    }
    connection.execute("INSERT INTO meetings(id,started,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET started=excluded.started,payload=excluded.payload", rusqlite::params![value.id, value.started_at_ms, encode(value)?])?;
    connection.execute(
        "DELETE FROM meeting_people WHERE meeting_id=?1",
        [&value.id],
    )?;
    let mut referenced_people = std::collections::HashSet::new();
    let mut insert_reference = None;
    for id in value
        .speakers
        .iter()
        .filter_map(|speaker| speaker.person_id.as_ref())
        .chain(value.summary.iter().flat_map(|summary| {
            summary
                .action_items
                .iter()
                .filter_map(|item| item.assignee_person_id.as_ref())
        }))
    {
        if !referenced_people.insert(id) {
            continue;
        }
        if contacts::person(connection, id)?.is_none() {
            return Err(MeetingStoreError::PersonNotFound { id: id.clone() });
        }
        let insert = match insert_reference.as_mut() {
            Some(insert) => insert,
            None => insert_reference.insert(connection.prepare(
                "INSERT OR IGNORE INTO meeting_people(meeting_id,person_id) VALUES(?1,?2)",
            )?),
        };
        insert.execute([&value.id, id])?;
    }
    Ok(())
}

pub(super) fn edit_meeting(
    connection: &Connection,
    id: &str,
    edit: impl FnOnce(&mut Meeting) -> MeetingStoreResult<()>,
) -> MeetingStoreResult<Option<Meeting>> {
    let Some(mut value) = meeting(connection, id)? else {
        return Ok(None);
    };
    value.updated_at = next_version(value.updated_at)?;
    edit(&mut value)?;
    save_meeting(connection, &value)?;
    Ok(Some(value))
}

pub(super) fn required(value: Option<Meeting>, id: &str) -> MeetingStoreResult<Meeting> {
    value.ok_or_else(|| MeetingStoreError::MeetingNotFound { id: id.into() })
}

pub(super) fn append_segment(
    connection: &Connection,
    value: &TranscriptSegment,
) -> MeetingStoreResult<()> {
    if value.id.is_empty()
        || !value.confidence.is_finite()
        || value.speaker_confidence.is_some_and(|v| !v.is_finite())
        || value.end_ms < value.start_ms
    {
        return Err(MeetingStoreError::InvalidInput(
            "Transcript identity, times and confidence must be valid.".into(),
        ));
    }
    if meeting(connection, &value.meeting_id)?.is_none() {
        return Err(MeetingStoreError::MeetingNotFound {
            id: value.meeting_id.clone(),
        });
    }
    if chunk_removed(connection, &value.meeting_id, &value.id)? {
        return Ok(());
    }
    if let Some(current) = get::<TranscriptSegment>(connection, "segments", &value.id)? {
        if current == *value {
            return Ok(());
        }
        if current.meeting_id != value.meeting_id || !current.partial {
            return Err(MeetingStoreError::Conflict(
                "A completed transcript segment cannot be replaced by a replay.".into(),
            ));
        }
    }
    connection.execute("INSERT INTO segments(id,meeting_id,started,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET started=excluded.started,payload=excluded.payload", rusqlite::params![value.id,value.meeting_id,value.start_ms,encode(value)?])?;
    Ok(())
}

pub(super) fn remove_segment(
    connection: &Connection,
    meeting_id: &str,
    segment_id: &str,
) -> MeetingStoreResult<()> {
    crate::meetings::valid_chunk_id(segment_id)?;
    if meeting(connection, meeting_id)?.is_none() {
        return Err(MeetingStoreError::MeetingNotFound {
            id: meeting_id.into(),
        });
    }
    connection.execute(
        "INSERT OR IGNORE INTO segment_removals(meeting_id,segment_id) VALUES(?1,?2)",
        [meeting_id, segment_id],
    )?;
    connection.execute(
        "DELETE FROM segments WHERE meeting_id=?1 AND id=?2",
        [meeting_id, segment_id],
    )?;
    Ok(())
}

pub(super) fn segment_removed(
    connection: &Connection,
    meeting_id: &str,
    segment_id: &str,
) -> MeetingStoreResult<bool> {
    chunk_removed(connection, meeting_id, segment_id)
}

pub(super) fn removed_segment_ids(
    connection: &Connection,
    meeting_id: &str,
) -> MeetingStoreResult<Vec<String>> {
    if meeting(connection, meeting_id)?.is_none() {
        return Err(MeetingStoreError::MeetingNotFound {
            id: meeting_id.into(),
        });
    }
    let mut ids = connection
        .prepare("SELECT segment_id FROM segment_removals WHERE meeting_id=?1 ORDER BY segment_id")?
        .query_map([meeting_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    ids.sort();
    Ok(ids)
}

fn chunk_removed(
    connection: &Connection,
    meeting_id: &str,
    segment_id: &str,
) -> MeetingStoreResult<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM segment_removals WHERE meeting_id=?1 AND segment_id=?2",
            [meeting_id, segment_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

fn removal_ids(
    connection: &Connection,
    meeting_id: &str,
) -> MeetingStoreResult<std::collections::HashSet<String>> {
    connection
        .prepare("SELECT segment_id FROM segment_removals WHERE meeting_id=?1")?
        .query_map([meeting_id], |row| row.get(0))?
        .collect::<Result<_, _>>()
        .map_err(Into::into)
}

pub(super) fn segments(
    connection: &Connection,
    meeting_id: &str,
) -> MeetingStoreResult<Vec<TranscriptSegment>> {
    connection.prepare("SELECT id,payload FROM segments WHERE meeting_id=?1 AND id NOT IN (SELECT segment_id FROM segment_removals WHERE meeting_id=?1) ORDER BY started,id")?
        .query_map([meeting_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
        .map(|row| {
            let (id, payload) = row?;
            let segment: TranscriptSegment = decode(payload)?;
            if segment.id != id || segment.meeting_id != meeting_id {
                return Err(MeetingStoreError::PersistenceFailed("Saved transcript identity does not match its key; original data was preserved.".into()));
            }
            Ok(segment)
        }).collect()
}

pub(super) fn cache_transcript(
    connection: &Connection,
    meeting_id: &str,
    mut expected: Vec<TranscriptSegment>,
    mut incoming: Vec<TranscriptSegment>,
) -> MeetingStoreResult<Vec<TranscriptSegment>> {
    let removed = removal_ids(connection, meeting_id)?;
    if !removed.is_empty() {
        connection.execute(
            "DELETE FROM segments WHERE meeting_id=?1 AND id IN (SELECT segment_id FROM segment_removals WHERE meeting_id=?1)",
            [meeting_id],
        )?;
        expected.retain(|segment| !removed.contains(&segment.id));
        incoming.retain(|segment| !removed.contains(&segment.id));
    }
    bruhs_models::validate_transcript_snapshot(meeting_id, &expected, &mut incoming)
        .map_err(|error| MeetingStoreError::Conflict(error.to_string()))?;
    required(meeting(connection, meeting_id)?, meeting_id)?;
    if segments(connection, meeting_id)? != expected {
        return Err(MeetingStoreError::Conflict("Transcript changed while capture was being fetched; retry with the current saved copy.".into()));
    }
    if incoming == expected {
        return Ok(incoming);
    }
    let incoming_ids = incoming
        .iter()
        .map(|segment| segment.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    for old in expected
        .iter()
        .filter(|old| old.partial && !incoming_ids.contains(old.id.as_str()))
    {
        connection.execute(
            "DELETE FROM segments WHERE id=?1 AND meeting_id=?2",
            [&old.id, meeting_id],
        )?;
    }
    for segment in &incoming {
        if let Some(saved) = get::<TranscriptSegment>(connection, "segments", &segment.id)? {
            if saved.meeting_id != meeting_id {
                return Err(MeetingStoreError::Conflict(
                    "Transcript identity belongs to another meeting.".into(),
                ));
            }
        }
        connection.execute("INSERT INTO segments(id,meeting_id,started,payload) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET started=excluded.started,payload=excluded.payload", rusqlite::params![segment.id,meeting_id,segment.start_ms,encode(segment)?])?;
    }
    Ok(incoming)
}

/// Merge cascade: every speaker and action item of `source` moves to
/// `target`. Returns what moved so the merge can be undone.
pub(super) fn retarget_meetings(
    connection: &Connection,
    source: &bruhs_models::PersonProfile,
    target: &bruhs_models::PersonProfile,
) -> MeetingStoreResult<MergeLinks> {
    let mut links = MergeLinks::default();
    for id in meetings_of(connection, &source.id)? {
        update_meeting_if(connection, &id, |meeting| {
            let moved = person_changes::retarget_meeting(meeting, source, target);
            let changed = !moved.speakers.is_empty() || !moved.action_items.is_empty();
            links.extend(moved);
            changed
        })?;
    }
    Ok(links)
}

/// Meetings that reference `person_id` through a speaker or action item.
fn meetings_of(connection: &Connection, person_id: &str) -> MeetingStoreResult<Vec<String>> {
    let ids = connection
        .prepare("SELECT meeting_id FROM meeting_people WHERE person_id=?1 ORDER BY meeting_id")?
        .query_map([person_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ids)
}

/// Save `edit` with a new version only when it reports a change.
fn update_meeting_if(
    connection: &Connection,
    id: &str,
    edit: impl FnOnce(&mut Meeting) -> bool,
) -> MeetingStoreResult<bool> {
    let Some(mut value) = meeting(connection, id)? else {
        return Ok(false);
    };
    let version = next_version(value.updated_at)?;
    if !edit(&mut value) {
        return Ok(false);
    }
    value.updated_at = version;
    save_meeting(connection, &value)?;
    Ok(true)
}

/// Delete cascade: unlink `person_id` from every meeting it is referenced in.
pub(super) fn unlink_person(
    connection: &Connection,
    person_id: &str,
) -> MeetingStoreResult<(Vec<MeetingSpeakerRef>, Vec<ActionItemRef>)> {
    let mut speakers = Vec::new();
    let mut items = Vec::new();
    for id in meetings_of(connection, person_id)? {
        update_meeting_if(connection, &id, |meeting| {
            let (lost_speakers, lost_items) = person_changes::unlink_meeting(meeting, person_id);
            let changed = !lost_speakers.is_empty() || !lost_items.is_empty();
            speakers.extend(lost_speakers);
            items.extend(lost_items);
            changed
        })?;
    }
    Ok((speakers, items))
}

/// Undo of [`unlink_person`] for a restored contact. The contact must be saved first.
pub(super) fn relink_person(
    connection: &Connection,
    person_id: &str,
    speakers: &[MeetingSpeakerRef],
    items: &[ActionItemRef],
) -> MeetingStoreResult<()> {
    let ids = speakers
        .iter()
        .map(|speaker| speaker.meeting_id.as_str())
        .chain(items.iter().map(|item| item.meeting_id.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    for id in ids {
        update_meeting_if(connection, id, |meeting| {
            person_changes::relink_meeting(meeting, person_id, speakers, items)
        })?;
    }
    Ok(())
}

/// Undo of [`retarget_meetings`]. Both contacts must be saved first.
pub(super) fn restore_merge_links(
    connection: &Connection,
    source_id: &str,
    target_id: &str,
    merged_label: &str,
    links: &MergeLinks,
) -> MeetingStoreResult<()> {
    for id in links.meeting_ids() {
        update_meeting_if(connection, id, |meeting| {
            person_changes::restore_meeting_links(meeting, source_id, target_id, merged_label, links)
        })?;
    }
    Ok(())
}
