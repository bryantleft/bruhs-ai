use super::*;

#[gpui::test]
async fn people_read_revocation_precedes_identity_lookup_and_write_does_not_authorize_reads(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        let mut person = PersonProfile::new("Alex");
        person.id = "existing".into();
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person]);
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
            .update(cx, |hub, _| hub.people = vec![person]);
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
            hub.people[0].observations[0].text = "x".repeat(600_000)
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
        w.meetings_hub
            .update(cx, |hub, _| hub.people = vec![person.clone(), person]);
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
