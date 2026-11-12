use super::*;
use crate::meetings::MeetingRecords;
use crate::meetings::PeopleStore;
use bruhs_models::{Meeting, ObservationKind, PersonObservation, PersonProfile, TranscriptSegment};

async fn fixture() -> (tempfile::TempDir, SqliteMeetingStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
        .await
        .unwrap();
    (directory, store)
}
fn observation() -> PersonObservation {
    PersonObservation {
        at_meeting_id: "recorded-meeting".into(),
        kind: ObservationKind::Topic,
        text: "Private observation".into(),
        created_at: 1,
    }
}
async fn pair(store: &SqliteMeetingStore) -> (PersonProfile, PersonProfile) {
    let mut source = PersonProfile::new("Source");
    source.observations.push(observation());
    source.aliases.push("Former source".into());
    source.tracked = false;
    let target = PersonProfile::new("Target");
    store.upsert_person(source.clone()).await.unwrap();
    store.upsert_person(target.clone()).await.unwrap();
    (source, target)
}

#[tokio::test]
async fn repeated_contact_references_preserve_relations_retargeting_and_missing_contact_rollback() {
    let (directory, store) = fixture().await;
    let (source, target) = pair(&store).await;
    let mut meeting = Meeting::new("Repeated assignments", 1.0);
    meeting.updated_at = 4_000_000_000_000;
    for speaker in &mut meeting.speakers {
        speaker.person_id = Some(source.id.clone());
        speaker.label.clone_from(&source.display_name);
    }
    meeting.summary = Some(bruhs_models::MeetingSummary {
        tldr: "Preserve every reference".into(),
        action_items: [&source.id, &source.id, &target.id]
            .into_iter()
            .map(|id| bruhs_models::ActionItem {
                text: "Assigned action".into(),
                assignee_person_id: Some(id.clone()),
                due_hint: None,
            })
            .collect(),
        decisions: Vec::new(),
        topics: Vec::new(),
        generated_by: "fixture".into(),
        generated_at: 1,
    });
    assert_eq!(
        store.create_meeting(meeting.clone()).await.unwrap(),
        meeting
    );
    let mut expected_ids = vec![source.id.clone(), target.id.clone()];
    expected_ids.sort();
    assert_eq!(relation_ids(&store, &meeting.id).await, expected_ids);

    let mut invalid = meeting.clone();
    invalid.id = "missing-contact-rollback".into();
    invalid
        .summary
        .as_mut()
        .unwrap()
        .action_items
        .push(bruhs_models::ActionItem {
            text: "Missing assignment".into(),
            assignee_person_id: Some("missing".into()),
            due_hint: None,
        });
    assert!(matches!(
        store.create_meeting(invalid.clone()).await,
        Err(MeetingStoreError::PersonNotFound { id }) if id == "missing"
    ));
    assert!(store.get_meeting(&invalid.id).await.unwrap().is_none());
    assert!(relation_ids(&store, &invalid.id).await.is_empty());
    assert_eq!(relation_ids(&store, &meeting.id).await, expected_ids);

    store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    for speaker in &mut meeting.speakers {
        speaker.person_id = Some(target.id.clone());
        speaker.label.clone_from(&target.display_name);
    }
    for action in &mut meeting.summary.as_mut().unwrap().action_items {
        action.assignee_person_id = Some(target.id.clone());
    }
    meeting.updated_at += 1;
    drop(store);
    let reopened = SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
        .await
        .unwrap();
    assert_eq!(
        reopened.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    assert_eq!(relation_ids(&reopened, &meeting.id).await, vec![target.id]);
}

async fn relation_ids(store: &SqliteMeetingStore, meeting_id: &str) -> Vec<String> {
    let id = meeting_id.to_owned();
    store
        .call(move |connection| {
            connection
                .prepare(
                    "SELECT person_id FROM meeting_people WHERE meeting_id=?1 ORDER BY person_id",
                )?
                .query_map([id], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()
                .map_err(MeetingStoreError::from)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn restart_preserves_contacts_merge_receipt_originals_and_linked_meetings() {
    let (directory, store) = fixture().await;
    let (source, target) = pair(&store).await;
    let meeting = Meeting::new("Linked meeting", 1.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    store
        .link_speaker_to_person(
            &meeting.id.parse().unwrap(),
            &"them".parse().unwrap(),
            &source.id.parse().unwrap(),
        )
        .await
        .unwrap();
    let merged = store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    drop(store);
    let reopened = SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
        .await
        .unwrap();
    assert_eq!(reopened.list_people().await.unwrap(), vec![merged.clone()]);
    assert_eq!(
        reopened
            .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
            .await
            .unwrap(),
        merged
    );
    let linked = reopened.get_meeting(&meeting.id).await.unwrap().unwrap();
    assert_eq!(
        linked.speakers[1].person_id.as_deref(),
        Some(target.id.as_str())
    );
    let originals = reopened
        .call(|connection| {
            let (a, b): (String, String) = connection.query_row(
                "SELECT source_original,target_original FROM merged_people",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            Ok((decode::<PersonProfile>(a)?, decode::<PersonProfile>(b)?))
        })
        .await
        .unwrap();
    assert_eq!(originals, (source.clone(), target));
    assert!(reopened.upsert_person(source).await.is_err());
}

#[tokio::test]
async fn independent_connections_merge_once_and_refuse_delayed_snapshots() {
    let (directory, first) = fixture().await;
    let (source, target) = pair(&first).await;
    let second = SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
        .await
        .unwrap();
    let (source_id, target_id) = (source.id.parse().unwrap(), target.id.parse().unwrap());
    let (a, b) = tokio::join!(
        first.merge_person_into(&source_id, &target_id),
        second.merge_person_into(&source_id, &target_id)
    );
    assert_eq!(a.unwrap(), b.unwrap());
    let mut stale = target;
    stale.updated_at = i64::MAX;
    assert!(second.upsert_person(stale).await.is_err());
    assert_eq!(
        first.list_people().await.unwrap()[0].observations,
        source.observations
    );
}

#[tokio::test]
async fn terminal_commit_failure_rolls_back_target_source_and_merge_receipt() {
    let (_directory, store) = fixture().await;
    let (source, target) = pair(&store).await;
    let before = store.list_people().await.unwrap();
    store.call(|connection| {
        connection.execute_batch("CREATE TABLE commit_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE commit_failure(id INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_merge_commit AFTER INSERT ON merged_people BEGIN INSERT INTO commit_failure VALUES(1); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .is_err());
    assert_eq!(store.list_people().await.unwrap(), before);
    let receipts: i64 = store
        .call(|connection| {
            Ok(connection.query_row("SELECT count(*) FROM merged_people", [], |row| row.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(receipts, 0);
    store
        .call(|connection| {
            connection.execute_batch("DROP TRIGGER fail_merge_commit")?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .is_ok());
}

#[tokio::test]
async fn unrelated_field_updates_and_enrolled_voice_survive_background_fill() {
    let (_directory, store) = fixture().await;
    let mut original = PersonProfile::new("Original");
    original.updated_at = i64::MAX - 10;
    store.upsert_person(original.clone()).await.unwrap();
    store
        .upsert_voice_embedding(&original.id.parse().unwrap(), vec![1.0, 0.0])
        .await
        .unwrap();
    let renamed = store
        .rename_person(&original.id.parse().unwrap(), "Renamed".into())
        .await
        .unwrap();
    let filled = store
        .fill_voice_embedding(&original.id.parse().unwrap(), vec![0.0, 1.0])
        .await
        .unwrap();
    assert_eq!(renamed, filled);
    assert!(renamed.updated_at > original.updated_at);
    assert_eq!(renamed.voice_embedding, Some(vec![1.0, 0.0]));
}

#[tokio::test]
async fn segment_replays_are_idempotent_and_delete_cascades_after_restart() {
    let (directory, store) = fixture().await;
    let meeting = Meeting::new("Transcript", 1.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    let mut segment = TranscriptSegment {
        id: "segment".into(),
        meeting_id: meeting.id.clone(),
        speaker_id: None,
        start_ms: 0,
        end_ms: 10,
        text: "Partial".into(),
        confidence: 0.9,
        partial: true,
        speaker_confidence: None,
    };
    store.append_segment(segment.clone()).await.unwrap();
    segment.partial = false;
    segment.text = "Final text".into();
    store.append_segment(segment.clone()).await.unwrap();
    store.append_segment(segment.clone()).await.unwrap();
    drop(store);
    let reopened = SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
        .await
        .unwrap();
    assert_eq!(
        reopened.list_segments(&meeting.id).await.unwrap(),
        vec![segment.clone()]
    );
    reopened
        .delete_meeting(&meeting.id.parse().unwrap())
        .await
        .unwrap();
    assert!(reopened
        .list_segments(&meeting.id)
        .await
        .unwrap()
        .is_empty());
    assert!(reopened.append_segment(segment).await.is_err());
}

#[tokio::test]
async fn malformed_payload_stays_intact_and_does_not_become_an_empty_list() {
    let (_directory, store) = fixture().await;
    let (_, target) = pair(&store).await;
    let id = target.id.clone();
    store
        .call(move |connection| {
            connection.execute("UPDATE people SET payload='broken JSON' WHERE id=?1", [id])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(store.list_people().await.is_err());
    assert!(store
        .rename_person(&target.id.parse().unwrap(), "Replacement".into())
        .await
        .is_err());
    let raw: String = store
        .call(move |connection| {
            Ok(connection.query_row(
                "SELECT payload FROM people WHERE id=?1",
                [&target.id],
                |row| row.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(raw, "broken JSON");
}

#[tokio::test]
async fn observation_alias_lookup_respects_merge_opt_out_after_restart() {
    let (directory, store) = fixture().await;
    let (source, target) = pair(&store).await;
    store
        .merge_person_into(&source.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    drop(store);
    let reopened = SqliteMeetingStore::open(directory.path().join("meetings.sqlite3"))
        .await
        .unwrap();
    assert!(reopened
        .append_person_observations(std::collections::HashMap::from([(
            "Former source".into(),
            vec![observation()]
        )]))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened.list_people().await.unwrap()[0].observations.len(),
        1
    );
}

#[tokio::test]
async fn unknown_database_version_is_preserved_without_reset() {
    let (directory, store) = fixture().await;
    store
        .call(|connection| {
            connection.pragma_update(None, "user_version", 999)?;
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    let path = directory.path().join("meetings.sqlite3");
    let before = std::fs::read(&path).unwrap();
    assert!(SqliteMeetingStore::open(&path).await.is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
