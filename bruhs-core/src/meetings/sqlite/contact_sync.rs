use super::*;
use bruhs_models::{PersonProfile, PersonVersion, PersonVersionPage, PERSON_VERSION_PAGE_SIZE};

pub(super) fn versions(
    connection: &Connection,
    after: &str,
) -> MeetingStoreResult<PersonVersionPage> {
    let mut statement = connection.prepare("SELECT id,json_extract(payload,'$.id'),json_extract(payload,'$.updated_at') FROM people WHERE id>?1 ORDER BY id LIMIT ?2")?;
    let rows = statement.query_map(
        rusqlite::params![
            after,
            i64::try_from(PERSON_VERSION_PAGE_SIZE + 1).unwrap_or(i64::MAX)
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    let mut entries = Vec::new();
    for row in rows {
        let (id, saved_id, updated_at) = row?;
        if id.trim().is_empty() || id != saved_id {
            return Err(MeetingStoreError::PersistenceFailed(
                "Saved contact identity is malformed; original data was preserved.".into(),
            ));
        }
        entries.push(PersonVersion { id, updated_at });
    }
    Ok(PersonVersionPage::from_sorted(entries))
}

pub(super) fn profiles(
    connection: &Connection,
    ids: &str,
) -> MeetingStoreResult<Vec<PersonProfile>> {
    let mut statement = connection.prepare("SELECT people.id,people.payload FROM json_each(?1) JOIN people ON people.id=json_each.value ORDER BY json_each.key")?;
    let rows = statement.query_map([ids], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut profiles = Vec::new();
    for row in rows {
        let (id, payload) = row?;
        let profile: PersonProfile = decode(payload)?;
        if profile.id != id || crate::meetings::person_changes::validate_person(&profile).is_err() {
            return Err(MeetingStoreError::PersistenceFailed(
                "Saved contact is malformed; original data was preserved.".into(),
            ));
        }
        profiles.push(profile);
    }
    Ok(profiles)
}
