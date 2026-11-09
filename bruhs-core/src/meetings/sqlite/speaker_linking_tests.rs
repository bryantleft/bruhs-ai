use super::*;
use crate::meetings::MeetingRecords;
use crate::meetings::PeopleStore;
use crate::meetings::SpeakerLinkJobs;
use bruhs_models::{CentroidsFile, ClusterCentroid, Meeting};

fn centroids() -> CentroidsFile {
    CentroidsFile {
        linking_auto: false,
        centroids: vec![ClusterCentroid {
            cluster_label: "them_1".into(),
            embedding: vec![1.0, 0.0],
            first_seen_ms: 0,
        }],
        ..Default::default()
    }
}

#[tokio::test]
async fn terminal_commit_failure_rolls_back_profiles_and_speaker_attachments() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.sqlite");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = store
        .create_meeting(Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    store.call(|connection| {
        connection.execute_batch("CREATE TABLE link_parent(id INTEGER PRIMARY KEY);
            CREATE TABLE link_failure(id INTEGER REFERENCES link_parent(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_link_commit AFTER INSERT ON meeting_people BEGIN INSERT INTO link_failure VALUES(1); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(store
        .link_speaker_centroids(&meeting.id, &centroids())
        .await
        .is_err());
    assert!(store.list_people().await.unwrap().is_empty());
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    drop(store);
    let reopened = SqliteMeetingStore::open(&path).await.unwrap();
    assert!(reopened.list_people().await.unwrap().is_empty());
    assert_eq!(
        reopened.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    reopened
        .call(|connection| {
            connection.execute_batch("DROP TRIGGER fail_link_commit")?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(reopened
        .link_speaker_centroids(&meeting.id, &centroids())
        .await
        .unwrap());
    assert_eq!(reopened.list_people().await.unwrap().len(), 1);
}

#[tokio::test]
async fn separate_connections_select_and_attach_exactly_one_profile_on_replay() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("synthetic.sqlite");
    let first = SqliteMeetingStore::open(&path).await.unwrap();
    let second = SqliteMeetingStore::open(&path).await.unwrap();
    let meeting = first
        .create_meeting(Meeting::new("Synthetic", 1.0))
        .await
        .unwrap();
    let file = centroids();
    let (one, two) = tokio::join!(
        first.link_speaker_centroids(&meeting.id, &file),
        second.link_speaker_centroids(&meeting.id, &file),
    );
    assert_ne!(one.unwrap(), two.unwrap());
    let linked = first.get_meeting(&meeting.id).await.unwrap().unwrap();
    let people = first.list_people().await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(
        linked
            .speakers
            .iter()
            .filter(|speaker| speaker.id == "them_1")
            .count(),
        1
    );
    assert_eq!(
        linked.speakers.last().unwrap().person_id.as_deref(),
        Some(people[0].id.as_str())
    );
    drop(first);
    drop(second);
    let reopened = SqliteMeetingStore::open(&path).await.unwrap();
    assert!(!reopened
        .link_speaker_centroids(&meeting.id, &file)
        .await
        .unwrap());
    assert_eq!(reopened.list_people().await.unwrap(), people);
    assert_eq!(
        reopened.get_meeting(&meeting.id).await.unwrap(),
        Some(linked)
    );
}

#[tokio::test]
async fn separate_connections_share_voice_matches_and_reserve_manual_numbers() {
    for automatic in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("synthetic.sqlite");
        let first = SqliteMeetingStore::open(&path).await.unwrap();
        let second = SqliteMeetingStore::open(&path).await.unwrap();
        let one = first
            .create_meeting(Meeting::new("One", 1.0))
            .await
            .unwrap();
        let two = first
            .create_meeting(Meeting::new("Two", 2.0))
            .await
            .unwrap();
        let mut file = centroids();
        file.linking_auto = automatic;
        let (a, b) = tokio::join!(
            first.link_speaker_centroids(&one.id, &file),
            second.link_speaker_centroids(&two.id, &file),
        );
        assert!(a.unwrap());
        assert!(b.unwrap());
        let mut people = first.list_people().await.unwrap();
        assert_eq!(people.len(), if automatic { 1 } else { 2 });
        people.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        assert_eq!(people[0].display_name, "Speaker 1");
        if !automatic {
            assert_eq!(people[1].display_name, "Speaker 2");
        }
        let one = first.get_meeting(&one.id).await.unwrap().unwrap();
        let two = second.get_meeting(&two.id).await.unwrap().unwrap();
        assert_eq!(
            one.speakers.last().unwrap().person_id == two.speakers.last().unwrap().person_id,
            automatic
        );
    }
}
