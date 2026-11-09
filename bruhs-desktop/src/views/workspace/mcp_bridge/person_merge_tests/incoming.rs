use super::*;
use std::time::Instant;

fn redirect(source: &str, target: &str) -> PersonMergeResolution {
    let mut profile = PersonProfile::new(format!("Contact {target}"));
    profile.id = target.into();
    PersonMergeResolution {
        source_id: source.into(),
        target: profile,
    }
}

#[gpui::test]
fn incoming_invalid_components_retain_originals_while_independent_sources_recover(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, cx);
        let before = w.relationships.clone();
        let incoming = vec![
            redirect("source", "loop"),
            redirect("loop", "source"),
            redirect("duplicate", "target"),
            redirect("duplicate", "elsewhere"),
            redirect("blank", " "),
            redirect("independent", "survivor"),
        ];
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.relationships, before);
        assert_eq!(w.captures[0].target_id.as_deref(), Some("source"));
        let hub = w.meetings_hub.read(cx);
        assert_eq!(hub.people_snapshot().merge_archives.len(), 1);
        assert!(hub.person_is_retired("independent"));
        assert_eq!(hub.pending_people_count(), 4);
    });
}

#[gpui::test]
fn incoming_deep_chain_is_iterative_and_replay_does_not_duplicate_archives(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, cx);
        let mut incoming = vec![redirect("source", "chain-00000")];
        incoming.extend((0..10_000).map(|index| {
            redirect(
                &format!("chain-{index:05}"),
                &format!("chain-{:05}", index + 1),
            )
        }));
        incoming.reverse();
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.captures[0].target_id.as_deref(), Some("chain-10000"));
        assert_eq!(
            w.meetings_hub
                .read(cx)
                .people_snapshot()
                .merge_archives
                .len(),
            10_001
        );
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
        let saved = serde_json::to_value(w.snapshot(cx)).unwrap();
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), saved);
    });
}

#[gpui::test]
fn incoming_chain_keeps_open_intermediate_draft_and_recovers_it_after_close(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let first = install(w, cx);
        let target_before = w.relationships[1].clone();
        w.expanded_person_id = Some("target".into());
        let incoming = vec![redirect("target", "final"), first];
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.captures[0].target_id.as_deref(), Some("final"));
        assert_eq!(
            w.relationships.iter().find(|r| r.person_id == "target"),
            Some(&target_before)
        );
        assert_eq!(w.expanded_person_id.as_deref(), Some("target"));
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 1);
        w.expanded_person_id = None;
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
        assert_eq!(w.relationships.len(), 1);
        assert!(w.relationships[0].notes.contains("Private source notes"));
        assert!(w.relationships[0].notes.contains("Private target notes"));
    });
}

#[gpui::test]
fn incoming_chain_follows_saved_evidence_without_rewriting_original_archives(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, cx);
        w.reconcile_person_merges(&[redirect("target", "final")], cx);
        let original = w.meetings_hub.read(cx).people_snapshot().merge_archives["target"].clone();
        w.reconcile_person_merges(
            &[
                redirect("source", "target"),
                redirect("target", "unconfirmed"),
            ],
            cx,
        );
        assert_eq!(w.captures[0].target_id.as_deref(), Some("final"));
        let hub = w.meetings_hub.read(cx);
        assert_eq!(hub.people_snapshot().merge_archives["target"], original);
        assert!(!hub.people.iter().any(|p| p.id == "unconfirmed"));
        assert_eq!(hub.pending_people_count(), 0);
    });
}

#[gpui::test]
fn incoming_merge_chain_recovers_all_references_regardless_of_reply_order(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let first = install(w, cx);
        let mut survivor = PersonProfile::new("Final survivor");
        survivor.id = "final".into();
        w.meetings_hub.update(cx, |_, cx| {
            cx.emit(HubEvent::PersonMergesResolved(vec![
                PersonMergeResolution {
                    source_id: "target".into(),
                    target: survivor,
                },
                first,
            ]));
        });
    });
    cx.run_until_parked();
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(w.captures[0].target_id.as_deref(), Some("final"));
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "final");
        assert_eq!(
            w.inbox_conversations[0].participants[0]
                .person_id
                .as_deref(),
            Some("final")
        );
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
    });
}

#[gpui::test]
fn incoming_contact_batch_timing(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let (count, document_count, warmups, samples) = if cfg!(debug_assertions) {
        (32, 1_000, 0, 1)
    } else {
        (1_000, 10_000, 3, 20)
    };
    let (mut people, relationships, documents) = super::bulk::dataset(count, document_count);
    people.merge_archives.clear();
    let incoming: Vec<_> = people
        .records
        .iter()
        .enumerate()
        .map(|(index, target)| PersonMergeResolution {
            source_id: format!("source-{index:06}"),
            target: target.clone(),
        })
        .collect();
    let mut times = Vec::new();
    for sample in 0..warmups + samples {
        workspace.update_in(cx, |w, _, cx| {
            w.meetings_hub.update(cx, |hub, _| {
                hub.set_people_test_client(
                    ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                );
                hub.restore_people(people.clone());
            });
            w.relationships = relationships.clone();
            w.knowledge_documents = documents.clone();
            let started = Instant::now();
            w.reconcile_person_merges(&incoming, cx);
            let elapsed = started.elapsed().as_secs_f64() * 1_000.;
            if sample >= warmups {
                times.push(elapsed);
            }
            assert_eq!(
                w.meetings_hub
                    .read(cx)
                    .people_snapshot()
                    .merge_archives
                    .len(),
                count
            );
            assert!(w
                .knowledge_documents
                .iter()
                .all(|d| d.links[0].target_id.starts_with("target-")));
        });
    }
    times.sort_by(f64::total_cmp);
    eprintln!("incoming recovery: merges={count} documents={document_count} samples={samples}; synchronous batch handling p50={:.3}ms p95={:.3}ms p99={:.3}ms; excludes fixture creation, GPUI event delivery/rendering, network completion and disk save", times[times.len()/2], times[(times.len()*95/100).min(times.len()-1)], times[(times.len()*99/100).min(times.len()-1)]);
}

#[gpui::test]
fn incoming_duplicate_cache_identities_do_not_get_hidden_by_batch_indexes(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, cx);
        let (mut people, mut relationships, documents) = super::bulk::dataset(3, 3);
        people.merge_archives.clear();
        let incoming: Vec<_> = (0..3)
            .map(|index| redirect(&format!("source-{index:06}"), &format!("target-{index:06}")))
            .collect();
        people.records.push(people.records[1].clone());
        relationships.push(relationships[2].clone());
        w.meetings_hub
            .update(cx, |hub, _| hub.restore_people(people));
        w.relationships = relationships;
        w.knowledge_documents = documents.clone();
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.knowledge_documents[0].links[0].target_id, "target-000000");
        assert_eq!(&w.knowledge_documents[1..], &documents[1..]);
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 2);
        assert_eq!(
            w.meetings_hub
                .read(cx)
                .people_snapshot()
                .merge_archives
                .len(),
            1
        );
    });
}

#[gpui::test]
fn incoming_shared_survivor_keeps_newest_profile_and_conflicting_source_until_retry(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        install(w, cx);
        let (mut people, mut relationships, documents) = super::bulk::dataset(3, 3);
        people.merge_archives.clear();
        people.records.truncate(1);
        people.records[0].display_name = "Newer cached survivor".into();
        people.records[0].updated_at = 30;
        let original = people.records[0].clone();
        let mut incoming: Vec<_> = (0..3)
            .map(|index| redirect(&format!("source-{index:06}"), "target-000000"))
            .collect();
        incoming[1].target.updated_at = 20;
        let item = RelationshipItem::new(RelationshipItemKind::Reminder, "Keep original", 1);
        relationships[0].items.push(item.clone());
        let mut conflict = item;
        conflict.text = "Keep conflicting source".into();
        relationships[1].items.push(conflict);
        let conflicting = relationships[1].clone();
        w.meetings_hub
            .update(cx, |hub, _| hub.restore_people(people));
        w.relationships = relationships;
        w.knowledge_documents = documents;
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.meetings_hub.read(cx).people, vec![original.clone()]);
        assert_eq!(
            w.relationships
                .iter()
                .find(|r| r.person_id == "source-000001"),
            Some(&conflicting)
        );
        assert_eq!(w.knowledge_documents[1].links[0].target_id, "source-000001");
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 1);
        w.relationships
            .iter_mut()
            .find(|r| r.person_id == "source-000001")
            .unwrap()
            .items
            .clear();
        w.reconcile_person_merges(&incoming, cx);
        assert_eq!(w.meetings_hub.read(cx).people, vec![original]);
        assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
        assert_eq!(w.relationships.len(), 1);
        assert!((0..3).all(|index| w.relationships[0]
            .notes
            .contains(&format!("Private context {index}"))));
    });
}
