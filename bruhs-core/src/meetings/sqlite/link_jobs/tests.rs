use crate::meetings::MeetingRecords;
use crate::meetings::SpeakerLinkJobs;
use crate::meetings::SqliteMeetingStore;
use bruhs_models::Meeting;

#[tokio::test]
async fn schema_two_upgrade_preserves_records_and_adds_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Preserved", 1.0))
        .await
        .unwrap();
    store
        .call(|connection| {
            connection.execute_batch(
                "DROP TABLE segment_removals; DROP TABLE local_focus; DROP TABLE speaker_link_jobs; PRAGMA user_version=2;",
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
    store.enqueue_speaker_link(&meeting.id).await.unwrap();
    assert_eq!(
        store
            .next_speaker_link(0)
            .await
            .unwrap()
            .unwrap()
            .meeting_id,
        meeting.id
    );
}

#[tokio::test]
async fn failed_terminal_acknowledgment_preserves_the_receipt_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Preserved", 1.0))
        .await
        .unwrap();
    store.enqueue_speaker_link(&meeting.id).await.unwrap();
    let job = store.next_speaker_link(0).await.unwrap().unwrap();
    store.call(|connection| {
        connection.execute_batch("CREATE TABLE ack_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE ack_failure(id INTEGER REFERENCES ack_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_ack AFTER DELETE ON speaker_link_jobs BEGIN INSERT INTO ack_failure VALUES(1); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(store.finish_speaker_link(&job).await.is_err());
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(store.next_speaker_link(0).await.unwrap(), Some(job));
}

#[tokio::test]
async fn independent_connections_cannot_acknowledge_a_newer_enqueue() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.sqlite3");
    let first = SqliteMeetingStore::open(&path).await.unwrap();
    let second = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = first
        .create_meeting(Meeting::new("Preserved", 1.0))
        .await
        .unwrap();
    first.enqueue_speaker_link(&meeting.id).await.unwrap();
    let old = first.next_speaker_link(0).await.unwrap().unwrap();
    second.enqueue_speaker_link(&meeting.id).await.unwrap();
    let newer = second.next_speaker_link(0).await.unwrap().unwrap();
    first.finish_speaker_link(&old).await.unwrap();
    first.retry_speaker_link(&old, 5000).await.unwrap();
    assert_eq!(second.next_speaker_link(0).await.unwrap(), Some(newer));
}
