//! Delete, restore, unmerge, alias and meeting-delete rules for contacts
//! (PEOPLE-04, PEOPLE-08, PEOPLE-09, PEOPLE-10, PEOPLE-14). Every contract
//! runs against both stores.
use super::*;
use bruhs_models::{
    ActionItem, ActionItemRef, MeetingSpeakerRef, MeetingSummary, ObservationKind,
    PersonObservation, Speaker,
};

fn observation(meeting_id: &str, text: &str) -> PersonObservation {
    PersonObservation {
        at_meeting_id: meeting_id.into(),
        kind: ObservationKind::Topic,
        text: text.into(),
        created_at: 1,
    }
}

fn pid(id: &str) -> bruhs_models::ids::PersonId {
    id.parse().unwrap()
}

async fn contact(
    store: &dyn MeetingStore,
    id: &str,
    name: &str,
    aliases: &[&str],
    observations: Vec<PersonObservation>,
) -> PersonProfile {
    let mut person = PersonProfile::new(name);
    person.id = id.into();
    person.aliases = aliases.iter().map(|alias| (*alias).to_string()).collect();
    person.observations = observations;
    person.updated_at = 10;
    store.upsert_person(person).await.unwrap()
}

fn summary(items: &[(&str, Option<&str>)]) -> MeetingSummary {
    MeetingSummary {
        tldr: String::new(),
        action_items: items
            .iter()
            .map(|(text, assignee)| ActionItem {
                text: (*text).into(),
                assignee_person_id: assignee.map(str::to_string),
                due_hint: None,
            })
            .collect(),
        decisions: Vec::new(),
        topics: Vec::new(),
        generated_by: String::new(),
        generated_at: 0,
    }
}

async fn linked_meeting(
    store: &dyn MeetingStore,
    id: &str,
    speaker: (&str, &str),
    items: &[(&str, Option<&str>)],
) -> Meeting {
    let mut meeting = Meeting::new(id, 1.0);
    meeting.id = id.into();
    meeting.speakers = vec![Speaker {
        id: "them_1".into(),
        label: speaker.1.into(),
        person_id: Some(speaker.0.into()),
        color: None,
    }];
    meeting.summary = Some(summary(items));
    store.create_meeting(meeting).await.unwrap()
}

async fn sqlite_store() -> (tempfile::TempDir, SqliteMeetingStore) {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("people.sqlite3"))
        .await
        .unwrap();
    (directory, store)
}

/// Delete removes the contact with its observations and aliases, unlinks
/// (never deletes) its meeting links, and keeps retired merge ids
/// resolvable. Restore puts every still unlinked link back.
async fn delete_and_restore_contract(store: &dyn MeetingStore) {
    let ada = contact(
        store,
        "ada",
        "Ada",
        &["Countess"],
        vec![observation("m1", "Engines")],
    )
    .await;
    contact(store, "zoe", "Zoë", &[], Vec::new()).await;
    contact(store, "old", "Old Ada", &[], Vec::new()).await;
    store
        .merge_person_into(&pid("old"), &pid("ada"))
        .await
        .unwrap();
    let ada = store.get_person("ada").await.unwrap().unwrap_or(ada);
    linked_meeting(
        store,
        "m1",
        ("ada", "Ada"),
        &[("Write notes", Some("ada")), ("Review", Some("zoe"))],
    )
    .await;

    let deletion = store.delete_person(&pid("ada")).await.unwrap();
    assert_eq!(deletion.person, ada);
    assert_eq!(
        deletion.speakers,
        vec![MeetingSpeakerRef {
            meeting_id: "m1".into(),
            speaker_id: "them_1".into()
        }]
    );
    assert_eq!(
        deletion.action_items,
        vec![ActionItemRef {
            meeting_id: "m1".into(),
            index: 0,
            text: "Write notes".into()
        }]
    );
    assert!(deletion.deleted_at > 0);
    assert!(store.get_person("ada").await.unwrap().is_none());
    assert_eq!(
        store
            .list_people()
            .await
            .unwrap()
            .into_iter()
            .map(|person| person.id)
            .collect::<Vec<_>>(),
        ["zoe"]
    );
    let meeting = store.get_meeting("m1").await.unwrap().unwrap();
    assert_eq!(meeting.speakers[0].person_id, None);
    assert_eq!(meeting.speakers[0].label, "Ada", "the label stays");
    let items = &meeting.summary.as_ref().unwrap().action_items;
    assert_eq!(items[0].assignee_person_id, None);
    assert_eq!(items[0].text, "Write notes", "the item stays");
    assert_eq!(items[1].assignee_person_id.as_deref(), Some("zoe"));
    // A retired id that pointed at the deleted contact is unknown, not an error.
    assert!(store
        .resolve_person_merges(&["old".to_string()])
        .await
        .unwrap()
        .is_empty());

    let restored = store.restore_person(&pid("ada")).await.unwrap();
    assert_eq!(restored.id, "ada");
    assert_eq!(restored.display_name, ada.display_name);
    assert_eq!(restored.aliases, ada.aliases);
    assert_eq!(restored.observations, ada.observations);
    assert!(restored.updated_at > ada.updated_at, "a new version");
    assert_eq!(
        store.get_person("ada").await.unwrap(),
        Some(restored.clone())
    );
    let meeting = store.get_meeting("m1").await.unwrap().unwrap();
    assert_eq!(meeting.speakers[0].person_id.as_deref(), Some("ada"));
    let items = &meeting.summary.as_ref().unwrap().action_items;
    assert_eq!(items[0].assignee_person_id.as_deref(), Some("ada"));
    assert_eq!(items[1].assignee_person_id.as_deref(), Some("zoe"));
    let redirects = store
        .resolve_person_merges(&["old".to_string()])
        .await
        .unwrap();
    assert_eq!(redirects.len(), 1);
    assert_eq!(redirects[0].target, restored);

    assert!(matches!(
        store.restore_person(&pid("ada")).await,
        Err(MeetingStoreError::PersonNotFound { .. })
    ));
    assert!(matches!(
        store.delete_person(&pid("nobody")).await,
        Err(MeetingStoreError::PersonNotFound { .. })
    ));
}

#[tokio::test]
async fn memory_delete_cascades_and_restore_relinks() {
    delete_and_restore_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_delete_cascades_and_restore_relinks() {
    let (_directory, store) = sqlite_store().await;
    delete_and_restore_contract(&store).await;
}

/// Restore never takes back a link the user moved since, and refuses when
/// the id is live again.
async fn restore_refusal_contract(store: &dyn MeetingStore) {
    contact(store, "ada", "Ada", &[], Vec::new()).await;
    contact(store, "zoe", "Zoë", &[], Vec::new()).await;
    linked_meeting(store, "m1", ("ada", "Ada"), &[("Write notes", Some("ada"))]).await;
    store.delete_person(&pid("ada")).await.unwrap();
    store
        .link_speaker_to_person(
            &"m1".parse().unwrap(),
            &"them_1".parse().unwrap(),
            &pid("zoe"),
        )
        .await
        .unwrap();
    store.restore_person(&pid("ada")).await.unwrap();
    let meeting = store.get_meeting("m1").await.unwrap().unwrap();
    assert_eq!(meeting.speakers[0].person_id.as_deref(), Some("zoe"));
    assert_eq!(
        meeting.summary.unwrap().action_items[0]
            .assignee_person_id
            .as_deref(),
        Some("ada")
    );

    store.delete_person(&pid("ada")).await.unwrap();
    let mut again = PersonProfile::new("New Ada");
    again.id = "ada".into();
    store.insert_person_if_absent(again.clone()).await.unwrap();
    assert!(matches!(
        store.restore_person(&pid("ada")).await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert_eq!(store.get_person("ada").await.unwrap(), Some(again));
}

#[tokio::test]
async fn memory_restore_refuses_live_ids_and_keeps_moved_links() {
    restore_refusal_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_restore_refuses_live_ids_and_keeps_moved_links() {
    let (_directory, store) = sqlite_store().await;
    restore_refusal_contract(&store).await;
}

/// Unmerge gives back both contacts as they were, keeps later edits to the
/// survivor, and returns the meeting links the merge moved.
async fn unmerge_contract(store: &dyn MeetingStore) {
    let mut source = PersonProfile::new("Sam");
    source.id = "source".into();
    source.aliases = vec!["Sammy".into()];
    source.observations = vec![observation("m1", "Source note")];
    source.voice_embedding = Some(vec![0.6, 0.8]);
    source.updated_at = 10;
    let source = store.upsert_person(source).await.unwrap();
    let source = store
        .patch_person_tracked(&pid("source"), false)
        .await
        .unwrap();
    let target = contact(
        store,
        "target",
        "Taylor",
        &[],
        vec![observation("m1", "Target note")],
    )
    .await;
    linked_meeting(
        store,
        "m1",
        ("source", "Sam"),
        &[("Write notes", Some("source"))],
    )
    .await;

    let merged = store
        .merge_person_into(&pid("source"), &pid("target"))
        .await
        .unwrap();
    assert!(!merged.tracked, "the opt-out is sticky through a merge");
    let meeting = store.get_meeting("m1").await.unwrap().unwrap();
    assert_eq!(meeting.speakers[0].person_id.as_deref(), Some("target"));
    assert_eq!(meeting.speakers[0].label, "Taylor");
    // An edit after the merge must survive the undo.
    store
        .rename_person(&pid("target"), "Taylor Swift".into())
        .await
        .unwrap();

    let unmerge = store.unmerge_person(&pid("source")).await.unwrap();
    assert_eq!(unmerge.source.display_name, "Sam");
    assert_eq!(unmerge.source.aliases, source.aliases);
    assert_eq!(unmerge.source.observations, source.observations);
    assert_eq!(unmerge.source.voice_embedding, source.voice_embedding);
    assert!(!unmerge.source.tracked, "the source keeps its own opt-out");
    assert!(unmerge.source.updated_at > source.updated_at);
    assert_eq!(unmerge.target.display_name, "Taylor Swift");
    assert!(unmerge.target.aliases.is_empty());
    assert_eq!(unmerge.target.observations, target.observations);
    assert_eq!(unmerge.target.voice_embedding, None);
    assert!(unmerge.target.tracked, "the target gets its own flag back");
    assert_eq!(
        store.get_person("source").await.unwrap(),
        Some(unmerge.source.clone())
    );
    assert_eq!(
        store.get_person("target").await.unwrap(),
        Some(unmerge.target.clone())
    );
    let meeting = store.get_meeting("m1").await.unwrap().unwrap();
    assert_eq!(meeting.speakers[0].person_id.as_deref(), Some("source"));
    assert_eq!(meeting.speakers[0].label, "Sam");
    assert_eq!(
        meeting.summary.unwrap().action_items[0]
            .assignee_person_id
            .as_deref(),
        Some("source")
    );
    assert!(store
        .resolve_person_merges(&["source".to_string()])
        .await
        .unwrap()
        .is_empty());
    assert!(matches!(
        store.unmerge_person(&pid("source")).await,
        Err(MeetingStoreError::PersonNotFound { .. })
    ));
}

#[tokio::test]
async fn memory_unmerge_restores_both_contacts_and_moved_links() {
    unmerge_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_unmerge_restores_both_contacts_and_moved_links() {
    let (_directory, store) = sqlite_store().await;
    unmerge_contract(&store).await;
}

/// A tracking change made after the merge is the user's newer choice.
async fn unmerge_keeps_later_tracking_contract(store: &dyn MeetingStore) {
    contact(store, "source", "Sam", &[], Vec::new()).await;
    store
        .patch_person_tracked(&pid("source"), false)
        .await
        .unwrap();
    contact(store, "target", "Taylor", &[], Vec::new()).await;
    store
        .merge_person_into(&pid("source"), &pid("target"))
        .await
        .unwrap();
    store
        .patch_person_tracked(&pid("target"), true)
        .await
        .unwrap();
    let unmerge = store.unmerge_person(&pid("source")).await.unwrap();
    assert!(unmerge.target.tracked);
    assert!(!unmerge.source.tracked);
}

#[tokio::test]
async fn memory_unmerge_keeps_a_later_tracking_choice() {
    unmerge_keeps_later_tracking_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_unmerge_keeps_a_later_tracking_choice() {
    let (_directory, store) = sqlite_store().await;
    unmerge_keeps_later_tracking_contract(&store).await;
}

/// Undo is refused, with nothing changed, once the survivor was merged on
/// or deleted, or another contact was merged into the source.
async fn unmerge_refusal_contract(store: &dyn MeetingStore) {
    for id in ["a", "b", "c"] {
        contact(store, id, &id.to_uppercase(), &[], Vec::new()).await;
    }
    store.merge_person_into(&pid("a"), &pid("b")).await.unwrap();
    store.merge_person_into(&pid("b"), &pid("c")).await.unwrap();
    let before = store.list_people().await.unwrap();
    assert!(matches!(
        store.unmerge_person(&pid("a")).await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert!(matches!(
        store.unmerge_person(&pid("b")).await,
        Err(MeetingStoreError::Conflict(_))
    ));
    assert_eq!(store.list_people().await.unwrap(), before);

    store.delete_person(&pid("c")).await.unwrap();
    assert!(store.unmerge_person(&pid("b")).await.is_err());
    assert!(store.get_person("b").await.unwrap().is_none());
    assert!(store.list_people().await.unwrap().is_empty());
}

#[tokio::test]
async fn memory_unmerge_refuses_after_the_survivor_changed_identity() {
    unmerge_refusal_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_unmerge_refuses_after_the_survivor_changed_identity() {
    let (_directory, store) = sqlite_store().await;
    unmerge_refusal_contract(&store).await;
}

/// Aliases normalize, match observations case-insensitively, and refuse a
/// name another contact answers to without changing anything.
async fn alias_contract(store: &dyn MeetingStore) {
    contact(store, "ada", "Ada", &[], Vec::new()).await;
    contact(store, "zoe", "Zoë", &["Z"], Vec::new()).await;
    let saved = store
        .set_person_aliases(
            &pid("ada"),
            vec![
                " Countess ".into(),
                "countess".into(),
                "ADA".into(),
                String::new(),
            ],
        )
        .await
        .unwrap();
    assert_eq!(saved.aliases, ["Countess"]);

    let refused = store
        .set_person_aliases(&pid("ada"), vec!["Countess".into(), "ZOË".into()])
        .await
        .unwrap_err();
    match refused {
        MeetingStoreError::Conflict(message) => {
            assert!(message.contains("Zoë already uses that name"), "{message}");
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(store.get_person("ada").await.unwrap(), Some(saved.clone()));

    // The name index follows the new alias under any casing.
    let appended = store
        .append_person_observations(HashMap::from([(
            "COUNTESS".to_string(),
            vec![observation("m1", "Alias match")],
        )]))
        .await
        .unwrap();
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].id, "ada");

    let cleared = store
        .set_person_aliases(&pid("ada"), Vec::new())
        .await
        .unwrap();
    assert!(cleared.aliases.is_empty());
    assert!(matches!(
        store.set_person_aliases(&pid("nobody"), Vec::new()).await,
        Err(MeetingStoreError::PersonNotFound { .. })
    ));
}

#[tokio::test]
async fn memory_aliases_are_reviewed_for_uniqueness() {
    alias_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_aliases_are_reviewed_for_uniqueness() {
    let (_directory, store) = sqlite_store().await;
    alias_contract(&store).await;
}

/// PEOPLE-14: deleting a meeting deletes the observations derived from it.
async fn meeting_delete_contract(store: &dyn MeetingStore) {
    for id in ["m1", "m2"] {
        let mut meeting = Meeting::new(id, 1.0);
        meeting.id = id.into();
        store.create_meeting(meeting).await.unwrap();
    }
    let ada = contact(
        store,
        "ada",
        "Ada",
        &[],
        vec![observation("m1", "Gone"), observation("m2", "Kept")],
    )
    .await;
    assert!(store.delete_meeting(&"m1".parse().unwrap()).await.unwrap());
    let after = store.get_person("ada").await.unwrap().unwrap();
    assert_eq!(after.observations, vec![observation("m2", "Kept")]);
    assert!(after.updated_at > ada.updated_at);
    assert!(!store
        .delete_meeting(&"missing".parse().unwrap())
        .await
        .unwrap());
    assert_eq!(store.get_person("ada").await.unwrap(), Some(after));
}

#[tokio::test]
async fn memory_meeting_delete_removes_derived_observations() {
    meeting_delete_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_meeting_delete_removes_derived_observations() {
    let (_directory, store) = sqlite_store().await;
    meeting_delete_contract(&store).await;
}

/// A version 5 database (no deleted_people table, no merge link record)
/// upgrades in place. Its legacy merges still undo, restoring profiles only.
#[tokio::test]
async fn sqlite_version_five_upgrades_and_legacy_merges_undo_profiles_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    contact(&store, "source", "Sam", &[], Vec::new()).await;
    contact(&store, "target", "Taylor", &[], Vec::new()).await;
    linked_meeting(&store, "m1", ("source", "Sam"), &[]).await;
    store
        .merge_person_into(&pid("source"), &pid("target"))
        .await
        .unwrap();
    drop(store);
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "ALTER TABLE merged_people DROP COLUMN links; DROP TABLE deleted_people; PRAGMA user_version=5;",
            )
            .unwrap();
    }
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    let unmerge = store.unmerge_person(&pid("source")).await.unwrap();
    assert_eq!(unmerge.source.display_name, "Sam");
    assert_eq!(unmerge.target.display_name, "Taylor");
    // No link record: the speaker stays with the survivor.
    let meeting = store.get_meeting("m1").await.unwrap().unwrap();
    assert_eq!(meeting.speakers[0].person_id.as_deref(), Some("target"));
    // The new archive table works after the upgrade.
    store.delete_person(&pid("source")).await.unwrap();
    store.restore_person(&pid("source")).await.unwrap();
    drop(store);

    // A file already at the new layout but labeled 5 reopens too.
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("PRAGMA user_version=5;").unwrap();
    }
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    assert!(store.get_person("source").await.unwrap().is_some());
}
