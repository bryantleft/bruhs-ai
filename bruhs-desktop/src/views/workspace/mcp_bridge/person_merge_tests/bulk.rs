use super::*;
use crate::views::meetings_hub::PeopleMergeArchive;
use std::{sync::Arc, time::Instant};

pub(super) fn dataset(
    count: usize,
    documents: usize,
) -> (
    PeopleSnapshot,
    Vec<RelationshipProfile>,
    Vec<KnowledgeDocument>,
) {
    let mut people = PeopleSnapshot::default();
    let mut relationships = Vec::new();
    for index in 0..count {
        let source_id = format!("source-{index:06}");
        let mut target = PersonProfile::new("Surviving contact");
        target.id = format!("target-{index:06}");
        people.merge_archives.insert(
            source_id.clone(),
            Arc::new(PeopleMergeArchive {
                source: None,
                target: Some(target.clone()),
                source_was_pending: false,
                source_relationship: None,
                target_relationship: None,
                target_id: target.id.clone(),
            }),
        );
        people.records.push(target);
        let mut relationship = RelationshipProfile::new(source_id, 1);
        relationship.notes = format!("Private context {index}");
        relationships.push(relationship);
    }
    let documents = (0..documents)
        .map(|index| {
            let mut document = KnowledgeDocument::new("Document", "Body retained", 1);
            document.links.push(KnowledgeLink {
                kind: KnowledgeLinkKind::Person,
                target_id: format!("source-{:06}", index % count),
                label: "Original label retained".into(),
            });
            document
        })
        .collect();
    (people, relationships, documents)
}

#[gpui::test]
fn bulk_retired_contact_recovery_timing(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let (count, documents, warmups, samples) = if cfg!(debug_assertions) {
        (32, 1_000, 0, 1)
    } else {
        (1_000, 10_000, 3, 20)
    };
    let (people, relationships, documents) = dataset(count, documents);
    let mut timings = Vec::new();
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
            w.repair_retired_person_references(cx);
            let elapsed = started.elapsed().as_secs_f64() * 1_000.;
            if sample >= warmups {
                timings.push(elapsed);
            }
            assert_eq!(w.relationships.len(), count);
            assert!(w
                .relationships
                .iter()
                .all(|p| p.person_id.starts_with("target-")));
            assert!(w.knowledge_documents.iter().all(|d| d.links[0]
                .target_id
                .starts_with("target-")
                && d.links[0].label == "Original label retained"
                && d.body_markdown == "Body retained"));
            assert_eq!(w.meetings_hub.read(cx).pending_people_count(), 0);
        });
    }
    timings.sort_by(f64::total_cmp);
    eprintln!("bulk recovery: aliases={count} documents={} samples={samples}; synchronous boundary p50={:.3}ms p95={:.3}ms p99={:.3}ms; excludes fixture construction, rendering, transport completion and disk save", documents.len(), timings[timings.len()/2], timings[(timings.len()*95/100).min(timings.len()-1)], timings[(timings.len()*99/100).min(timings.len()-1)]);
}
