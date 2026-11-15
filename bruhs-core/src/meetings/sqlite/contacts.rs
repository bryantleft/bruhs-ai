use super::*;
use crate::meetings::person_changes::{
    aliases_to_save, merged_profile, next_version, now_ms, restored_source, unmerged_target,
    validate_person, ArchivedMerge, DeletionArchive, RESTORE_LIVE, UNMERGE_SOURCE_LIVE,
    UNMERGE_SOURCE_TARGETED, UNMERGE_TARGET_GONE,
};
use bruhs_models::{person_name_key, PersonDeletion, PersonObservation, PersonProfile};
use std::collections::HashMap;

pub(super) fn resolve_merges(
    connection: &Connection,
    ids: &str,
) -> MeetingStoreResult<Vec<bruhs_models::PersonMergeResolution>> {
    // UNION (not UNION ALL) terminates corrupt cycles. One statement gives every
    // requested redirect and live target the same SQLite read snapshot.
    let mut statement = connection.prepare("WITH RECURSIVE
        roots(source_id) AS (SELECT source_id FROM merged_people JOIN json_each(?1) ON source_id=json_each.value),
        paths(source_id,target_id) AS (
            SELECT roots.source_id,merged_people.target_id FROM roots JOIN merged_people USING(source_id)
            UNION SELECT paths.source_id,merged_people.target_id FROM paths JOIN merged_people ON paths.target_id=merged_people.source_id),
        terminal(source_id,id,payload) AS (SELECT paths.source_id,people.id,people.payload FROM paths JOIN people ON people.id=paths.target_id)
        SELECT roots.source_id,terminal.id,terminal.payload FROM roots LEFT JOIN terminal USING(source_id) ORDER BY roots.source_id")?;
    let rows = statement.query_map([ids], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut resolved = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let (source_id, target_id, payload) = row?;
        let target: PersonProfile = decode(payload.ok_or_else(|| {
            MeetingStoreError::PersistenceFailed(
                "Merged contact has no live target; original records were preserved.".into(),
            )
        })?)?;
        if target_id.as_deref() != Some(&target.id)
            || target.id == source_id
            || !seen.insert(source_id.clone())
            || validate_person(&target).is_err()
        {
            return Err(MeetingStoreError::PersistenceFailed(
                "Saved merge identity is malformed; original records were preserved.".into(),
            ));
        }
        resolved.push(bruhs_models::PersonMergeResolution { source_id, target });
    }
    Ok(resolved)
}

pub(super) fn list(connection: &Connection) -> MeetingStoreResult<Vec<PersonProfile>> {
    all(
        connection,
        "SELECT id,payload FROM people ORDER BY id",
        |id, value: &PersonProfile| value.id == id && validate_person(value).is_ok(),
    )
}

pub(super) fn person(
    connection: &Connection,
    id: &str,
) -> MeetingStoreResult<Option<PersonProfile>> {
    let value: Option<PersonProfile> = get(connection, "people", id)?;
    if value
        .as_ref()
        .is_some_and(|value| value.id != id || validate_person(value).is_err())
    {
        return Err(MeetingStoreError::PersistenceFailed(
            "Saved contact is malformed. Original data was preserved.".into(),
        ));
    }
    Ok(value)
}

pub(super) fn required(connection: &Connection, id: &str) -> MeetingStoreResult<PersonProfile> {
    person(connection, id)?.ok_or_else(|| MeetingStoreError::PersonNotFound { id: id.into() })
}

pub(super) fn save(connection: &Connection, value: &PersonProfile) -> MeetingStoreResult<()> {
    validate_person(value)?;
    let voice = value.voice_embedding.as_ref().map(encode).transpose()?;
    connection.execute("INSERT INTO people(id,payload,voice) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,voice=excluded.voice", rusqlite::params![value.id,encode(value)?,voice])?;
    connection.execute("DELETE FROM person_names WHERE person_id=?1", [&value.id])?;
    let mut insert_name =
        connection.prepare("INSERT OR IGNORE INTO person_names(name,person_id) VALUES(?1,?2)")?;
    for name in std::iter::once(&value.display_name).chain(&value.aliases) {
        insert_name.execute([&person_name_key(name), &value.id])?;
    }
    Ok(())
}

/// Rekey names indexed by older builds, which stored ASCII-lowercased names, so
/// a contact (and its opt-out) still matches every differently cased label.
/// Keys are idempotent, so this is a no-op once every row is current.
pub(super) fn rekey_names(connection: &Connection) -> MeetingStoreResult<()> {
    let rows = connection
        .prepare("SELECT name,person_id FROM person_names")?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut delete =
        connection.prepare("DELETE FROM person_names WHERE name=?1 AND person_id=?2")?;
    let mut insert =
        connection.prepare("INSERT OR IGNORE INTO person_names(name,person_id) VALUES(?1,?2)")?;
    for (name, person_id) in rows {
        let key = person_name_key(&name);
        if key != name {
            delete.execute([&name, &person_id])?;
            insert.execute([&key, &person_id])?;
        }
    }
    Ok(())
}

pub(super) fn insert(
    connection: &Connection,
    value: PersonProfile,
) -> MeetingStoreResult<PersonProfile> {
    let merged: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM merged_people WHERE source_id=?1)",
        [&value.id],
        |row| row.get(0),
    )?;
    if merged {
        return Err(MeetingStoreError::Conflict(
            "This contact was already merged.".into(),
        ));
    }
    if let Some(current) = person(connection, &value.id)? {
        if current != value {
            return Err(MeetingStoreError::Conflict(
                "Contact already exists; use a field-specific update.".into(),
            ));
        }
        return Ok(current);
    }
    save(connection, &value)?;
    Ok(value)
}

/// Inside the caller's write transaction, so the existence check and the
/// insert are one atomic step.
pub(super) fn insert_if_absent(
    connection: &Connection,
    value: PersonProfile,
) -> MeetingStoreResult<crate::meetings::PersonInsert> {
    use crate::meetings::PersonInsert;
    validate_person(&value)?;
    if let Some(current) = person(connection, &value.id)? {
        return Ok(PersonInsert::Existing(current));
    }
    insert(connection, value).map(PersonInsert::Created)
}

pub(super) fn edit(
    connection: &Connection,
    id: &str,
    edit: impl FnOnce(&mut PersonProfile) -> MeetingStoreResult<()>,
) -> MeetingStoreResult<PersonProfile> {
    let mut value = required(connection, id)?;
    value.updated_at = next_version(value.updated_at)?;
    edit(&mut value)?;
    save(connection, &value)?;
    Ok(value)
}

pub(super) fn merge(
    connection: &Connection,
    source_id: &str,
    target_id: &str,
) -> MeetingStoreResult<PersonProfile> {
    if source_id == target_id {
        return Err(MeetingStoreError::InvalidInput(
            "Source and target must be different contacts.".into(),
        ));
    }
    let prior: Option<String> = connection
        .query_row(
            "SELECT target_id FROM merged_people WHERE source_id=?1",
            [source_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(prior) = prior {
        if prior != target_id {
            return Err(MeetingStoreError::Conflict(
                "Source was already merged into another contact.".into(),
            ));
        }
        return required(connection, target_id);
    }
    let source = required(connection, source_id)?;
    let target = required(connection, target_id)?;
    let merged = merged_profile(&source, &target)?;
    save(connection, &merged)?;
    let links = records::retarget_meetings(connection, &source, &merged)?;
    connection.execute("INSERT INTO merged_people(source_id,target_id,source_original,target_original,links) VALUES(?1,?2,?3,?4,?5)", rusqlite::params![source_id,target_id,encode(&source)?,encode(&target)?,encode(&links)?])?;
    remove(connection, source_id)?;
    Ok(merged)
}

fn remove(connection: &Connection, id: &str) -> MeetingStoreResult<()> {
    connection.execute("DELETE FROM person_names WHERE person_id=?1", [id])?;
    connection.execute("DELETE FROM people WHERE id=?1", [id])?;
    Ok(())
}

fn is_merged(connection: &Connection, id: &str) -> MeetingStoreResult<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM merged_people WHERE source_id=?1)",
        [id],
        |row| row.get(0),
    )?)
}

/// A `merged_people` row: source, target, both originals and the link record.
type MergeRow = (String, String, String, String, Option<String>);

fn archived_merge(
    (source_id, target_id, source, target, links): MergeRow,
) -> MeetingStoreResult<ArchivedMerge> {
    Ok(ArchivedMerge {
        source_id,
        target_id,
        source_original: decode(source)?,
        target_original: decode(target)?,
        links: links.map(decode).transpose()?,
    })
}

fn merge_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MergeRow> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
}

fn insert_merge(connection: &Connection, merge: &ArchivedMerge) -> MeetingStoreResult<()> {
    let links = merge.links.as_ref().map(encode).transpose()?;
    connection.execute("INSERT INTO merged_people(source_id,target_id,source_original,target_original,links) VALUES(?1,?2,?3,?4,?5)", rusqlite::params![merge.source_id, merge.target_id, encode(&merge.source_original)?, encode(&merge.target_original)?, links])?;
    Ok(())
}

/// PEOPLE-10: see [`crate::meetings::PeopleStore::delete_person`].
pub(super) fn delete(connection: &Connection, id: &str) -> MeetingStoreResult<PersonDeletion> {
    let person = required(connection, id)?;
    let (speakers, action_items) = records::unlink_person(connection, id)?;
    // Every merge whose redirect chain ends here, or `resolve_merges` would
    // fail for those retired ids once the terminal contact is gone.
    let merges = connection
        .prepare(
            "WITH RECURSIVE chain(source_id) AS (
                SELECT source_id FROM merged_people WHERE target_id=?1
                UNION SELECT merged_people.source_id FROM merged_people JOIN chain ON merged_people.target_id=chain.source_id)
            SELECT merged_people.source_id,target_id,source_original,target_original,links
            FROM merged_people JOIN chain USING(source_id) ORDER BY merged_people.source_id",
        )?
        .query_map([id], merge_row)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(archived_merge)
        .collect::<MeetingStoreResult<Vec<_>>>()?;
    for merge in &merges {
        connection.execute(
            "DELETE FROM merged_people WHERE source_id=?1",
            [&merge.source_id],
        )?;
    }
    remove(connection, id)?;
    let deletion = PersonDeletion {
        person,
        speakers,
        action_items,
        deleted_at: now_ms(),
    };
    let archive = DeletionArchive {
        deletion: deletion.clone(),
        merges,
    };
    connection.execute("INSERT INTO deleted_people(id,deleted,payload) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET deleted=excluded.deleted,payload=excluded.payload", rusqlite::params![id, deletion.deleted_at, encode(&archive)?])?;
    Ok(deletion)
}

/// PEOPLE-09: see [`crate::meetings::PeopleStore::restore_person`].
pub(super) fn restore(connection: &Connection, id: &str) -> MeetingStoreResult<PersonProfile> {
    let archive: DeletionArchive = get(connection, "deleted_people", id)?
        .ok_or_else(|| MeetingStoreError::PersonNotFound { id: id.into() })?;
    if archive.deletion.person.id != id {
        return Err(MeetingStoreError::PersistenceFailed(
            "Saved contact deletion is malformed. Original data was preserved.".into(),
        ));
    }
    if person(connection, id)?.is_some() || is_merged(connection, id)? {
        return Err(MeetingStoreError::Conflict(RESTORE_LIVE.into()));
    }
    let mut value = archive.deletion.person.clone();
    value.updated_at = next_version(value.updated_at)?;
    save(connection, &value)?;
    records::relink_person(
        connection,
        id,
        &archive.deletion.speakers,
        &archive.deletion.action_items,
    )?;
    for merge in &archive.merges {
        // A retired id that is live again keeps its new identity.
        if person(connection, &merge.source_id)?.is_none() && !is_merged(connection, &merge.source_id)? {
            insert_merge(connection, merge)?;
        }
    }
    connection.execute("DELETE FROM deleted_people WHERE id=?1", [id])?;
    Ok(value)
}

/// PEOPLE-09: see [`crate::meetings::PeopleStore::unmerge_person`].
pub(super) fn unmerge(
    connection: &Connection,
    source_id: &str,
) -> MeetingStoreResult<bruhs_models::PersonUnmerge> {
    let merge = connection
        .query_row(
            "SELECT source_id,target_id,source_original,target_original,links FROM merged_people WHERE source_id=?1",
            [source_id],
            merge_row,
        )
        .optional()?
        .map(archived_merge)
        .transpose()?
        .ok_or_else(|| MeetingStoreError::PersonNotFound {
            id: source_id.into(),
        })?;
    if person(connection, source_id)?.is_some() {
        return Err(MeetingStoreError::Conflict(UNMERGE_SOURCE_LIVE.into()));
    }
    let targeted: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM merged_people WHERE target_id=?1)",
        [source_id],
        |row| row.get(0),
    )?;
    if targeted {
        return Err(MeetingStoreError::Conflict(UNMERGE_SOURCE_TARGETED.into()));
    }
    let current = person(connection, &merge.target_id)?
        .ok_or_else(|| MeetingStoreError::Conflict(UNMERGE_TARGET_GONE.into()))?;
    let target = unmerged_target(&current, &merge.source_original, &merge.target_original)?;
    let source = restored_source(&merge.source_original)?;
    connection.execute("DELETE FROM merged_people WHERE source_id=?1", [source_id])?;
    save(connection, &target)?;
    save(connection, &source)?;
    if let Some(links) = &merge.links {
        records::restore_merge_links(
            connection,
            source_id,
            &merge.target_id,
            &merge.target_original.display_name,
            links,
        )?;
    }
    Ok(bruhs_models::PersonUnmerge { source, target })
}

/// PEOPLE-04: see [`crate::meetings::PeopleStore::set_person_aliases`].
pub(super) fn set_aliases(
    connection: &Connection,
    id: &str,
    aliases: &[String],
) -> MeetingStoreResult<PersonProfile> {
    let people = list(connection)?;
    edit(connection, id, |person| {
        person.aliases = aliases_to_save(person, aliases, people.iter())?;
        Ok(())
    })
}

/// PEOPLE-14: drop every observation sourced from `meeting_id`, bumping each
/// changed contact's version. Returns how many were deleted.
pub(super) fn drop_meeting_observations(
    connection: &Connection,
    meeting_id: &str,
) -> MeetingStoreResult<usize> {
    let mut deleted = 0;
    for mut person in list(connection)? {
        let removed = bruhs_models::drop_meeting_observations(&mut person, meeting_id);
        if removed > 0 {
            deleted += removed;
            person.updated_at = next_version(person.updated_at)?;
            save(connection, &person)?;
        }
    }
    Ok(deleted)
}

pub(super) fn observations(
    connection: &Connection,
    by_name: HashMap<String, Vec<PersonObservation>>,
) -> MeetingStoreResult<Vec<PersonProfile>> {
    let mut updated = HashMap::new();
    for (name, observations) in by_name {
        let name = name.trim();
        if name.is_empty() || observations.is_empty() {
            continue;
        }
        let ids = connection
            .prepare("SELECT person_id FROM person_names WHERE name=?1")?
            .query_map([person_name_key(name)], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if ids.len() > 1 {
            return Err(MeetingStoreError::Conflict(
                "Observation name matches multiple contacts; no observations were saved.".into(),
            ));
        }
        let mut value = match ids.first() {
            Some(id) => required(connection, id)?,
            None => {
                let mut value = PersonProfile::new(name);
                value.created_at = chrono::Utc::now().timestamp_millis();
                value
            }
        };
        if !value.tracked {
            continue;
        }
        value.updated_at = next_version(value.updated_at)?;
        value.observations.extend(observations);
        save(connection, &value)?;
        updated.insert(value.id.clone(), value);
    }
    Ok(updated.into_values().collect())
}

pub(super) fn match_voice(
    connection: &Connection,
    embedding: &[f32],
    threshold: f32,
) -> MeetingStoreResult<Option<PersonProfile>> {
    let norm = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 || !threshold.is_finite() {
        return Err(MeetingStoreError::InvalidInput(
            "Voice matching requires a finite nonzero vector and threshold.".into(),
        ));
    }
    let mut statement =
        connection.prepare("SELECT id,voice FROM people WHERE voice IS NOT NULL ORDER BY id")?;
    let candidates = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut best: Option<(String, f32)> = None;
    for candidate in candidates {
        let (id, voice) = candidate?;
        let voice: Vec<f32> = decode(voice)?;
        if voice.len() != embedding.len() {
            continue;
        }
        let length = voice.iter().map(|v| v * v).sum::<f32>().sqrt();
        if !length.is_finite() || length == 0.0 {
            continue;
        }
        let score = voice.iter().zip(embedding).map(|(a, b)| a * b).sum::<f32>() / (length * norm);
        if score >= threshold && best.as_ref().is_none_or(|(_, prior)| score > *prior) {
            best = Some((id, score));
        }
    }
    best.map(|(id, _)| required(connection, &id)).transpose()
}
