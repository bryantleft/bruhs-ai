use super::*;
use crate::meetings::person_changes::{merged_profile, next_version, validate_person};
use bruhs_models::{PersonObservation, PersonProfile};
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
    for name in std::iter::once(&value.display_name).chain(&value.aliases) {
        connection.execute(
            "INSERT OR IGNORE INTO person_names(name,person_id) VALUES(?1,?2)",
            [&name.to_ascii_lowercase(), &value.id],
        )?;
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
    records::retarget_meetings(connection, &source, &merged)?;
    connection.execute("INSERT INTO merged_people(source_id,target_id,source_original,target_original) VALUES(?1,?2,?3,?4)", rusqlite::params![source_id,target_id,encode(&source)?,encode(&target)?])?;
    connection.execute("DELETE FROM people WHERE id=?1", [source_id])?;
    Ok(merged)
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
            .query_map([name.to_ascii_lowercase()], |row| row.get::<_, String>(0))?
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
