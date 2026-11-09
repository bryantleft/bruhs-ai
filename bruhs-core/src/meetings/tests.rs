use super::error::MeetingStoreError;
use super::in_memory::InMemoryMeetingStore;
use crate::meetings::MeetingPatch;
use crate::meetings::MeetingRecords;
use crate::meetings::PeopleStore;
use bruhs_models::{Meeting, MeetingStatus, TranscriptSegment};

#[tokio::test]
async fn create_get_list_meetings_roundtrip() {
    let store = InMemoryMeetingStore::new();
    let m = Meeting::new("Sync", 1.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();

    let fetched = store.get_meeting(&id).await.unwrap().unwrap();
    assert_eq!(fetched.title, "Sync");

    let all = store.list_meetings().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, id);
}

#[tokio::test]
async fn patch_meeting_updates_fields() {
    let store = InMemoryMeetingStore::new();
    let m = Meeting::new("Sync", 1.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();

    let patched = store
        .patch_meeting(
            &id.parse().unwrap(),
            MeetingPatch {
                title: Some("Sync (renamed)".into()),
                status: Some(MeetingStatus::Ended),
                ended_at_ms: Some(10.0),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(patched.title, "Sync (renamed)");
    assert_eq!(patched.status, MeetingStatus::Ended);
    assert_eq!(patched.ended_at_ms, Some(10.0));
}

#[tokio::test]
async fn patch_unknown_meeting_returns_none() {
    let store = InMemoryMeetingStore::new();
    let res = store
        .patch_meeting(
            &"missing".parse().unwrap(),
            MeetingPatch {
                title: Some("x".into()),
                status: None,
                ended_at_ms: None,
            },
        )
        .await
        .unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn delete_meeting_also_drops_segments() {
    let store = InMemoryMeetingStore::new();
    let m = Meeting::new("Sync", 1.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();
    store
        .append_segment(TranscriptSegment {
            id: "s1".into(),
            meeting_id: id.clone(),
            speaker_id: None,
            start_ms: 0,
            end_ms: 1,
            text: "hi".into(),
            confidence: 1.0,
            partial: false,
            speaker_confidence: None,
        })
        .await
        .unwrap();

    let removed = store.delete_meeting(&id.parse().unwrap()).await.unwrap();
    assert!(removed);
    let segs = store.list_segments(&id).await.unwrap();
    assert!(segs.is_empty());
}

#[tokio::test]
async fn patch_meeting_summary_updates_field() {
    let store = InMemoryMeetingStore::new();
    let m = bruhs_models::Meeting::new("Sync", 1.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();
    let summary = bruhs_models::MeetingSummary {
        tldr: "x".into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "test".into(),
        generated_at: 0,
    };
    let patched = store
        .patch_meeting_summary(&id.parse().unwrap(), summary.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(patched.summary.as_ref().unwrap().tldr, "x");
}

#[tokio::test]
async fn patch_summary_unknown_returns_none() {
    let store = InMemoryMeetingStore::new();
    let summary = bruhs_models::MeetingSummary {
        tldr: "x".into(),
        action_items: vec![],
        decisions: vec![],
        topics: vec![],
        generated_by: "test".into(),
        generated_at: 0,
    };
    let res = store
        .patch_meeting_summary(&"missing".parse().unwrap(), summary)
        .await
        .unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn list_meetings_sorted_recent_first() {
    let store = InMemoryMeetingStore::new();
    let a = Meeting::new("Old", 1.0);
    let b = Meeting::new("New", 5.0);
    let a_id = a.id.clone();
    let b_id = b.id.clone();
    store.create_meeting(a).await.unwrap();
    store.create_meeting(b).await.unwrap();
    let all = store.list_meetings().await.unwrap();
    assert_eq!(all[0].id, b_id, "newer started_at_ms should be first");
    assert_eq!(all[1].id, a_id);
}

#[tokio::test]
async fn patch_speaker_label_updates_label() {
    let store = InMemoryMeetingStore::new();
    let m = Meeting::new("Title", 0.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();
    let updated = store
        .patch_speaker_label(
            &id.parse().unwrap(),
            &"me".parse().unwrap(),
            "Bryant".into(),
        )
        .await
        .unwrap();
    let me = updated.speakers.iter().find(|s| s.id == "me").unwrap();
    assert_eq!(me.label, "Bryant");
}

#[tokio::test]
async fn patch_speaker_label_unknown_speaker_errors() {
    let store = InMemoryMeetingStore::new();
    let m = Meeting::new("Title", 0.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();
    let res = store
        .patch_speaker_label(&id.parse().unwrap(), &"ghost".parse().unwrap(), "x".into())
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn patch_speaker_label_unknown_meeting_errors() {
    let store = InMemoryMeetingStore::new();
    let res = store
        .patch_speaker_label(
            &"missing".parse().unwrap(),
            &"me".parse().unwrap(),
            "x".into(),
        )
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn patch_meeting_thread_id_sets_field() {
    let store = InMemoryMeetingStore::new();
    let m = Meeting::new("Sync", 0.0);
    let id = m.id.clone();
    store.create_meeting(m).await.unwrap();
    let updated = store
        .patch_meeting_thread_id(&id.parse().unwrap(), &"t-abc".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(updated.thread_id.as_deref(), Some("t-abc"));
}

#[tokio::test]
async fn patch_meeting_thread_id_unknown_errors() {
    let store = InMemoryMeetingStore::new();
    let res = store
        .patch_meeting_thread_id(&"ghost".parse().unwrap(), &"t-1".parse().unwrap())
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn append_creates_profile_for_unknown_label() {
    let store = InMemoryMeetingStore::default();
    let mut by_label = std::collections::HashMap::new();
    by_label.insert(
        "Alice".to_string(),
        vec![bruhs_models::PersonObservation {
            at_meeting_id: "m-1".into(),
            kind: bruhs_models::ObservationKind::Topic,
            text: "Pricing strategy".into(),
            created_at: 100,
        }],
    );
    let updated = store.append_person_observations(by_label).await.unwrap();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].display_name, "Alice");
    assert_eq!(updated[0].observations.len(), 1);
    let people = store.list_people().await.unwrap();
    assert_eq!(people.len(), 1);
}

#[tokio::test]
async fn append_extends_existing_profile_by_display_name() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut by_label = std::collections::HashMap::new();
    by_label.insert(
        "alice".to_string(),
        vec![bruhs_models::PersonObservation {
            at_meeting_id: "m-1".into(),
            kind: bruhs_models::ObservationKind::Decision,
            text: "Ship Friday".into(),
            created_at: 100,
        }],
    );
    store.append_person_observations(by_label).await.unwrap();
    let people = store.list_people().await.unwrap();
    assert_eq!(people.len(), 1);
    let stored_alice = people.iter().find(|p| p.id == alice_id).unwrap();
    assert_eq!(stored_alice.observations.len(), 1);
}

#[tokio::test]
async fn append_skips_empty_observations() {
    let store = InMemoryMeetingStore::default();
    let mut by_label = std::collections::HashMap::new();
    by_label.insert("Empty".into(), vec![]);
    store.append_person_observations(by_label).await.unwrap();
    assert!(store.list_people().await.unwrap().is_empty());
}

#[tokio::test]
async fn match_centroid_returns_above_threshold() {
    let store = InMemoryMeetingStore::default();
    let mut alice = bruhs_models::PersonProfile::new("Alice");
    let alice_emb = vec![1.0, 0.0, 0.0, 0.0];
    alice.voice_embedding = Some(alice_emb.clone());
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let result = store
        .match_centroid_to_person(&alice_emb, 0.5)
        .await
        .unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().id, alice_id);
}

#[tokio::test]
async fn match_centroid_returns_none_below_threshold() {
    let store = InMemoryMeetingStore::default();
    let mut alice = bruhs_models::PersonProfile::new("Alice");
    alice.voice_embedding = Some(vec![1.0, 0.0, 0.0, 0.0]);
    store.upsert_person(alice).await.unwrap();

    let result = store
        .match_centroid_to_person(&[0.0, 1.0, 0.0, 0.0], 0.5)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn match_centroid_ignores_profiles_without_embedding() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice"); // no voice_embedding
    store.upsert_person(alice).await.unwrap();

    let result = store
        .match_centroid_to_person(&[1.0, 0.0], 0.5)
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn upsert_voice_embedding_sets_field() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let updated = store
        .upsert_voice_embedding(&alice_id.parse().unwrap(), vec![1.0, 2.0, 3.0])
        .await
        .unwrap();
    assert_eq!(updated.voice_embedding.as_ref().unwrap().len(), 3);
}

// ── Phase 6c mutator tests ────────────────────────────────────────────────

#[tokio::test]
async fn patch_person_tracked_sets_false() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let updated = store
        .patch_person_tracked(&alice_id.parse().unwrap(), false)
        .await
        .unwrap();
    assert!(!updated.tracked);

    let people = store.list_people().await.unwrap();
    let alice_stored = people.iter().find(|p| p.id == alice_id).unwrap();
    assert!(!alice_stored.tracked);
}

#[tokio::test]
async fn patch_person_tracked_unknown_errors() {
    let store = InMemoryMeetingStore::default();
    let res = store
        .patch_person_tracked(&"ghost".parse().unwrap(), true)
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn patch_meeting_skip_observations_sets_true() {
    let store = InMemoryMeetingStore::default();
    let m = Meeting::new("Sync", 0.0);
    let mid = m.id.clone();
    store.upsert_meeting(m).await.unwrap();

    let updated = store
        .patch_meeting_skip_observations(&mid.parse().unwrap(), true)
        .await
        .unwrap();
    assert!(updated.skip_observations);

    let fetched = store.get_meeting(&mid).await.unwrap().unwrap();
    assert!(fetched.skip_observations);
}

#[tokio::test]
async fn patch_meeting_skip_observations_unknown_errors() {
    let store = InMemoryMeetingStore::default();
    let res = store
        .patch_meeting_skip_observations(&"ghost".parse().unwrap(), true)
        .await;
    assert!(res.is_err());
}

#[tokio::test]
async fn delete_observations_for_meeting_removes_only_target() {
    let store = InMemoryMeetingStore::default();

    let mut alice = bruhs_models::PersonProfile::new("Alice");
    alice.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: "m-1".into(),
        kind: bruhs_models::ObservationKind::Topic,
        text: "Topic from m-1".into(),
        created_at: 1,
    });
    alice.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: "m-2".into(),
        kind: bruhs_models::ObservationKind::Decision,
        text: "Decision from m-2".into(),
        created_at: 2,
    });
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut bob = bruhs_models::PersonProfile::new("Bob");
    bob.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: "m-1".into(),
        kind: bruhs_models::ObservationKind::Tone,
        text: "Bob was talkative".into(),
        created_at: 3,
    });
    let bob_id = bob.id.clone();
    store.upsert_person(bob).await.unwrap();

    let count = store
        .delete_observations_for_meeting(&"m-1".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(count, 2, "should delete 1 from Alice + 1 from Bob");

    let people = store.list_people().await.unwrap();
    let alice_stored = people.iter().find(|p| p.id == alice_id).unwrap();
    assert_eq!(alice_stored.observations.len(), 1);
    assert_eq!(alice_stored.observations[0].at_meeting_id, "m-2");

    let bob_stored = people.iter().find(|p| p.id == bob_id).unwrap();
    assert!(bob_stored.observations.is_empty());
}

#[tokio::test]
async fn delete_observations_for_meeting_no_match_returns_zero() {
    let store = InMemoryMeetingStore::default();
    let count = store
        .delete_observations_for_meeting(&"nonexistent".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn append_skips_untracked_existing_profile() {
    let store = InMemoryMeetingStore::default();

    let mut alice = bruhs_models::PersonProfile::new("Alice");
    alice.tracked = false;
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut by_label = std::collections::HashMap::new();
    by_label.insert(
        "Alice".to_string(),
        vec![bruhs_models::PersonObservation {
            at_meeting_id: "m-1".into(),
            kind: bruhs_models::ObservationKind::Topic,
            text: "something".into(),
            created_at: 100,
        }],
    );
    let updated = store.append_person_observations(by_label).await.unwrap();
    assert!(updated.is_empty());

    let people = store.list_people().await.unwrap();
    let alice_stored = people.iter().find(|p| p.id == alice_id).unwrap();
    assert!(alice_stored.observations.is_empty());
    assert_eq!(people.len(), 1);
}

// ── Phase 6d mutator tests ────────────────────────────────────────────────

#[tokio::test]
async fn rename_person_updates_display_name() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let updated = store
        .rename_person(&alice_id.parse().unwrap(), "Alice B.".into())
        .await
        .unwrap();
    assert_eq!(updated.display_name, "Alice B.");

    let fetched = store.list_people().await.unwrap();
    assert_eq!(fetched[0].display_name, "Alice B.");
}

#[tokio::test]
async fn rename_person_trims_whitespace() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let updated = store
        .rename_person(&alice_id.parse().unwrap(), "  Bob  ".into())
        .await
        .unwrap();
    assert_eq!(updated.display_name, "Bob");
}

#[tokio::test]
async fn rename_person_rejects_empty_name() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let res = store
        .rename_person(&alice_id.parse().unwrap(), "   ".into())
        .await;
    assert!(
        matches!(res, Err(MeetingStoreError::InvalidInput(_))),
        "expected InvalidInput, got: {res:?}"
    );
}

#[tokio::test]
async fn rename_person_not_found_errors() {
    let store = InMemoryMeetingStore::default();
    let res = store
        .rename_person(&"ghost".parse().unwrap(), "X".into())
        .await;
    assert!(
        matches!(
            res,
            Err(MeetingStoreError::MeetingNotFound { .. }
                | MeetingStoreError::PersonNotFound { .. }
                | MeetingStoreError::SpeakerNotFound { .. })
        ),
        "expected NotFound, got: {res:?}"
    );
}

#[tokio::test]
async fn merge_person_moves_observations() {
    let store = InMemoryMeetingStore::default();
    let mut alice = bruhs_models::PersonProfile::new("Alice");
    alice.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: "m-1".into(),
        kind: bruhs_models::ObservationKind::Topic,
        text: "pricing".into(),
        created_at: 1,
    });
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut bob = bruhs_models::PersonProfile::new("Bob");
    bob.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: "m-2".into(),
        kind: bruhs_models::ObservationKind::Decision,
        text: "ship it".into(),
        created_at: 2,
    });
    let bob_id = bob.id.clone();
    store.upsert_person(bob).await.unwrap();

    let updated_bob = store
        .merge_person_into(&alice_id.parse().unwrap(), &bob_id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(updated_bob.observations.len(), 2);

    let people = store.list_people().await.unwrap();
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].id, bob_id);
}

#[tokio::test]
async fn merge_person_keeps_target_voice_embedding_when_set() {
    let store = InMemoryMeetingStore::default();
    let mut alice = bruhs_models::PersonProfile::new("Alice");
    alice.voice_embedding = Some(vec![0.1, 0.2]);
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut bob = bruhs_models::PersonProfile::new("Bob");
    let bob_emb = vec![0.9, 0.8];
    bob.voice_embedding = Some(bob_emb.clone());
    let bob_id = bob.id.clone();
    store.upsert_person(bob).await.unwrap();

    let updated = store
        .merge_person_into(&alice_id.parse().unwrap(), &bob_id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(updated.voice_embedding.as_ref().unwrap(), &bob_emb);
}

#[tokio::test]
async fn merge_person_copies_source_voice_embedding_when_target_empty() {
    let store = InMemoryMeetingStore::default();
    let mut alice = bruhs_models::PersonProfile::new("Alice");
    let alice_emb = vec![0.1, 0.2];
    alice.voice_embedding = Some(alice_emb.clone());
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let bob = bruhs_models::PersonProfile::new("Bob");
    let bob_id = bob.id.clone();
    store.upsert_person(bob).await.unwrap();

    let updated = store
        .merge_person_into(&alice_id.parse().unwrap(), &bob_id.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(updated.voice_embedding.as_ref().unwrap(), &alice_emb);
}

#[tokio::test]
async fn merge_person_adds_source_name_as_alias() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let bob = bruhs_models::PersonProfile::new("Bob");
    let bob_id = bob.id.clone();
    store.upsert_person(bob).await.unwrap();

    let updated = store
        .merge_person_into(&alice_id.parse().unwrap(), &bob_id.parse().unwrap())
        .await
        .unwrap();
    assert!(
        updated
            .aliases
            .iter()
            .any(|a| a.eq_ignore_ascii_case("Alice")),
        "expected 'Alice' alias on Bob, got: {:?}",
        updated.aliases
    );
}

#[tokio::test]
async fn merge_person_dedupes_alias_case_insensitive() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut bob = bruhs_models::PersonProfile::new("Bob");
    bob.aliases.push("alice".into());
    let bob_id = bob.id.clone();
    store.upsert_person(bob).await.unwrap();

    let updated = store
        .merge_person_into(&alice_id.parse().unwrap(), &bob_id.parse().unwrap())
        .await
        .unwrap();
    let alice_alias_count = updated
        .aliases
        .iter()
        .filter(|a| a.eq_ignore_ascii_case("alice"))
        .count();
    assert_eq!(
        alice_alias_count, 1,
        "duplicate alias added: {:?}",
        updated.aliases
    );
}

#[tokio::test]
async fn merge_person_into_self_errors() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let res = store
        .merge_person_into(&alice_id.parse().unwrap(), &alice_id.parse().unwrap())
        .await;
    assert!(
        matches!(res, Err(MeetingStoreError::InvalidInput(_))),
        "expected InvalidInput, got: {res:?}"
    );
}

#[tokio::test]
async fn ensure_meeting_speakers_preserves_existing() {
    let store = InMemoryMeetingStore::default();
    let mut m = Meeting::new("Sync", 0.0);
    if let Some(me) = m.speakers.iter_mut().find(|s| s.id == "me") {
        me.label = "Bryant".into();
    }
    let id = m.id.clone();
    store.upsert_meeting(m).await.unwrap();

    let new_speakers = vec![
        bruhs_models::Speaker {
            id: "me".into(),
            label: "WRONG".into(),
            person_id: None,
            color: None,
        },
        bruhs_models::Speaker {
            id: "them_1".into(),
            label: "Alice".into(),
            person_id: None,
            color: None,
        },
    ];
    let updated = store
        .ensure_meeting_speakers(&id.parse().unwrap(), new_speakers)
        .await
        .unwrap();
    let me = updated.speakers.iter().find(|s| s.id == "me").unwrap();
    assert_eq!(me.label, "Bryant");
    let them_1 = updated.speakers.iter().find(|s| s.id == "them_1").unwrap();
    assert_eq!(them_1.label, "Alice");
}

// ── Phase 3d: link_speaker_to_person ─────────────────────────────────────

#[tokio::test]
async fn link_speaker_updates_existing_entry() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let mut m = Meeting::new("Sync", 0.0);
    m.speakers.push(bruhs_models::Speaker {
        id: "them_1".into(),
        label: "Speaker 1".into(),
        person_id: None,
        color: None,
    });
    let mid = m.id.clone();
    store.upsert_meeting(m).await.unwrap();

    let updated = store
        .link_speaker_to_person(
            &mid.parse().unwrap(),
            &"them_1".parse().unwrap(),
            &alice_id.parse().unwrap(),
        )
        .await
        .unwrap();
    let sp = updated.speakers.iter().find(|s| s.id == "them_1").unwrap();
    assert_eq!(sp.label, "Alice");
    assert_eq!(sp.person_id.as_deref(), Some(alice_id.as_str()));
}

#[tokio::test]
async fn link_speaker_creates_entry_when_absent() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let m = Meeting::new("Sync", 0.0);
    let mid = m.id.clone();
    store.upsert_meeting(m).await.unwrap();

    let updated = store
        .link_speaker_to_person(
            &mid.parse().unwrap(),
            &"them_1".parse().unwrap(),
            &alice_id.parse().unwrap(),
        )
        .await
        .unwrap();
    let sp = updated.speakers.iter().find(|s| s.id == "them_1").unwrap();
    assert_eq!(sp.label, "Alice");
    assert_eq!(sp.person_id.as_deref(), Some(alice_id.as_str()));
}

#[tokio::test]
async fn link_speaker_missing_meeting_errors() {
    let store = InMemoryMeetingStore::default();
    let alice = bruhs_models::PersonProfile::new("Alice");
    let alice_id = alice.id.clone();
    store.upsert_person(alice).await.unwrap();

    let res = store
        .link_speaker_to_person(
            &"ghost-meeting".parse().unwrap(),
            &"them_1".parse().unwrap(),
            &alice_id.parse().unwrap(),
        )
        .await;
    assert!(
        matches!(
            res,
            Err(MeetingStoreError::MeetingNotFound { .. }
                | MeetingStoreError::PersonNotFound { .. }
                | MeetingStoreError::SpeakerNotFound { .. })
        ),
        "expected NotFound, got: {res:?}"
    );
}

#[tokio::test]
async fn link_speaker_missing_person_errors() {
    let store = InMemoryMeetingStore::default();
    let m = Meeting::new("Sync", 0.0);
    let mid = m.id.clone();
    store.upsert_meeting(m).await.unwrap();

    let res = store
        .link_speaker_to_person(
            &mid.parse().unwrap(),
            &"them_1".parse().unwrap(),
            &"ghost-person".parse().unwrap(),
        )
        .await;
    assert!(
        matches!(
            res,
            Err(MeetingStoreError::MeetingNotFound { .. }
                | MeetingStoreError::PersonNotFound { .. }
                | MeetingStoreError::SpeakerNotFound { .. })
        ),
        "expected NotFound, got: {res:?}"
    );
}

/// `insert_person_if_absent` on one backend: creates once, then returns the
/// saved contact unchanged; concurrent creates of one id yield exactly one
/// `Created`; a merged-away id is refused.
async fn check_insert_person_if_absent(store: std::sync::Arc<dyn super::MeetingStore>) {
    use super::PersonInsert;
    use bruhs_models::PersonProfile;

    let original = PersonProfile::new("Original");
    let created = store
        .insert_person_if_absent(original.clone())
        .await
        .unwrap();
    assert_eq!(created, PersonInsert::Created(original.clone()));

    let mut different = original.clone();
    different.display_name = "Someone else".into();
    let existing = store.insert_person_if_absent(different).await.unwrap();
    assert_eq!(existing, PersonInsert::Existing(original.clone()));
    assert_eq!(
        store.get_person(&original.id).await.unwrap(),
        Some(original.clone())
    );

    let racer = PersonProfile::new("Racer");
    let attempts: Vec<_> = (0..8)
        .map(|i| {
            let store = store.clone();
            let mut person = racer.clone();
            person.display_name = format!("Racer {i}");
            tokio::spawn(async move { store.insert_person_if_absent(person).await })
        })
        .collect();
    let mut created = 0;
    for attempt in attempts {
        if matches!(attempt.await.unwrap().unwrap(), PersonInsert::Created(_)) {
            created += 1;
        }
    }
    assert_eq!(created, 1);

    let target = PersonProfile::new("Target");
    store.upsert_person(target.clone()).await.unwrap();
    store
        .merge_person_into(&original.id.parse().unwrap(), &target.id.parse().unwrap())
        .await
        .unwrap();
    assert!(store.insert_person_if_absent(original).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn insert_person_if_absent_is_atomic_on_both_backends() {
    check_insert_person_if_absent(std::sync::Arc::new(InMemoryMeetingStore::new())).await;
    let dir = tempfile::tempdir().unwrap();
    let sqlite = super::SqliteMeetingStore::open(dir.path().join("people.sqlite3"))
        .await
        .unwrap();
    check_insert_person_if_absent(std::sync::Arc::new(sqlite)).await;
}
