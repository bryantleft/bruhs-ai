use super::*;
use bruhs_models::{LocalFocusObservation, MAX_LOCAL_FOCUS_OBSERVATIONS};

pub(super) fn append(
    connection: &Connection,
    sample: &LocalFocusObservation,
) -> MeetingStoreResult<()> {
    if !sample.is_valid() {
        return Err(MeetingStoreError::InvalidInput(
            "Invalid local focus metadata.".into(),
        ));
    }
    records::required(
        records::meeting(connection, &sample.meeting_id)?,
        &sample.meeting_id,
    )?;
    if let Some(existing) = get::<LocalFocusObservation>(connection, "local_focus", &sample.id)? {
        return if &existing == sample {
            Ok(())
        } else {
            Err(MeetingStoreError::Conflict(
                "Local focus identity already used.".into(),
            ))
        };
    }
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM local_focus WHERE meeting_id=?1",
        [&sample.meeting_id],
        |r| r.get(0),
    )?;
    if usize::try_from(count).is_ok_and(|count| count >= MAX_LOCAL_FOCUS_OBSERVATIONS) {
        return Err(MeetingStoreError::Conflict(
            "Local focus timeline is full; tracking must stop.".into(),
        ));
    }
    connection.execute(
        "INSERT INTO local_focus (id,meeting_id,observed,payload) VALUES (?1,?2,?3,?4)",
        rusqlite::params![
            sample.id,
            sample.meeting_id,
            sample.observed_at_ms,
            encode(sample)?
        ],
    )?;
    Ok(())
}

pub(super) fn list(
    connection: &Connection,
    meeting_id: &str,
) -> MeetingStoreResult<Vec<LocalFocusObservation>> {
    let mut statement = connection.prepare("SELECT id,observed,payload FROM local_focus WHERE meeting_id=?1 ORDER BY observed,id LIMIT ?2")?;
    let rows = statement.query_map(
        rusqlite::params![
            meeting_id,
            i64::try_from(MAX_LOCAL_FOCUS_OBSERVATIONS).unwrap_or(i64::MAX)
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        },
    )?;
    rows.map(|row| {
        let (id, observed, payload) = row?;
        let sample: LocalFocusObservation = decode(payload)?;
        if !sample.is_valid()
            || sample.id != id
            || sample.meeting_id != meeting_id
            || sample.observed_at_ms != observed
        {
            return Err(MeetingStoreError::PersistenceFailed(
                "Invalid saved local focus metadata; original data preserved.".into(),
            ));
        }
        Ok(sample)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meetings::MeetingRecords;

    #[tokio::test]
    async fn local_focus_schema_three_upgrade_and_quota_preserve_old_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.sqlite3");
        let store = SqliteMeetingStore::open(&path).await.unwrap();
        let meeting = bruhs_models::Meeting::new("Retained meeting", 1.0);
        store.create_meeting(meeting.clone()).await.unwrap();
        store
            .call(|connection| {
                connection.execute_batch(
                    "DROP TABLE segment_removals; DROP TABLE local_focus; PRAGMA user_version=3;",
                )?;
                Ok(())
            })
            .await
            .unwrap();
        drop(store);
        let store = SqliteMeetingStore::open(&path).await.unwrap();
        assert_eq!(
            store.get_meeting(&meeting.id).await.unwrap(),
            Some(meeting.clone())
        );
        let id = meeting.id.clone();
        store.call(move |connection| write(connection, |connection| {
            let mut insert = connection.prepare("INSERT INTO local_focus (id,meeting_id,observed,payload) VALUES (?1,?2,1,?3)")?;
            for index in 0..MAX_LOCAL_FOCUS_OBSERVATIONS {
                let sample = LocalFocusObservation { id: index.to_string(), meeting_id: id.clone(), observed_at_ms: 1, focus: bruhs_models::LocalFocus::Unavailable {} };
                insert.execute(rusqlite::params![sample.id, id, encode(&sample)?])?;
            }
            Ok(())
        })).await.unwrap();
        let sample = LocalFocusObservation {
            id: "over-limit".into(),
            meeting_id: meeting.id.clone(),
            observed_at_ms: 2,
            focus: bruhs_models::LocalFocus::Unavailable {},
        };
        assert!(matches!(
            store.append_local_focus(sample).await,
            Err(MeetingStoreError::Conflict(_))
        ));
        assert_eq!(
            store.list_local_focus(&meeting.id).await.unwrap().len(),
            MAX_LOCAL_FOCUS_OBSERVATIONS
        );
    }
}
