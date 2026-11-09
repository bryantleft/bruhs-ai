use super::*;

fn remove_source(w: &mut Workspace, cx: &mut gpui::Context<'_, Workspace>) {
    w.meetings_hub.update(cx, |hub, _| {
        let mut saved = hub.people_snapshot();
        saved.records.retain(|person| person.id != "source");
        saved.pending_creates.clear();
        hub.restore_people(saved);
    });
}

#[gpui::test]
async fn orphan_references_enter_real_refresh_and_save_without_a_fabricated_contact(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-orphan-merge-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    let server = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        remove_source(w, cx);
        w.persistence_test_path = Some(path.clone());
        let server = Server::new(
            &cx.global::<super::super::super::super::DesktopState>()
                .runtime,
            redirect,
        );
        w.meetings_hub.update(cx, |hub, cx| {
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.apply_meeting_event(
                MeetingEvent::PersonMerged {
                    source_id: "source".into(),
                },
                cx,
            );
        });
        server
    });
    cx.condition(&workspace, |w, cx| {
        w.save_status.is_saved() && w.meetings_hub.read(cx).person_is_retired("source")
    })
    .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    let archive = &saved.people_cache.merge_archives["source"];
    assert!(archive.source.is_none());
    assert!(!archive.source_was_pending);
    assert_eq!(
        archive.source_relationship.as_ref().unwrap().notes,
        "Private source notes"
    );
    assert_eq!(saved.people_cache.records.len(), 1);
    assert_eq!(saved.people_cache.records[0].id, "target");
    assert_eq!(saved.relationships[0].person_id, "target");
    assert_eq!(saved.captures[0].target_id.as_deref(), Some("target"));
    assert_eq!(saved.knowledge_documents[0].links[0].target_id, "target");
    assert_eq!(
        saved.inbox_conversations[0].participants[0]
            .person_id
            .as_deref(),
        Some("target")
    );
    assert!(!server
        .requests
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.starts_with("POST /api/people ")));
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.restore_people(saved.people_cache.clone());
            assert!(hub.person_is_retired("source"));
            assert!(hub.people_snapshot().merge_archives["source"]
                .source
                .is_none());
        })
    });
    std::fs::remove_dir_all(directory).unwrap();
}

#[gpui::test]
async fn orphan_lookup_failure_keeps_a_visible_retry_without_a_pending_create(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let (server, before) = workspace.update_in(cx, |w, _, cx| {
        let redirect = install(w, cx);
        remove_source(w, cx);
        w.active_view = AppView::People;
        let server = Server::new(
            &cx.global::<super::super::super::super::DesktopState>()
                .runtime,
            redirect,
        );
        server.mode.store(2, Ordering::SeqCst);
        let before = serde_json::to_value(w.snapshot(cx)).unwrap();
        w.meetings_hub.update(cx, |hub, cx| {
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.refresh_people(cx);
        });
        (server, before)
    });
    cx.condition(&workspace, |w, cx| w.meetings_hub.read(cx).notice.is_some())
        .await;
    workspace.update_in(cx, |w, _, cx| {
        let hub = w.meetings_hub.read(cx);
        assert_eq!(hub.pending_people_count(), 0);
        assert!(hub.needs_people_retry());
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
    });
    server.mode.store(0, Ordering::SeqCst);
    for selector in ["desktop-hub-notice-dismiss", "desktop-retry-contact-sync"] {
        cx.run_until_parked();
        let control = cx.debug_bounds(selector).unwrap();
        cx.simulate_click(control.center(), gpui::Modifiers::default());
    }
    cx.condition(&workspace, |w, cx| {
        w.meetings_hub.read(cx).person_is_retired("source")
    })
    .await;
    workspace.update_in(cx, |w, _, cx| {
        assert!(!w.meetings_hub.read(cx).needs_people_retry())
    });
}
