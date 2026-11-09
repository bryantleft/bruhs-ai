use crate::meetings::MeetingRecords;
use crate::meetings::SpeakerLinkJobs;
use crate::meetings::{InMemoryMeetingStore, MeetingStore, SqliteMeetingStore};
use bruhs_models::Meeting;
use std::sync::Arc;

async fn stores() -> (tempfile::TempDir, Vec<Arc<dyn MeetingStore>>) {
    let dir = tempfile::tempdir().unwrap();
    let sql = SqliteMeetingStore::open(dir.path().join("jobs.sqlite3"))
        .await
        .unwrap();
    (
        dir,
        vec![Arc::new(InMemoryMeetingStore::new()), Arc::new(sql)],
    )
}

#[tokio::test]
async fn stale_success_and_failure_cannot_erase_or_delay_a_new_event() {
    let (_dir, stores) = stores().await;
    for store in stores {
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        store.enqueue_speaker_link(&meeting.id).await.unwrap();
        let old = store.next_speaker_link(0).await.unwrap().unwrap();
        for _ in 0..100 {
            store.enqueue_speaker_link(&meeting.id).await.unwrap();
        }
        let fresh = store.next_speaker_link(0).await.unwrap().unwrap();
        assert_ne!(fresh.token, old.token);
        store.finish_speaker_link(&old).await.unwrap();
        store.retry_speaker_link(&old, 50_000).await.unwrap();
        assert_eq!(
            store.next_speaker_link(0).await.unwrap(),
            Some(fresh.clone())
        );
        store.finish_speaker_link(&fresh).await.unwrap();
        assert!(store.next_speaker_link(i64::MAX).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn retries_are_delayed_capped_and_do_not_starve_other_jobs() {
    let (_dir, stores) = stores().await;
    for store in stores {
        let first = store
            .create_meeting(Meeting::new("One", 1.0))
            .await
            .unwrap();
        store.enqueue_speaker_link(&first.id).await.unwrap();
        let mut job = store.next_speaker_link(0).await.unwrap().unwrap();
        store.retry_speaker_link(&job, 100).await.unwrap();
        assert!(store.next_speaker_link(1099).await.unwrap().is_none());
        let second = store
            .create_meeting(Meeting::new("Two", 1.0))
            .await
            .unwrap();
        store.enqueue_speaker_link(&second.id).await.unwrap();
        let other = store.next_speaker_link(100).await.unwrap().unwrap();
        assert_eq!(other.meeting_id, second.id);
        store.finish_speaker_link(&other).await.unwrap();
        let saved = store.next_speaker_link(1100).await.unwrap().unwrap();
        // An older failed attempt cannot move the already-saved retry window.
        store.retry_speaker_link(&job, 100_000).await.unwrap();
        assert_eq!(
            store.next_speaker_link(1100).await.unwrap(),
            Some(saved.clone())
        );
        job = saved;
        for _ in 0..20 {
            let now = job.due_at_ms;
            store.retry_speaker_link(&job, now).await.unwrap();
            job = store.next_speaker_link(i64::MAX).await.unwrap().unwrap();
            assert!((1_000..=300_000).contains(&(job.due_at_ms - now)));
        }
        assert_eq!(job.attempts, 21);
    }
}

#[tokio::test]
async fn deletion_removes_work_and_refuses_requeue_for_a_missing_recording() {
    let (_dir, stores) = stores().await;
    for store in stores {
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        store.enqueue_speaker_link(&meeting.id).await.unwrap();
        let job = store.next_speaker_link(0).await.unwrap().unwrap();
        store
            .delete_meeting(&meeting.id.parse().unwrap())
            .await
            .unwrap();
        store.retry_speaker_link(&job, 0).await.unwrap();
        assert!(store.next_speaker_link(i64::MAX).await.unwrap().is_none());
        assert!(store.enqueue_speaker_link(&meeting.id).await.is_err());
        assert!(store.list_people().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn pending_and_backed_off_receipts_survive_database_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jobs.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    store.enqueue_speaker_link(&meeting.id).await.unwrap();
    let original = store.next_speaker_link(0).await.unwrap().unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(
        store.next_speaker_link(0).await.unwrap(),
        Some(original.clone())
    );
    store.retry_speaker_link(&original, 1_000).await.unwrap();
    let retry = store.next_speaker_link(2_000).await.unwrap().unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert!(store.next_speaker_link(1999).await.unwrap().is_none());
    assert_eq!(
        store.next_speaker_link(2000).await.unwrap(),
        Some(retry.clone())
    );
    store.finish_speaker_link(&retry).await.unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert!(store.next_speaker_link(i64::MAX).await.unwrap().is_none());
}

#[tokio::test]
async fn duplicate_capture_receipt_preserves_backoff_and_new_receipt_fences_old_ack() {
    let (_dir, stores) = stores().await;
    for store in stores {
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        let mut receipt = bruhs_models::CaptureRecoveryReceipt {
            sequence: 1,
            meeting_id: meeting.id,
            token: uuid::Uuid::new_v4().to_string(),
        };
        store.enqueue_capture_receipt(&receipt).await.unwrap();
        let original = store.next_speaker_link(0).await.unwrap().unwrap();
        store.retry_speaker_link(&original, 1_000).await.unwrap();
        let delayed = store.next_speaker_link(i64::MAX).await.unwrap().unwrap();
        for _ in 0..3 {
            store.enqueue_capture_receipt(&receipt).await.unwrap();
        }
        assert_eq!(
            store.next_speaker_link(i64::MAX).await.unwrap(),
            Some(delayed.clone())
        );
        receipt.token = uuid::Uuid::new_v4().to_string();
        store.enqueue_capture_receipt(&receipt).await.unwrap();
        let fresh = store.next_speaker_link(0).await.unwrap().unwrap();
        assert_ne!(fresh.token, delayed.token);
        store.finish_speaker_link(&delayed).await.unwrap();
        assert_eq!(store.next_speaker_link(0).await.unwrap(), Some(fresh));
    }
}

#[tokio::test]
async fn capture_receipt_backoff_survives_reopen_and_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jobs.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    let receipt = bruhs_models::CaptureRecoveryReceipt {
        sequence: 1,
        meeting_id: meeting.id,
        token: uuid::Uuid::new_v4().to_string(),
    };
    store.enqueue_capture_receipt(&receipt).await.unwrap();
    let original = store.next_speaker_link(0).await.unwrap().unwrap();
    store.retry_speaker_link(&original, 5_000).await.unwrap();
    let delayed = store.next_speaker_link(i64::MAX).await.unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    store.enqueue_capture_receipt(&receipt).await.unwrap();
    assert_eq!(store.next_speaker_link(i64::MAX).await.unwrap(), delayed);
}

#[tokio::test]
async fn invalid_and_deleted_capture_receipts_cannot_create_work() {
    let (_dir, stores) = stores().await;
    for store in stores {
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        let mut receipt = bruhs_models::CaptureRecoveryReceipt {
            sequence: 1,
            meeting_id: meeting.id,
            token: "not-a-token".into(),
        };
        assert!(store.enqueue_capture_receipt(&receipt).await.is_err());
        receipt.token = uuid::Uuid::new_v4().to_string();
        store
            .delete_meeting(&receipt.meeting_id.parse().unwrap())
            .await
            .unwrap();
        assert!(store.enqueue_capture_receipt(&receipt).await.is_err());
        assert!(store.next_speaker_link(i64::MAX).await.unwrap().is_none());
    }
}
