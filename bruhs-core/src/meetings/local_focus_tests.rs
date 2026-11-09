use super::*;
use bruhs_models::{LocalFocus, LocalFocusObservation};

async fn exercise(store: &dyn MeetingStore) {
    let meeting = Meeting::new("Synthetic meeting", 1.0);
    let other = Meeting::new("Unrelated meeting", 1.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    store.create_meeting(other.clone()).await.unwrap();
    let sample = LocalFocusObservation {
        id: "focus-1".into(),
        meeting_id: meeting.id.clone(),
        observed_at_ms: 2,
        focus: LocalFocus::Available {
            app_name: "Synthetic editor".into(),
            bundle_id: Some("test.editor".into()),
        },
    };
    store.append_local_focus(sample.clone()).await.unwrap();
    store.append_local_focus(sample.clone()).await.unwrap();
    assert_eq!(
        store.list_local_focus(&meeting.id).await.unwrap(),
        vec![sample.clone()]
    );
    assert!(store.list_local_focus(&other.id).await.unwrap().is_empty());
    let mut conflicting = sample.clone();
    conflicting.meeting_id.clone_from(&other.id);
    assert!(matches!(
        store.append_local_focus(conflicting).await,
        Err(MeetingStoreError::Conflict(_))
    ));
    let mut invalid = sample.clone();
    invalid.observed_at_ms = -1;
    assert!(matches!(
        store.append_local_focus(invalid).await,
        Err(MeetingStoreError::InvalidInput(_))
    ));
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone()),
        "focus must not alter AI summary inputs"
    );
    assert!(store.list_segments(&meeting.id).await.unwrap().is_empty());
    store
        .delete_meeting(&meeting.id.parse().unwrap())
        .await
        .unwrap();
    assert!(store
        .list_local_focus(&meeting.id)
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        store.append_local_focus(sample).await,
        Err(MeetingStoreError::MeetingNotFound { .. })
    ));
}

#[tokio::test]
async fn local_focus_memory_contract() {
    exercise(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn local_focus_sqlite_contract_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    exercise(&store).await;
    let meeting = Meeting::new("Retained context", 1.0);
    store.create_meeting(meeting.clone()).await.unwrap();
    let sample = LocalFocusObservation {
        id: "retained".into(),
        meeting_id: meeting.id.clone(),
        observed_at_ms: 3,
        focus: LocalFocus::Unavailable {},
    };
    store.append_local_focus(sample.clone()).await.unwrap();
    drop(store);
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(
        store.list_local_focus(&meeting.id).await.unwrap(),
        vec![sample]
    );
}
