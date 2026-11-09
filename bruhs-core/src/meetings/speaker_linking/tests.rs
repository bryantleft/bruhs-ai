use super::*;
use crate::meetings::{InMemoryMeetingStore, MeetingStore, SqliteMeetingStore};
use bruhs_models::ClusterCentroid;
use std::sync::Arc;

async fn stores() -> (tempfile::TempDir, Vec<Arc<dyn MeetingStore>>) {
    let directory = tempfile::tempdir().unwrap();
    let sqlite = SqliteMeetingStore::open(directory.path().join("synthetic.sqlite"))
        .await
        .unwrap();
    (
        directory,
        vec![Arc::new(InMemoryMeetingStore::new()), Arc::new(sqlite)],
    )
}

fn file(labels: &[&str]) -> CentroidsFile {
    CentroidsFile {
        linking_auto: false,
        centroids: labels
            .iter()
            .map(|label| ClusterCentroid {
                cluster_label: (*label).into(),
                embedding: vec![1.0, 0.0],
                first_seen_ms: 0,
            })
            .collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn later_invalid_cluster_rolls_back_new_profiles_voice_fills_and_meeting() {
    let (_directory, stores) = stores().await;
    for store in stores {
        let person = PersonProfile::new("Alice");
        store.upsert_person(person.clone()).await.unwrap();
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        let centroids = file(&["name:Alice", "them_1", "name:"]);
        assert!(store
            .link_speaker_centroids(&meeting.id, &centroids)
            .await
            .is_err());
        assert_eq!(store.list_people().await.unwrap(), vec![person]);
        assert_eq!(
            store.get_meeting(&meeting.id).await.unwrap(),
            Some(meeting.clone())
        );
        assert!(store
            .link_speaker_centroids(&meeting.id, &file(&["name:Alice", "them_1"]))
            .await
            .unwrap());
        let people = store.list_people().await.unwrap();
        assert_eq!(people.len(), 2);
        assert_eq!(
            people
                .iter()
                .find(|person| person.display_name == "Alice")
                .unwrap()
                .voice_embedding,
            Some(vec![1.0, 0.0])
        );
    }
}

#[tokio::test]
async fn exhausted_meeting_version_does_not_leak_staged_profile() {
    let (_directory, stores) = stores().await;
    for store in stores {
        let mut meeting = Meeting::new("Synthetic", 1.0);
        meeting.updated_at = i64::MAX;
        store.create_meeting(meeting.clone()).await.unwrap();
        assert!(store
            .link_speaker_centroids(&meeting.id, &file(&["them_1"]))
            .await
            .is_err());
        assert!(store.list_people().await.unwrap().is_empty());
        assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
    }
}

#[tokio::test]
async fn ambiguous_name_refuses_to_guess_and_preserves_whole_batch() {
    let (_directory, stores) = stores().await;
    for store in stores {
        for name in ["Alice", "ALICE"] {
            store.upsert_person(PersonProfile::new(name)).await.unwrap();
        }
        let before = store.list_people().await.unwrap();
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        assert!(store
            .link_speaker_centroids(&meeting.id, &file(&["them_1", "name:alice"]))
            .await
            .is_err());
        assert_eq!(store.list_people().await.unwrap(), before);
        assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), Some(meeting));
    }
}

#[tokio::test]
async fn cosine_matching_is_normalized_and_zero_vectors_are_rejected_in_both_stores() {
    let (_directory, stores) = stores().await;
    for store in stores {
        let mut person = PersonProfile::new("Known voice");
        person.voice_embedding = Some(vec![4.0, 0.0]);
        store.upsert_person(person.clone()).await.unwrap();
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        let mut centroids = file(&["them_1", "them_2"]);
        centroids.linking_auto = true;
        centroids.centroids[0].embedding = vec![2.0, 0.0];
        centroids.centroids[1].embedding = vec![0.0, 0.0];
        assert!(store
            .link_speaker_centroids(&meeting.id, &centroids)
            .await
            .is_err());
        assert_eq!(
            store.get_meeting(&meeting.id).await.unwrap(),
            Some(meeting.clone())
        );
        centroids.centroids.pop();
        assert!(store
            .link_speaker_centroids(&meeting.id, &centroids)
            .await
            .unwrap());
        let linked = store.get_meeting(&meeting.id).await.unwrap().unwrap();
        assert_eq!(
            linked.speakers.last().unwrap().person_id,
            Some(person.id.clone())
        );
        assert_eq!(store.list_people().await.unwrap(), vec![person]);
    }
}

#[tokio::test]
async fn named_numbered_profiles_cannot_collide_with_later_manual_clusters() {
    let (_directory, stores) = stores().await;
    for store in stores {
        store
            .upsert_person(PersonProfile::new("Speaker 8"))
            .await
            .unwrap();
        let meeting = store
            .create_meeting(Meeting::new("Synthetic", 1.0))
            .await
            .unwrap();
        assert!(store
            .link_speaker_centroids(&meeting.id, &file(&["them_1", "name:Speaker 10", "them_2"]))
            .await
            .unwrap());
        let linked = store.get_meeting(&meeting.id).await.unwrap().unwrap();
        let names: Vec<_> = linked
            .speakers
            .iter()
            .skip(2)
            .map(|speaker| speaker.label.as_str())
            .collect();
        assert_eq!(names, vec!["Speaker 9", "Speaker 10", "Speaker 11"]);
    }
}
