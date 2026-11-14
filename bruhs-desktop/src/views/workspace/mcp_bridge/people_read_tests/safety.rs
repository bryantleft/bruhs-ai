use super::*;
use std::sync::Arc;

#[gpui::test]
async fn people_read_revocation_precedes_identity_lookup_and_write_does_not_authorize_reads(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let mut person = PersonProfile::new("Alex");
        person.id = "existing".into();
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person.into()]);
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, false);
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Write, true);
    });
    connect(&workspace, cx).await;
    for id in ["existing", "missing", ""] {
        for section in [
            PersonReadSection::Profile,
            PersonReadSection::Observations,
            PersonReadSection::RelationshipItems,
        ] {
            let response = call(&workspace, cx, details(id, section, 0)).await;
            assert!(
                matches!(response, BridgeResponse::Error(ref message) if message == "Read access is not granted for this domain.")
            );
        }
    }
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true)
    });
    let page = value(
        call(
            &workspace,
            cx,
            details("existing", PersonReadSection::Profile, 0),
        )
        .await,
    );
    assert!(page["records"][0]["relationship"].is_null());
    assert_eq!(
        value(
            call(
                &workspace,
                cx,
                details("existing", PersonReadSection::RelationshipItems, 0)
            )
            .await
        )["records"],
        json!([])
    );
    for request in [
        details("missing", PersonReadSection::Profile, 0),
        details("existing", PersonReadSection::Profile, 1),
        details("existing", PersonReadSection::Observations, 100_001),
    ] {
        assert!(matches!(
            call(&workspace, cx, request).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, false)
    });
    assert!(matches!(
        call(
            &workspace,
            cx,
            BridgeRequest::Read {
                domain: AgentDomain::People,
                query: String::new(),
                offset: 0
            }
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn person_history_byte_pages_resume_and_oversized_records_fail_without_losing_identity(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        let mut person = PersonProfile::new("Large history");
        person.id = "large".into();
        person.observations = (0..2)
            .map(|i| PersonObservation {
                at_meeting_id: format!("m{i}"),
                kind: ObservationKind::Quote,
                text: "x".repeat(300_000),
                created_at: i,
            })
            .collect();
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person.into()]);
    });
    connect(&workspace, cx).await;
    let first = value(
        call(
            &workspace,
            cx,
            details("large", PersonReadSection::Observations, 0),
        )
        .await,
    );
    assert_eq!(first["next_offset"], 1);
    assert_eq!(first["records"].as_array().unwrap().len(), 1);
    assert!(first.to_string().len() <= 512 * 1024);
    let second = value(
        call(
            &workspace,
            cx,
            details("large", PersonReadSection::Observations, 1),
        )
        .await,
    );
    assert!(second["next_offset"].is_null());
    assert_eq!(second["records"][0]["at_meeting_id"], "m1");
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            Arc::make_mut(&mut hub.people[0]).observations[0].text = "x".repeat(600_000)
        })
    });
    assert!(matches!(
        call(
            &workspace,
            cx,
            details("large", PersonReadSection::Observations, 0)
        )
        .await,
        BridgeResponse::Error(_)
    ));
    let summary = value(
        call(
            &workspace,
            cx,
            BridgeRequest::Read {
                domain: AgentDomain::People,
                query: "large".into(),
                offset: 0,
            },
        )
        .await,
    );
    assert_eq!(summary["records"][0]["id"], "large");
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn duplicate_people_or_relationship_keys_never_select_an_arbitrary_profile(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        let mut person = PersonProfile::new("Duplicate");
        person.id = "same-id".into();
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![person.clone().into(), person.into()]
        });
    });
    connect(&workspace, cx).await;
    assert!(matches!(
        call(
            &workspace,
            cx,
            details("same-id", PersonReadSection::Profile, 0)
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, cx| {
        w.meetings_hub.update(cx, |hub, _| {
            hub.people.pop();
        });
        w.relationships = vec![
            RelationshipProfile::new("same-id", 1),
            RelationshipProfile::new("same-id", 2),
        ];
    });
    assert!(matches!(
        call(
            &workspace,
            cx,
            details("same-id", PersonReadSection::Profile, 0)
        )
        .await,
        BridgeResponse::Error(_)
    ));
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

/// Opting a person out of observation tracking is a privacy guarantee: their
/// stored observations must never reach agent context through any read.
#[gpui::test]
async fn opted_out_person_observations_never_reach_agent_reads(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.automation_state
            .permissions
            .set(AgentDomain::People, AgentAccess::Read, true);
        let mut person = PersonProfile::new("Opted Out");
        person.id = "opted-out".into();
        person.tracked = false;
        person.observations = vec![PersonObservation {
            at_meeting_id: "meeting-private".into(),
            kind: ObservationKind::Tone,
            text: "Synthetic-private-observation".into(),
            created_at: 3,
        }];
        let mut tracked = PersonProfile::new("Tracked");
        tracked.id = "tracked".into();
        tracked.observations = vec![PersonObservation {
            at_meeting_id: "meeting-shared".into(),
            kind: ObservationKind::Decision,
            text: "Synthetic-shared-observation".into(),
            created_at: 4,
        }];
        w.meetings_hub.update(cx, |hub, _| {
            hub.people = vec![person.into(), tracked.into()]
        });
    });
    connect(&workspace, cx).await;
    let mut seen = String::new();
    for query in ["", "Opted", "opted-out"] {
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
        let opted = page["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["id"] == "opted-out")
            .expect("identity stays discoverable so the agent can respect the opt-out");
        assert_eq!(opted["tracked"], false);
        assert_eq!(opted["observations_withheld"], true);
        assert!(opted["observation_count"].is_null());
        seen.push_str(&page.to_string());
    }
    for section in [
        PersonReadSection::Profile,
        PersonReadSection::Observations,
        PersonReadSection::RelationshipItems,
    ] {
        let page = value(call(&workspace, cx, details("opted-out", section, 0)).await);
        if section == PersonReadSection::Observations {
            assert_eq!(page["records"], json!([]));
            assert_eq!(page["observations_withheld"], true);
            assert!(page["next_offset"].is_null());
        }
        seen.push_str(&page.to_string());
    }
    assert!(!seen.contains("Synthetic-private-observation"), "{seen}");
    assert!(!seen.contains("meeting-private"), "{seen}");
    let tracked = value(
        call(
            &workspace,
            cx,
            details("tracked", PersonReadSection::Observations, 0),
        )
        .await,
    );
    assert_eq!(
        tracked["records"][0]["text"],
        "Synthetic-shared-observation"
    );
    assert!(tracked.get("observations_withheld").is_none());
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}
