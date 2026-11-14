//! A per-person opt-out (`PersonProfile.tracked == false`) is a privacy
//! guarantee. Name matching must not let a differently cased speaker label
//! mint a fresh, tracked profile that collects what the opt-out withholds.
use super::*;
use bruhs_models::{MeetingSummary, ObservationKind, PersonObservation, Speaker};

fn observation(meeting_id: &str) -> PersonObservation {
    PersonObservation {
        at_meeting_id: meeting_id.into(),
        kind: ObservationKind::Tone,
        text: "Synthetic observation that must not be collected".into(),
        created_at: 1,
    }
}

async fn opted_out(store: &dyn MeetingStore, name: &str) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = "opted-out".into();
    person.aliases = vec!["Ünal Alias".into()];
    let person = store.upsert_person(person).await.unwrap();
    store
        .patch_person_tracked(&person.id.parse().unwrap(), false)
        .await
        .unwrap()
}

/// Labels that differ from the opted-out profile only by non-ASCII case.
const LABELS: [&str; 2] = ["ZOË", "ÜNAL ALIAS"];

async fn append_contract(store: &dyn MeetingStore) {
    let person = opted_out(store, "Zoë").await;
    for label in LABELS {
        let saved = store
            .append_person_observations(HashMap::from([(
                label.to_string(),
                vec![observation("meeting")],
            )]))
            .await
            .unwrap();
        assert!(saved.is_empty(), "{label} minted {saved:?}");
    }
    assert_eq!(store.list_people().await.unwrap(), vec![person]);
}

async fn summary_contract(store: &dyn MeetingStore) {
    let person = opted_out(store, "Zoë").await;
    let mut meeting = Meeting::new("Opt-out", 0.0);
    meeting.id = "opt-out-meeting".into();
    meeting.speakers = LABELS
        .iter()
        .enumerate()
        .map(|(index, label)| Speaker {
            id: format!("speaker-{index}"),
            label: (*label).into(),
            person_id: None,
            color: None,
        })
        .collect();
    let meeting = store.create_meeting(meeting).await.unwrap();
    let transcript = vec![TranscriptSegment {
        id: "opt-out-segment".into(),
        meeting_id: meeting.id.clone(),
        speaker_id: Some("speaker-0".into()),
        start_ms: 0,
        end_ms: 1000,
        text: "Synthetic speech".into(),
        confidence: 1.0,
        partial: false,
        speaker_confidence: None,
    }];
    store.append_segment(transcript[0].clone()).await.unwrap();
    let people_versions = store.summary_people_versions(&meeting).await.unwrap();
    assert_eq!(
        people_versions.get(&person.id),
        Some(&person.updated_at),
        "the opted-out contact is a participant under a cased label"
    );
    store
        .commit_summary(SummaryUpdate {
            expected: meeting.clone(),
            people_versions,
            transcript,
            summary: MeetingSummary {
                tldr: "Synthetic summary".into(),
                action_items: vec![],
                decisions: vec![],
                topics: vec![],
                generated_by: "fixture".into(),
                generated_at: 2,
            },
            observations: LABELS
                .iter()
                .map(|label| ((*label).to_string(), vec![observation(&meeting.id)]))
                .collect(),
            replace_observations: false,
        })
        .await
        .unwrap();
    assert_eq!(store.list_people().await.unwrap(), vec![person]);
}

#[tokio::test]
async fn memory_opt_out_survives_unicode_case_in_appended_observations() {
    append_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_opt_out_survives_unicode_case_in_appended_observations() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("opt-out.db"))
        .await
        .unwrap();
    append_contract(&store).await;
}

#[tokio::test]
async fn memory_opt_out_survives_unicode_case_in_summary_observations() {
    summary_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_opt_out_survives_unicode_case_in_summary_observations() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("opt-out.db"))
        .await
        .unwrap();
    summary_contract(&store).await;
}

/// Databases written before Unicode name keys stored ASCII-lowercased names.
/// Reopening must still match those contacts, or the opt-out silently lapses.
#[tokio::test]
async fn sqlite_reopen_rekeys_legacy_ascii_name_index() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.db");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let person = opted_out(&store, "ZOË").await;
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute("DELETE FROM person_names WHERE person_id=?1", [&person.id])
        .unwrap();
    for legacy in ["zoË", "Ünal alias"] {
        connection
            .execute(
                "INSERT INTO person_names(name,person_id) VALUES(?1,?2)",
                [legacy, person.id.as_str()],
            )
            .unwrap();
    }
    drop(connection);
    let reopened = SqliteMeetingStore::open(&path).await.unwrap();
    for label in ["zoë", "ünal alias"] {
        let saved = reopened
            .append_person_observations(HashMap::from([(
                label.to_string(),
                vec![observation("meeting")],
            )]))
            .await
            .unwrap();
        assert!(saved.is_empty(), "{label} minted {saved:?}");
    }
    assert_eq!(reopened.list_people().await.unwrap(), vec![person]);
}

/// A recognized on-screen name equal to an opted-out contact's alias (for
/// example a name merged into it) must link to that contact. Minting a new
/// tracked profile would route the next summary's observations around the
/// opt-out.
async fn named_speaker_contract(store: &dyn MeetingStore) {
    let person = opted_out(store, "Zoë").await;
    let meeting = store
        .create_meeting(Meeting::new("Named speaker", 0.0))
        .await
        .unwrap();
    let file = bruhs_models::CentroidsFile {
        linking_auto: false,
        centroids: ["name:ünal alias", "name:ZOË"]
            .iter()
            .map(|label| bruhs_models::ClusterCentroid {
                cluster_label: (*label).into(),
                embedding: vec![],
                first_seen_ms: 0,
            })
            .collect(),
        ..Default::default()
    };
    assert!(store
        .link_speaker_centroids(&meeting.id, &file)
        .await
        .unwrap());
    let linked = store.get_meeting(&meeting.id).await.unwrap().unwrap();
    let named: Vec<_> = linked
        .speakers
        .iter()
        .filter(|speaker| speaker.id.starts_with("name:"))
        .collect();
    assert_eq!(named.len(), 2, "{:?}", linked.speakers);
    for speaker in named {
        assert_eq!(speaker.person_id.as_deref(), Some(person.id.as_str()));
    }
    assert_eq!(store.list_people().await.unwrap(), vec![person]);
}

#[tokio::test]
async fn memory_named_speaker_links_opted_out_alias_without_new_profile() {
    named_speaker_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_named_speaker_links_opted_out_alias_without_new_profile() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("opt-out.db"))
        .await
        .unwrap();
    named_speaker_contract(&store).await;
}
