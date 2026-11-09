use super::super::{
    desktop_state_backup_path, load_desktop_state_from_path, SaveStatus, Workspace,
};
use super::tests::fixture;
use crate::{
    server_client::ServerClient,
    views::meetings_hub::{HubEvent, PeopleSnapshot},
};
use bruhs_models::*;
use gpui::TestAppContext;
mod bulk;
mod bulk_cases;
mod http;
mod incoming;
mod orphan;
mod retired;

fn install(w: &mut Workspace, cx: &mut gpui::Context<'_, Workspace>) -> PersonMergeResolution {
    let mut source = PersonProfile::new("Unsynced source name");
    source.id = "source".into();
    let mut target = PersonProfile::new("Surviving name");
    target.id = "target".into();
    target.updated_at = 10;
    w.meetings_hub.update(cx, |hub, _| {
        hub.set_people_test_client(
            ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").expect("test client"),
        );
        hub.restore_people(PeopleSnapshot {
            records: vec![source, target.clone()],
            pending_creates: ["source".into()].into_iter().collect(),
            ..Default::default()
        });
    });
    let mut source_relationship = RelationshipProfile::new("source", 1);
    source_relationship.notes = "Private source notes".into();
    source_relationship.company = "Source company".into();
    source_relationship.add_item(
        RelationshipItem::new(RelationshipItemKind::Reminder, "Keep reminder", 1),
        1,
    );
    let mut target_relationship = RelationshipProfile::new("target", 1);
    target_relationship.notes = "Private target notes".into();
    target_relationship.company = "Target company".into();
    w.relationships = vec![source_relationship, target_relationship];
    w.inbox_conversations = vec![InboxConversation::local_draft(
        "Original participant",
        "Unsent body",
        1,
    )];
    w.inbox_conversations[0].participants[0].person_id = Some("source".into());
    w.captures = vec![
        CaptureRecord::new(CaptureKind::Contact, "Exact capture text", 1)
            .expect("contact capture")
            .with_target("source"),
    ];
    let mut document = KnowledgeDocument::new("Linked document", "Exact body", 1);
    document.links.push(KnowledgeLink {
        kind: KnowledgeLinkKind::Person,
        target_id: "source".into(),
        label: "Original link label".into(),
    });
    w.knowledge_documents = vec![document];
    PersonMergeResolution {
        source_id: "source".into(),
        target,
    }
}

#[gpui::test]
async fn canonical_contact_and_all_private_links_save_with_exact_originals(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory = std::env::temp_dir().join(format!(
        "bruhs-native-contact-merge-{}",
        uuid::Uuid::new_v4()
    ));
    let path = directory.join("state.json");
    let (redirect, before) = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        w.persistence_test_path = Some(path.clone());
        let before = w.snapshot(cx);
        // Exercise the real hub-to-workspace subscription, not only the helper.
        w.meetings_hub.update(cx, |_, cx| {
            cx.emit(HubEvent::PersonMergeResolved(Box::new(redirect.clone())))
        });
        (redirect, before)
    });
    cx.condition(&workspace, |w, cx| {
        w.save_status.is_saved() && w.meetings_hub.read(cx).person_is_retired("source")
    })
    .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.people_cache.records, vec![redirect.target.clone()]);
    assert!(saved.people_cache.pending_creates.is_empty());
    let archive = &saved.people_cache.merge_archives["source"];
    assert_eq!(
        archive.source.as_ref(),
        Some(&before.people_cache.records[0])
    );
    assert_eq!(
        archive.source_relationship.as_ref(),
        Some(&before.relationships[0])
    );
    assert_eq!(
        archive.target_relationship.as_ref(),
        Some(&before.relationships[1])
    );
    assert!(archive.source_was_pending);
    assert_eq!(saved.relationships.len(), 1);
    assert_eq!(saved.relationships[0].person_id, "target");
    for text in [
        "Private source notes",
        "Private target notes",
        "Source company",
        "Unsynced source name",
    ] {
        assert!(saved.relationships[0].notes.contains(text));
    }
    assert_eq!(saved.relationships[0].items, before.relationships[0].items);
    assert_eq!(
        saved.inbox_conversations[0].participants[0]
            .person_id
            .as_deref(),
        Some("target")
    );
    assert_eq!(
        saved.inbox_conversations[0].messages,
        before.inbox_conversations[0].messages
    );
    assert_eq!(saved.captures[0].target_id.as_deref(), Some("target"));
    assert_eq!(saved.captures[0].content(), before.captures[0].content());
    assert_eq!(saved.knowledge_documents[0].links[0].target_id, "target");
    assert_eq!(
        saved.knowledge_documents[0].body_markdown,
        before.knowledge_documents[0].body_markdown
    );
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, cx| {
            hub.restore_people(saved.people_cache.clone());
            hub.capture_person(before.people_cache.records[0].clone(), cx);
            assert_eq!(hub.people, vec![redirect.target.clone()]);
            assert_eq!(hub.pending_people_count(), 0);
        });
        let old = w.snapshot(cx);
        w.reconcile_person_merge(&redirect, cx).unwrap();
        assert_eq!(
            serde_json::to_value(w.snapshot(cx)).unwrap(),
            serde_json::to_value(old).unwrap()
        );
    });
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn open_profile_and_conflicting_relationship_items_block_without_losing_drafts(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, window, cx| {
        let redirect = install(w, cx);
        w.expanded_person_id = Some("source".into());
        w.relationship_notes_input.update(cx, |input, cx| {
            input.set_value("Unsubmitted draft", window, cx)
        });
        let before = serde_json::to_value(w.snapshot(cx)).unwrap();
        assert!(w
            .reconcile_person_merge(&redirect, cx)
            .unwrap_err()
            .contains("editor"));
        assert_eq!(
            w.relationship_notes_input.read(cx).value().as_str(),
            "Unsubmitted draft"
        );
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
        w.expanded_person_id = None;
        let mut conflicting = w.relationships[0].items[0].clone();
        conflicting.text = "Different contents".into();
        w.relationships[1].items.push(conflicting);
        let before = serde_json::to_value(w.snapshot(cx)).unwrap();
        assert!(w.reconcile_person_merge(&redirect, cx).is_err());
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
        w.relationships[1].items.clear();
        w.reconcile_person_merge(&redirect, cx).unwrap();
        assert!(w.meetings_hub.read(cx).person_is_retired("source"));
    });
}

#[gpui::test]
async fn failed_merge_save_keeps_old_disk_snapshot_and_retry_commits_all_references(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-native-merge-save-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    let redirect = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        w.persistence_test_path = Some(path.clone());
        w.persist_and_notify(cx);
        redirect
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let before = std::fs::read(&path).unwrap();
    let backup = desktop_state_backup_path(&path);
    assert!(!backup.exists());
    std::fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.reconcile_person_merge(&redirect, cx).unwrap()
    });
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    workspace.update_in(cx, |w, _, cx| {
        assert!(w.meetings_hub.read(cx).person_is_retired("source"));
        assert_eq!(w.relationships[0].person_id, "target");
    });
    std::fs::remove_dir(&backup).unwrap();
    cx.run_until_parked();
    let button = cx.debug_bounds("desktop-retry-save").unwrap();
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert!(saved.people_cache.merge_archives.contains_key("source"));
    assert_eq!(saved.relationships[0].person_id, "target");
    assert_eq!(saved.captures[0].target_id.as_deref(), Some("target"));
    std::fs::remove_dir_all(directory).unwrap();
}
