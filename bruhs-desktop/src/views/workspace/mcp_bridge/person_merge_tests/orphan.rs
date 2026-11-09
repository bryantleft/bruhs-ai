use super::*;

#[test]
fn legacy_merge_archives_keep_real_source_objects_and_new_archives_allow_explicit_absence() {
    let source = PersonProfile::new("Original source");
    let mut legacy = serde_json::json!({
        "source":source, "target":null, "source_was_pending":false,
        "source_relationship":null, "target_relationship":null, "target_id":"target"
    });
    let restored: crate::views::meetings_hub::PeopleMergeArchive =
        serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(restored.source, Some(source));
    legacy["source"] = serde_json::Value::Null;
    let restored: crate::views::meetings_hub::PeopleMergeArchive =
        serde_json::from_value(legacy).unwrap();
    assert!(restored.source.is_none());
}

#[gpui::test]
fn reference_discovery_includes_only_typed_current_person_links(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, cx);
        w.relationships = vec![RelationshipProfile::new("relationship-person", 1)];
        w.captures[0].target_id = Some("capture-person".into());
        w.knowledge_documents[0].links[0].target_id = "knowledge-person".into();
        w.inbox_conversations[0].participants[0].person_id = Some("inbox-person".into());
        w.meetings_hub.update(cx, |hub, _| {
            let mut meeting = Meeting::new("Meeting", 1.0);
            meeting.speakers[0].person_id = Some("speaker-person".into());
            meeting.summary = Some(MeetingSummary {
                tldr: "Private summary, not an identity".into(),
                action_items: vec![ActionItem {
                    text: "Private action, not an identity".into(),
                    assignee_person_id: Some("assignee-person".into()),
                    due_hint: None,
                }],
                decisions: vec![],
                topics: vec![],
                generated_by: String::new(),
                generated_at: 1,
            });
            hub.meetings = vec![meeting];
        });
        w.captures.push(
            CaptureRecord::new(CaptureKind::Task, "Task", 1)
                .unwrap()
                .with_target("not-a-person"),
        );
        w.knowledge_documents[0].links.push(KnowledgeLink {
            kind: KnowledgeLinkKind::Task,
            target_id: "not-a-person-either".into(),
            label: "Task".into(),
        });
        let ids = w.referenced_person_ids(cx);
        assert_eq!(
            ids,
            [
                "relationship-person",
                "capture-person",
                "knowledge-person",
                "inbox-person",
                "speaker-person",
                "assignee-person"
            ]
            .map(str::to_string)
            .into_iter()
            .collect()
        );
    });
}

#[gpui::test]
fn orphan_relationship_and_links_reconcile_without_inventing_a_source_profile(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        w.meetings_hub.update(cx, |hub, _| {
            let mut saved = hub.people_snapshot();
            saved.records.retain(|person| person.id != "source");
            saved.pending_creates.clear();
            hub.restore_people(saved);
        });
        let result = w.reconcile_person_merge(&redirect, cx);
        assert!(
            result.is_ok(),
            "A confirmed redirect must recover orphan references: {result:?}"
        );
        assert_eq!(w.relationships[0].person_id, "target");
        assert!(w.relationships[0].notes.contains("Private source notes"));
        assert_eq!(w.captures[0].target_id.as_deref(), Some("target"));
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "target");
        assert_eq!(
            w.inbox_conversations[0].participants[0]
                .person_id
                .as_deref(),
            Some("target")
        );
        let saved = serde_json::to_value(w.snapshot(cx)).unwrap();
        assert!(saved["people_cache"]["merge_archives"]["source"]["source"].is_null());
        assert_eq!(
            saved["people_cache"]["records"].as_array().unwrap().len(),
            1
        );
    });
}
