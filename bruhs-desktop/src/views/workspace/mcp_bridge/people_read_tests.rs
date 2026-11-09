use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse, PersonReadSection};
use bruhs_models::{
    AgentAccess, AgentDomain, ObservationKind, PersonObservation, PersonProfile, RelationshipItem,
    RelationshipItemKind, RelationshipProfile,
};
use gpui::TestAppContext;
use serde_json::{json, Value};

mod safety;
mod workflow;

fn details(person_id: &str, section: PersonReadSection, offset: usize) -> BridgeRequest {
    BridgeRequest::ReadPerson {
        person_id: person_id.into(),
        section,
        offset,
    }
}

fn value(response: BridgeResponse) -> Value {
    match response {
        BridgeResponse::Ok(page) => page,
        other => panic!("Expected read page: {other:?}"),
    }
}

#[gpui::test]
async fn granted_people_read_discovers_identity_without_voice_fingerprint(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let mut person = PersonProfile::new("Ada");
        person.id = "exact-ada".into();
        person.voice_embedding = Some(vec![0.125; 512]);
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person]);
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Write, false);
    });
    connect(&workspace, cx).await;
    let response = call(
        &workspace,
        cx,
        BridgeRequest::Read {
            domain: AgentDomain::People,
            query: "Ada".into(),
            offset: 0,
        },
    )
    .await;
    let BridgeResponse::Ok(page) = response else {
        panic!("Granted People Read must discover cached contacts: {response:?}")
    };
    assert_eq!(page["records"][0]["id"], "exact-ada");
    assert_eq!(page["records"][0]["display_name"], "Ada");
    assert!(!page.to_string().contains("voice_embedding"));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn people_summary_search_skips_large_histories_and_private_relationship_notes(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        let mut person = PersonProfile::new("Adá");
        person.id = "person-ada".into();
        person.aliases = vec!["Enchantress".into()];
        person.observations = vec![PersonObservation {
            at_meeting_id: "meeting".into(),
            kind: ObservationKind::Quote,
            text: "Secret-history".repeat(100_000),
            created_at: 5,
        }];
        person.voice_embedding = Some(vec![0.1; 1_000_000]);
        let mut profile = RelationshipProfile::new(&person.id, 7);
        profile.notes = "Secret-relationship".repeat(50_000);
        w.relationships = vec![profile];
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person]);
    });
    connect(&workspace, cx).await;
    for query in ["PERSON-ADA", "ADÁ", "enchantress", ""] {
        let page = value(
            call(
                &workspace,
                cx,
                BridgeRequest::Read {
                    domain: AgentDomain::People,
                    query: query.into(),
                    offset: 0,
                },
            )
            .await,
        );
        assert_eq!(page["records"][0]["id"], "person-ada");
        assert_eq!(page["records"][0]["observation_count"], 1);
        assert!(page.to_string().len() < 512);
    }
    for query in ["Secret-history", "Secret-relationship", "voice_embedding"] {
        let page = value(
            call(
                &workspace,
                cx,
                BridgeRequest::Read {
                    domain: AgentDomain::People,
                    query: query.into(),
                    offset: 0,
                },
            )
            .await,
        );
        assert_eq!(page["records"], json!([]));
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn person_profile_and_paginated_history_preserve_context_without_mutating_workspace(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    let (expected_observations, expected_items) = workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Write, false);
        let mut person = PersonProfile::new("Alex");
        person.id = "exact".into();
        person.aliases = vec!["A".into()];
        person.voice_embedding = Some(vec![0.123; 20]);
        person.observations = (0..123)
            .map(|i| PersonObservation {
                at_meeting_id: format!("meeting-{i}"),
                kind: ObservationKind::Decision,
                text: format!("Decision {i}"),
                created_at: i,
            })
            .collect();
        let mut relationship = RelationshipProfile::new("exact", 55);
        relationship.company = "Company".into();
        relationship.role = "Role".into();
        relationship.email = "alex@example.test".into();
        relationship.phone = "+15550000".into();
        relationship.location = "Local".into();
        relationship.notes = "Private context 🗒".into();
        relationship.last_contacted_at_ms = Some(44);
        relationship.items = (0..123)
            .map(|i| {
                let mut item = RelationshipItem::new(
                    RelationshipItemKind::Reminder,
                    format!("Reminder {i}"),
                    i,
                );
                item.due_at_ms = Some(i + 100);
                item.completed_at_ms = (i % 2 == 0).then_some(i + 200);
                item.source_record_id = Some(format!("source-{i}"));
                item
            })
            .collect();
        let expected = (
            serde_json::to_value(&person.observations).unwrap(),
            serde_json::to_value(&relationship.items).unwrap(),
        );
        w.relationships = vec![relationship];
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person]);
        expected
    });
    connect(&workspace, cx).await;
    let before = workspace.update_in(cx, |w, _, cx| serde_json::to_value(w.snapshot(cx)).unwrap());
    let page = value(
        call(
            &workspace,
            cx,
            details("exact", PersonReadSection::Profile, 0),
        )
        .await,
    );
    assert_eq!(
        page["records"][0]["relationship"],
        json!({"company":"Company","role":"Role","email":"alex@example.test","phone":"+15550000","location":"Local","notes":"Private context 🗒","last_contacted_at_ms":44,"updated_at_ms":55,"item_count":123})
    );
    assert_eq!(page["records"][0]["aliases"], json!(["A"]));
    assert!(page["records"][0].get("voice_embedding").is_none());
    for (section, expected) in [
        (PersonReadSection::Observations, expected_observations),
        (PersonReadSection::RelationshipItems, expected_items),
    ] {
        let mut all = Vec::new();
        for offset in [0, 50, 100] {
            let page = value(call(&workspace, cx, details("exact", section, offset)).await);
            assert_eq!(page["person_id"], "exact");
            assert_eq!(page["section"], json!(section));
            assert_eq!(
                page["next_offset"],
                if offset < 100 {
                    json!(offset + 50)
                } else {
                    Value::Null
                }
            );
            all.extend(page["records"].as_array().unwrap().iter().cloned());
        }
        assert_eq!(Value::Array(all), expected);
    }
    workspace.update_in(cx, |w, _, cx| {
        assert_eq!(serde_json::to_value(w.snapshot(cx)).unwrap(), before);
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
