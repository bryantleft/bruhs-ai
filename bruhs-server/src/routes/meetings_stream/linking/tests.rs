use super::*;
use bruhs_core::SqliteMeetingStore;
use bruhs_models::{ClusterCentroid, Meeting, Speaker};

fn centroid(label: &str, automatic: bool) -> CentroidsFile {
    CentroidsFile {
        linking_auto: automatic,
        centroids: vec![ClusterCentroid {
            cluster_label: label.into(),
            embedding: vec![1.0, 0.0],
            first_seen_ms: 0,
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn concurrent_and_reopened_manual_replays_do_not_create_orphan_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.sqlite");
    let app = AppState::builder()
        .meetings(Arc::new(SqliteMeetingStore::open(&path).await.unwrap()))
        .build();
    let meeting = app
        .meetings()
        .create_meeting(Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    let mut file = centroid("them_1", false);
    file.centroids.push(file.centroids[0].clone());
    let (first, second) = tokio::join!(
        link_centroids(&app, &meeting.id, &file),
        link_centroids(&app, &meeting.id, &file)
    );
    assert_ne!(first.unwrap(), second.unwrap());
    let saved = app
        .meetings()
        .get_meeting(&meeting.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .speakers
            .iter()
            .filter(|speaker| speaker.id == "them_1")
            .count(),
        1
    );
    let people = app.meetings().list_people().await.unwrap();
    assert_eq!(people.len(), 1);
    assert!(people[0].created_at > 0);
    assert!(people[0].updated_at >= people[0].created_at);
    drop(app);
    let reopened = AppState::builder()
        .meetings(Arc::new(SqliteMeetingStore::open(&path).await.unwrap()))
        .build();
    assert!(!link_centroids(&reopened, &meeting.id, &file).await.unwrap());
    assert_eq!(reopened.meetings().list_people().await.unwrap().len(), 1);
    assert_eq!(
        reopened
            .meetings()
            .get_meeting(&meeting.id)
            .await
            .unwrap()
            .unwrap(),
        saved
    );
}

#[tokio::test]
async fn automatic_and_named_identity_selection_is_serialized_across_meetings() {
    for label in ["them_1", "name:Alice"] {
        let app = AppState::new();
        let first = app
            .meetings()
            .create_meeting(Meeting::new("One", 1.0))
            .await
            .unwrap();
        let second = app
            .meetings()
            .create_meeting(Meeting::new("Two", 2.0))
            .await
            .unwrap();
        let file = centroid(label, true);
        let (one, two) = tokio::join!(
            link_centroids(&app, &first.id, &file),
            link_centroids(&app, &second.id, &file)
        );
        assert!(one.unwrap());
        assert!(two.unwrap());
        let people = app.meetings().list_people().await.unwrap();
        assert_eq!(people.len(), 1);
        assert!(people[0].created_at > 0);
        assert!(people[0].updated_at >= people[0].created_at);
        let one = app
            .meetings()
            .get_meeting(&first.id)
            .await
            .unwrap()
            .unwrap();
        let two = app
            .meetings()
            .get_meeting(&second.id)
            .await
            .unwrap()
            .unwrap();
        let one = one
            .speakers
            .iter()
            .find(|speaker| speaker.id == label)
            .unwrap();
        let two = two
            .speakers
            .iter()
            .find(|speaker| speaker.id == label)
            .unwrap();
        assert!(one.person_id.is_some());
        assert_eq!(one.person_id, two.person_id);
    }
}

#[tokio::test]
async fn missing_meeting_and_invalid_centroids_do_not_create_profiles() {
    let app = AppState::new();
    assert!(link_centroids(&app, "missing", &centroid("them_1", false))
        .await
        .is_err());
    let meeting = app
        .meetings()
        .create_meeting(Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    for label in ["", "name:"] {
        assert!(link_centroids(&app, &meeting.id, &centroid(label, false))
            .await
            .is_err());
    }
    let mut invalid = centroid("them_1", false);
    invalid.linking_threshold = f32::NAN;
    assert!(link_centroids(&app, &meeting.id, &invalid).await.is_err());
    assert!(app.meetings().list_people().await.unwrap().is_empty());
}

#[tokio::test]
async fn replay_preserves_existing_manual_speaker_choices() {
    let app = AppState::new();
    let mut meeting = Meeting::new("Synthetic", 1.0);
    meeting.speakers.push(Speaker {
        id: "them_1".into(),
        label: "User choice".into(),
        person_id: None,
        color: Some("custom".into()),
    });
    let saved = app.meetings().create_meeting(meeting).await.unwrap();
    assert!(!link_centroids(&app, &saved.id, &centroid("them_1", false))
        .await
        .unwrap());
    assert_eq!(
        app.meetings()
            .get_meeting(&saved.id)
            .await
            .unwrap()
            .unwrap(),
        saved
    );
    assert!(app.meetings().list_people().await.unwrap().is_empty());
}
