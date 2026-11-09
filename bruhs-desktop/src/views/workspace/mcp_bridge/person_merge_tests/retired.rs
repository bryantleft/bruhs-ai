use super::*;

#[gpui::test]
async fn retired_person_reconciliation_repairs_late_references_without_replacing_originals(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-late-relationship-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        w.persistence_test_path = Some(path.clone());
        w.reconcile_person_merge(&redirect, cx).unwrap();
        let original = w.meetings_hub.read(cx).people_snapshot().merge_archives["source"].clone();
        w.meetings_hub.update(cx, |hub, _| {
            let mut snapshot = hub.people_snapshot();
            let mut late_person = PersonProfile::new("Later recovered name");
            late_person.id = "source".into();
            snapshot.records.push(late_person);
            snapshot.pending_creates.insert("source".into());
            hub.restore_people(snapshot);
        });
        let mut late = RelationshipProfile::new("source", 20);
        late.notes = "Recovered after merge".into();
        w.relationships.push(late);
        w.captures[0].target_id = Some("source".into());
        w.knowledge_documents[0].links[0].target_id = "source".into();
        w.inbox_conversations[0].participants[0].person_id = Some("source".into());
        w.reconcile_person_merge(&redirect, cx).unwrap();
        assert_eq!(w.relationships.len(), 1);
        assert!(w.relationships[0].notes.contains("Recovered after merge"));
        assert_eq!(w.captures[0].target_id.as_deref(), Some("target"));
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "target");
        assert_eq!(
            w.inbox_conversations[0].participants[0]
                .person_id
                .as_deref(),
            Some("target")
        );
        assert_eq!(
            w.meetings_hub.read(cx).people_snapshot().merge_archives["source"],
            original
        );
        let saved = w.meetings_hub.read(cx).people_snapshot();
        assert_eq!(saved.merge_recoveries["source"].len(), 1);
        assert_eq!(
            saved.merge_recoveries["source"][0]
                .source_relationship
                .as_ref()
                .unwrap()
                .notes,
            "Recovered after merge"
        );
        assert_eq!(
            saved.merge_recoveries["source"][0]
                .source
                .as_ref()
                .unwrap()
                .display_name,
            "Later recovered name"
        );
        w.reconcile_person_merge(&redirect, cx).unwrap();
        assert_eq!(w.meetings_hub.read(cx).people_snapshot(), saved);
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(
        saved.people_cache.merge_recoveries["source"][0]
            .source_relationship
            .as_ref()
            .unwrap()
            .notes,
        "Recovered after merge"
    );
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_people(saved.people_cache.clone());
            assert_eq!(hub.people_snapshot(), saved.people_cache);
        });
    });
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
fn document_link_resolution_is_typed_and_atomic_when_a_saved_alias_is_invalid(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        w.reconcile_person_merge(&redirect, cx).unwrap();
        w.meetings_hub.update(cx, |hub, _| {
            let mut links = vec![
                KnowledgeLink {
                    kind: KnowledgeLinkKind::Person,
                    target_id: "source".into(),
                    label: "Original".into(),
                },
                KnowledgeLink {
                    kind: KnowledgeLinkKind::Task,
                    target_id: "source".into(),
                    label: "Task".into(),
                },
            ];
            hub.canonicalize_person_links(&mut links).unwrap();
            assert_eq!(links[0].target_id, "target");
            assert_eq!(links[0].label, "Original");
            assert_eq!(links[1].target_id, "source");
            let mut snapshot = hub.people_snapshot();
            let mut bad = (*snapshot.merge_archives["source"]).clone();
            bad.target_id = "".into();
            snapshot
                .merge_archives
                .insert("bad".into(), std::sync::Arc::new(bad));
            hub.restore_people(snapshot);
            links[0].target_id = "source".into();
            links.push(KnowledgeLink {
                kind: KnowledgeLinkKind::Person,
                target_id: "bad".into(),
                label: "Invalid".into(),
            });
            let before = links.clone();
            assert!(hub.canonicalize_person_links(&mut links).is_err());
            assert_eq!(links, before);
        });
    });
}

#[gpui::test]
fn saved_redirect_chain_reaches_latest_survivor_and_rejects_old_or_cyclic_targets(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let first = install(w, cx);
        w.reconcile_person_merge(&first, cx).unwrap();
        let mut final_person = PersonProfile::new("Latest survivor");
        final_person.id = "final".into();
        w.reconcile_person_merge(
            &PersonMergeResolution {
                source_id: "target".into(),
                target: final_person,
            },
            cx,
        )
        .unwrap();
        w.knowledge_documents[0].links[0].target_id = "source".into();
        let before = serde_json::to_value(w.snapshot(cx)).unwrap();
        assert!(w.reconcile_person_merge(&first, cx).is_err());
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
        w.repair_retired_person_references(cx);
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "final");
        w.meetings_hub.update(cx, |hub, _| {
            let mut snapshot = hub.people_snapshot();
            let mut bad = (*snapshot.merge_archives["target"]).clone();
            bad.target_id = "source".into();
            snapshot
                .merge_archives
                .insert("final".into(), std::sync::Arc::new(bad));
            hub.restore_people(snapshot);
            assert!(hub
                .canonical_person_id("source")
                .unwrap_err()
                .contains("cycle"));
        });
        w.captures[0].target_id = Some("source".into());
        let before = serde_json::to_value(w.snapshot(cx)).unwrap();
        w.repair_retired_person_references(cx);
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
        assert!(w.meetings_hub.read(cx).needs_people_retry());
    });
}

#[gpui::test]
fn retired_recovery_retains_open_draft_and_retries_after_editor_closes(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, window, cx| {
        let redirect = install(w, cx);
        w.reconcile_person_merge(&redirect, cx).unwrap();
        w.relationships.push(RelationshipProfile::new("source", 20));
        w.expanded_person_id = Some("target".into());
        w.relationship_notes_input
            .update(cx, |input, cx| input.set_value("Still editing", window, cx));
        let before = serde_json::to_value(w.snapshot(cx)).unwrap();
        w.repair_retired_person_references(cx);
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
        assert_eq!(
            w.relationship_notes_input.read(cx).value().as_str(),
            "Still editing"
        );
        assert!(w.meetings_hub.read(cx).needs_people_retry());
        w.expanded_person_id = None;
        w.repair_retired_person_references(cx);
        assert!(!w.meetings_hub.read(cx).needs_people_retry());
        assert_eq!(w.relationships.len(), 1);
    });
}

#[gpui::test]
async fn native_document_history_restore_saves_canonical_link_and_preserves_history(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-retired-document-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    let (version, original) = workspace.update_in(cx, |w, window, cx| {
        let redirect = install(w, cx);
        w.knowledge_documents[0].update_links(vec![], 30);
        let version = w.knowledge_documents[0].versions[0].clone();
        w.reconcile_person_merge(&redirect, cx).unwrap();
        let original = w.meetings_hub.read(cx).people_snapshot().merge_archives["source"].clone();
        w.persistence_test_path = Some(path.clone());
        let id = w.knowledge_documents[0].id.clone();
        w.select_knowledge_document(&id, window, cx);
        (version, original)
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let before = std::fs::read(&path).unwrap();
    let backup = desktop_state_backup_path(&path);
    std::fs::create_dir(&backup).unwrap();
    cx.run_until_parked();
    let selector = format!("desktop-knowledge-version-{}", version.id).leak();
    let control = cx.debug_bounds(selector).unwrap();
    cx.simulate_click(control.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "target")
    });
    std::fs::remove_dir(&backup).unwrap();
    cx.run_until_parked();
    let retry = cx.debug_bounds("desktop-retry-save").unwrap();
    cx.simulate_click(retry.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| {
        w.save_status.is_saved() && !w.knowledge_documents[0].links.is_empty()
    })
    .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.knowledge_documents[0].links[0].target_id, "target");
    assert_eq!(
        saved.knowledge_documents[0]
            .versions
            .iter()
            .find(|v| v.id == version.id),
        Some(&version)
    );
    assert_eq!(saved.people_cache.merge_archives["source"], original);
    assert!(saved.people_cache.merge_recoveries.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}
