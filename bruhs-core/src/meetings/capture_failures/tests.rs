use crate::meetings::MeetingPatch;
use crate::meetings::MeetingRecords;
use crate::meetings::{InMemoryMeetingStore, MeetingStore, SqliteMeetingStore};
use bruhs_models::{CaptureFailureReceipt, Meeting, MeetingStatus};
use std::sync::Arc;

fn receipt(id: &str) -> CaptureFailureReceipt {
    CaptureFailureReceipt {
        sequence: 1,
        token: uuid::Uuid::new_v4().to_string(),
        meeting_id: id.into(),
        started_at_ms: 10.0,
        failed_at_ms: 20.0,
    }
}

#[tokio::test]
async fn postprocessing_failure_preserves_successful_stop_time_and_existing_content() {
    let dir = tempfile::tempdir().unwrap();
    let stores: Vec<Arc<dyn MeetingStore>> = vec![
        Arc::new(InMemoryMeetingStore::new()),
        Arc::new(
            SqliteMeetingStore::open(dir.path().join("ended.sqlite3"))
                .await
                .unwrap(),
        ),
    ];
    for store in stores {
        let mut meeting = Meeting::new("Retain recorded meeting", 10.0);
        meeting.status = MeetingStatus::Ended;
        meeting.ended_at_ms = Some(15.0);
        meeting.audio_path = Some("synthetic/mixed.wav".into());
        let meeting = store.create_meeting(meeting).await.unwrap();
        assert!(store
            .apply_capture_failure(&receipt(&meeting.id))
            .await
            .unwrap());
        let failed = store.get_meeting(&meeting.id).await.unwrap().unwrap();
        let mut expected = meeting;
        expected.status = MeetingStatus::Failed;
        expected.updated_at = failed.updated_at;
        assert_eq!(
            failed, expected,
            "Only status/version may change after a successfully stopped recording"
        );
        assert!(!store
            .apply_capture_failure(&receipt(&failed.id))
            .await
            .unwrap());
        assert_eq!(
            store.get_meeting(&failed.id).await.unwrap().unwrap(),
            failed
        );
    }
}

#[tokio::test]
async fn failure_is_durable_idempotent_and_survives_a_late_successful_stop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failures.sqlite3");
    let stores: Vec<Arc<dyn MeetingStore>> = vec![
        Arc::new(InMemoryMeetingStore::new()),
        Arc::new(SqliteMeetingStore::open(&path).await.unwrap()),
    ];
    let mut saved_id = String::new();
    for store in stores {
        let meeting = store
            .create_meeting(Meeting::new("Retained title", 10.0))
            .await
            .unwrap();
        let receipt = receipt(&meeting.id);
        assert!(store.apply_capture_failure(&receipt).await.unwrap());
        let failed = store.get_meeting(&meeting.id).await.unwrap().unwrap();
        assert_eq!(failed.status, MeetingStatus::Failed);
        assert_eq!(failed.ended_at_ms, Some(20.0));
        assert_eq!(failed.title, meeting.title);
        assert_eq!(failed.speakers, meeting.speakers);
        assert!(!store.apply_capture_failure(&receipt).await.unwrap());
        assert_eq!(
            store.get_meeting(&meeting.id).await.unwrap().unwrap(),
            failed
        );
        let late = store
            .patch_meeting(
                &meeting.id.parse().unwrap(),
                MeetingPatch {
                    title: Some("Renamed".into()),
                    status: Some(MeetingStatus::Ended),
                    ended_at_ms: Some(30.0),
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(late.status, MeetingStatus::Failed);
        assert_eq!(late.ended_at_ms, Some(20.0));
        assert_eq!(late.title, "Renamed");
        saved_id = meeting.id;
    }
    let reopened = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .get_meeting(&saved_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        MeetingStatus::Failed
    );
}

#[tokio::test]
async fn stale_invalid_and_deleted_attempts_cannot_modify_a_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let stores: Vec<Arc<dyn MeetingStore>> = vec![
        Arc::new(InMemoryMeetingStore::new()),
        Arc::new(
            SqliteMeetingStore::open(dir.path().join("failures.sqlite3"))
                .await
                .unwrap(),
        ),
    ];
    for store in stores {
        let meeting = store
            .create_meeting(Meeting::new("New attempt", 11.0))
            .await
            .unwrap();
        let mut receipt = receipt(&meeting.id);
        assert!(!store.apply_capture_failure(&receipt).await.unwrap());
        receipt.started_at_ms = f64::NAN;
        assert!(store.apply_capture_failure(&receipt).await.is_err());
        assert_eq!(
            store.get_meeting(&meeting.id).await.unwrap().unwrap(),
            meeting
        );
        receipt.started_at_ms = 11.0;
        store
            .delete_meeting(&meeting.id.parse().unwrap())
            .await
            .unwrap();
        assert!(store.apply_capture_failure(&receipt).await.is_err());
        assert!(store.get_meeting(&meeting.id).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn failed_status_commit_rolls_back_without_losing_original_meeting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("failure.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Retained", 10.0))
        .await
        .unwrap();
    let fault = rusqlite::Connection::open(&path).unwrap();
    fault.execute_batch("CREATE TRIGGER synthetic_failure BEFORE UPDATE ON meetings BEGIN SELECT RAISE(ABORT, 'synthetic disk failure'); END;").unwrap();
    assert!(store
        .apply_capture_failure(&receipt(&meeting.id))
        .await
        .is_err());
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap().unwrap(),
        meeting
    );
    fault
        .execute_batch("DROP TRIGGER synthetic_failure;")
        .unwrap();
    assert!(store
        .apply_capture_failure(&receipt(&meeting.id))
        .await
        .unwrap());
}
