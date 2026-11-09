use super::*;

#[test]
fn life_read_wire_defaults_and_strict_fields_preserve_app_owned_privacy() {
    for (kind, default) in [
        ("read_finance", "transactions"),
        ("read_health", "workouts"),
    ] {
        let request: BridgeRequest =
            serde_json::from_value(serde_json::json!({"kind":kind})).unwrap();
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["section"], default);
        assert_eq!(value["query"], "");
        assert_eq!(value["offset"], 0);
        for flag in [
            "allowed",
            "approved",
            "allow_agent_context",
            "reveal_exact_amounts_to_agent",
        ] {
            let mut value = serde_json::json!({"kind":kind});
            value[flag] = true.into();
            assert!(serde_json::from_value::<BridgeRequest>(value).is_err());
        }
        for field in [
            serde_json::json!({"section":"unknown"}),
            serde_json::json!({"query":false}),
            serde_json::json!({"offset":-1}),
        ] {
            let mut value = field;
            value["kind"] = kind.into();
            assert!(serde_json::from_value::<BridgeRequest>(value).is_err());
        }
    }
}

#[test]
fn rollback_wire_cannot_carry_execution_or_approval_flags() {
    for flag in ["approved", "execute", "auto_run"] {
        let mut value =
            serde_json::json!({"kind":"request_rollback","workflow_id":"owned","dry_run":false});
        value[flag] = true.into();
        assert!(serde_json::from_value::<BridgeRequest>(value).is_err());
    }
}
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn authenticated_request_reaches_only_the_app_receiver() {
    let (session, mut receiver) = BridgeSession::start().unwrap();
    let client = BridgeClient::from_file(session.connection_path()).unwrap();
    let call = tokio::spawn(async move {
        client
            .call(BridgeRequest::Read {
                domain: AgentDomain::Tasks,
                query: "needle".into(),
                offset: 0,
            })
            .await
    });
    let pending = receiver.recv().await.unwrap();
    assert!(matches!(
        pending.request,
        BridgeRequest::Read {
            domain: AgentDomain::Tasks,
            ..
        }
    ));
    pending
        .respond
        .send(BridgeResponse::Ok(serde_json::json!({"from_app": true})))
        .unwrap();
    assert!(
        matches!(call.await.unwrap().unwrap(), BridgeResponse::Ok(value) if value["from_app"] == true)
    );
}

#[tokio::test]
async fn invalid_token_never_reaches_app() {
    let (session, mut receiver) = BridgeSession::start().unwrap();
    let mut client = BridgeClient::from_file(session.connection_path()).unwrap();
    client.connection.token = "0".repeat(64);
    assert!(matches!(
        client
            .call(BridgeRequest::Read {
                domain: AgentDomain::Tasks,
                query: String::new(),
                offset: 0
            })
            .await
            .unwrap(),
        BridgeResponse::Error(_)
    ));
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn disabling_session_revokes_existing_client_and_in_flight_request() {
    let (session, mut receiver) = BridgeSession::start().unwrap();
    let path = session.connection_path().to_owned();
    let client = BridgeClient::from_file(&path).unwrap();
    let active = client.clone();
    let call = tokio::spawn(async move {
        active
            .call(BridgeRequest::Read {
                domain: AgentDomain::Tasks,
                query: String::new(),
                offset: 0,
            })
            .await
    });
    let pending = receiver.recv().await.unwrap();
    drop(session);
    assert!(call.await.unwrap().is_err());
    assert!(pending.respond.is_closed());
    assert!(!path.exists());
    assert!(client
        .call(BridgeRequest::Read {
            domain: AgentDomain::Tasks,
            query: String::new(),
            offset: 0
        })
        .await
        .is_err());
}

#[tokio::test]
async fn oversized_frame_is_rejected_before_dispatch() {
    use tokio::io::AsyncWriteExt;
    let (session, mut receiver) = BridgeSession::start().unwrap();
    let connection = wire::read_connection(session.connection_path()).unwrap();
    let mut stream = UnixStream::connect(connection.socket).await.unwrap();
    stream
        .write_u32(u32::try_from(MAX_REQUEST_BYTES + 1).unwrap())
        .await
        .unwrap();
    assert!(wire::read_frame(&mut stream, MAX_RESPONSE_BYTES)
        .await
        .is_err());
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn connection_files_are_private_and_public_files_are_rejected() {
    let (session, _) = BridgeSession::start().unwrap();
    assert_eq!(
        std::fs::metadata(&session.directory)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(session.connection_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::set_permissions(
        session.connection_path(),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(BridgeClient::from_file(session.connection_path()).is_err());
}

#[test]
fn unknown_requests_cannot_be_used_to_grant_or_execute() {
    for kind in ["grant", "execute", "approve", "rollback", "set_capability"] {
        assert!(serde_json::from_value::<BridgeRequest>(serde_json::json!({"kind":kind})).is_err());
    }
}
