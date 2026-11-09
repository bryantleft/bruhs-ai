use super::*;
use std::cell::Cell;
use std::rc::Rc;

#[gpui::test]
async fn incoming_http_batches_commit_once_and_native_save_retry_preserves_all_originals(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-incoming-batch-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    let events = Rc::new(Cell::new(0));
    let (server, _subscription) = workspace.update_in(cx, |w, _, cx| {
        let (mut people, relationships, documents) = super::super::bulk::dataset(130, 1_000);
        people.merge_archives.clear();
        let mut redirects: Vec<_> = people
            .records
            .iter()
            .enumerate()
            .map(|(index, target)| PersonMergeResolution {
                source_id: format!("source-{index:06}"),
                target: target.clone(),
            })
            .collect();
        // One intermediate can land in a different, concurrently fetched chunk.
        redirects[129].target.id = "source-000000".into();
        for redirect in &redirects {
            let mut source = PersonProfile::new(format!("Local {}", redirect.source_id));
            source.id = redirect.source_id.clone();
            people.pending_creates.insert(source.id.clone());
            people.records.push(source);
        }
        let server = Server::new_batch(
            &cx.global::<super::super::super::super::DesktopState>()
                .runtime,
            redirects,
        );
        w.meetings_hub.update(cx, |hub, _| {
            hub.set_people_test_client(ServerClient::new(&server.base, &server.base).unwrap());
            hub.restore_people(people);
        });
        w.relationships = relationships;
        w.knowledge_documents = documents;
        w.persistence_test_path = Some(path.clone());
        w.persist_and_notify(cx);
        let events = events.clone();
        let subscription = cx.subscribe(&w.meetings_hub, move |_, _, event, _| {
            if matches!(event, HubEvent::PersonMergesResolved(_)) {
                events.set(events.get() + 1);
            }
        });
        (server, subscription)
    });
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let before = std::fs::read(&path).unwrap();
    let backup = desktop_state_backup_path(&path);
    std::fs::create_dir(&backup).unwrap();
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, cx| hub.refresh_people(cx))
    });
    cx.condition(&workspace, |w, _| {
        matches!(w.save_status, SaveStatus::Failed(_))
    })
    .await;
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(events.get(), 1);
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
        assert_eq!(
            w.meetings_hub
                .read(cx)
                .people_snapshot()
                .merge_archives
                .len(),
            130
        );
    });
    std::fs::remove_dir(&backup).unwrap();
    cx.run_until_parked();
    let retry = cx.debug_bounds("desktop-retry-save").unwrap();
    cx.simulate_click(retry.center(), gpui::Modifiers::default());
    cx.condition(&workspace, |w, _| w.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
    assert_eq!(saved.people_cache.merge_archives.len(), 130);
    for index in 0..130 {
        let source = format!("source-{index:06}");
        let target_index = if index == 129 { 0 } else { index };
        let target = format!("target-{target_index:06}");
        let archive = &saved.people_cache.merge_archives[&source];
        assert_eq!(archive.source.as_ref().unwrap().id, source);
        assert_eq!(
            archive.source_relationship.as_ref().unwrap().notes,
            format!("Private context {index}")
        );
        assert_eq!(archive.target_id, target);
        assert_eq!(saved.knowledge_documents[index].links[0].target_id, target);
        assert!(saved
            .relationships
            .iter()
            .find(|r| r.person_id == target)
            .unwrap()
            .notes
            .contains(&format!("Private context {index}")));
    }
    let requests = server.requests.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.starts_with("POST /api/people/resolve-merges "))
            .count(),
        3
    );
    assert!(!requests.iter().any(|r| r.starts_with("POST /api/people ")));
    std::fs::remove_dir_all(directory).unwrap();
    drop(requests);
}
