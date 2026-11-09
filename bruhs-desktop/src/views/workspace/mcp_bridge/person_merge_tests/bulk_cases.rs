use super::bulk::dataset;
use super::*;
use std::sync::Arc;

#[gpui::test]
fn bulk_indexes_do_not_hide_duplicate_contacts_or_relationships(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let (mut people, mut relationships, documents) = dataset(3, 3);
        people.records.push(people.records[1].clone());
        relationships.push(relationships[2].clone());
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
            );
            hub.restore_people(people);
        });
        w.relationships = relationships;
        w.knowledge_documents = documents.clone();
        w.repair_retired_person_references(cx);
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "target-000000");
        assert_eq!(&w.knowledge_documents[1..], &documents[1..]);
        assert_eq!(
            w.relationships
                .iter()
                .filter(|p| p.person_id == "source-000002")
                .count(),
            2
        );
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 2);
        assert_eq!(
            w.meetings_hub
                .read(cx)
                .people_snapshot()
                .merge_recoveries
                .len(),
            1
        );
    });
}

#[gpui::test]
fn bulk_recovery_accumulates_shared_survivor_and_retains_only_conflicting_source(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let (mut people, mut relationships, documents) = dataset(3, 6);
        for archive in people.merge_archives.values_mut() {
            Arc::make_mut(archive).target_id = "target-000000".into();
        }
        people.records.retain(|p| p.id == "target-000000");
        let item = RelationshipItem::new(RelationshipItemKind::Reminder, "Keep original", 1);
        relationships[0].items.push(item.clone());
        let mut conflicting = item;
        conflicting.text = "Conflict stays local".into();
        relationships[1].items.push(conflicting);
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
            );
            hub.restore_people(people.clone());
        });
        w.relationships = relationships.clone();
        w.knowledge_documents = documents;
        w.repair_retired_person_references(cx);
        assert_eq!(w.relationships.len(), 2);
        assert_eq!(
            w.relationships
                .iter()
                .find(|p| p.person_id == "source-000001"),
            Some(&relationships[1])
        );
        let survivor = w
            .relationships
            .iter()
            .find(|p| p.person_id == "target-000000")
            .unwrap();
        assert!(
            survivor.notes.contains("Private context 0")
                && survivor.notes.contains("Private context 2")
        );
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "target-000000");
        assert_eq!(w.knowledge_documents[1].links[0].target_id, "source-000001");
        assert_eq!(w.knowledge_documents[2].links[0].target_id, "target-000000");
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 1);
        assert_eq!(
            w.meetings_hub.read(cx).people_snapshot().merge_archives,
            people.merge_archives
        );
        w.relationships
            .iter_mut()
            .find(|p| p.person_id == "source-000001")
            .unwrap()
            .items
            .clear();
        w.repair_retired_person_references(cx);
        assert_eq!(w.relationships.len(), 1);
        assert!((0..3).all(|index| w.relationships[0]
            .notes
            .contains(&format!("Private context {index}"))));
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
        let saved = serde_json::to_value(w.snapshot(cx)).unwrap();
        w.repair_retired_person_references(cx);
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), saved);
    });
}

#[gpui::test]
async fn bulk_recovery_failed_save_keeps_original_file_and_retry_saves_all_aliases(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-bulk-recovery-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, cx| {
        let (people, relationships, documents) = dataset(32, 100);
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
            );
            hub.restore_people(people);
        });
        w.relationships = relationships;
        w.knowledge_documents = documents;
        w.persistence_test_path = Some(path.clone());
        w.persist_and_notify(cx);
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let before = std::fs::read(&path).unwrap();
    let backup = desktop_state_backup_path(&path);
    std::fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |w, _, cx| w.repair_retired_person_references(cx));
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::remove_dir(&backup).unwrap();
    cx.run_until_parked();
    let retry = cx.debug_bounds("desktop-retry-save").unwrap();
    cx.simulate_click(retry.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.people_cache.merge_recoveries.len(), 32);
    assert!(saved
        .relationships
        .iter()
        .all(|p| p.person_id.starts_with("target-")));
    assert!(saved
        .knowledge_documents
        .iter()
        .all(|d| d.links[0].target_id.starts_with("target-")));
    std::fs::remove_dir_all(directory).unwrap();
}
