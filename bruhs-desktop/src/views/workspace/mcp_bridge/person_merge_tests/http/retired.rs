use super::*;

#[gpui::test]
async fn refresh_resolves_missing_intermediate_survivor_and_repairs_older_alias(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let (server, original) = workspace.update_in(cx, |w, _, cx| {
        let first = install(w, cx);
        w.reconcile_person_merge(&first, cx).unwrap();
        w.captures[0].target_id = Some("source".into());
        let original = w.meetings_hub.read(cx).people_snapshot().merge_archives["source"].clone();
        let mut final_person = PersonProfile::new("Final survivor");
        final_person.id = "final".into();
        let server = Server::new(
            &cx.global::<super::super::super::super::DesktopState>()
                .runtime,
            PersonMergeResolution {
                source_id: "target".into(),
                target: final_person,
            },
        );
        w.meetings_hub.update(cx, |hub, cx| {
            let mut snapshot = hub.people_snapshot();
            snapshot.records.clear();
            hub.restore_people(snapshot);
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.refresh_people(cx);
        });
        (server, original)
    });
    cx.condition(&workspace, |w, _| {
        w.captures[0].target_id.as_deref() == Some("final")
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        let saved = w.meetings_hub.read(cx).people_snapshot();
        assert_eq!(saved.merge_archives["source"], original);
        assert_eq!(saved.merge_archives["target"].target_id, "final");
        assert!(saved.merge_recoveries.is_empty());
        assert_eq!(w.relationships[0].person_id, "final");
        assert!(!w.meetings_hub.read(cx).needs_people_retry());
    });
    assert!(server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.starts_with("POST /api/people/resolve-merges ")));
    assert!(!server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|r| r.starts_with("POST /api/people ")));
}
