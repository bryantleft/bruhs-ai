use super::*;
use crate::meetings::{summary_update::versions, SummaryUpdate};
use bruhs_models::{Meeting, PersonProfile};

pub(super) fn people(
    connection: &Connection,
    meeting: &Meeting,
) -> MeetingStoreResult<Vec<PersonProfile>> {
    let names: Vec<_> = meeting
        .speakers
        .iter()
        .map(|speaker| speaker.label.to_ascii_lowercase())
        .collect();
    let ids: Vec<_> = meeting
        .speakers
        .iter()
        .filter_map(|speaker| speaker.person_id.as_ref())
        .collect();
    // Deserialize only relevant contacts, not the whole voice-embedding directory.
    let mut query = connection.prepare("SELECT id,payload FROM people WHERE id IN (SELECT person_id FROM person_names WHERE name IN (SELECT value FROM json_each(?1))) OR id IN (SELECT value FROM json_each(?2)) OR EXISTS (SELECT 1 FROM json_each(people.payload,'$.observations') WHERE json_extract(value,'$.at_meeting_id')=?3)")?;
    let result = query
        .query_map(
            rusqlite::params![encode(&names)?, encode(&ids)?, meeting.id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?
        .map(|row| {
            let (id, payload) = row?;
            let person: PersonProfile = decode(payload)?;
            if person.id != id {
                return Err(MeetingStoreError::PersistenceFailed(
                    "Saved participant identity does not match its key.".into(),
                ));
            }
            Ok(person)
        })
        .collect();
    result
}

pub(super) fn participant_versions(
    connection: &Connection,
    expected: &Meeting,
) -> MeetingStoreResult<std::collections::BTreeMap<String, i64>> {
    if records::meeting(connection, &expected.id)?.as_ref() != Some(expected) {
        return Err(MeetingStoreError::Conflict(
            "Meeting changed before generation; reload and retry.".into(),
        ));
    }
    Ok(versions(people(connection, expected)?.iter(), expected))
}

pub(super) fn commit(
    connection: &Connection,
    update: SummaryUpdate,
) -> MeetingStoreResult<Meeting> {
    let current = records::required(
        records::meeting(connection, &update.expected.id)?,
        &update.expected.id,
    )?;
    update.validate(&current, &records::segments(connection, &current.id)?)?;
    let relevant = people(connection, &current)?;
    let changed = update.stage_people(relevant.iter())?;
    for person in changed.values() {
        contacts::save(connection, person)?;
    }
    records::required(
        records::edit_meeting(connection, &current.id, |meeting| {
            meeting.summary = Some(update.summary);
            Ok(())
        })?,
        &current.id,
    )
}
