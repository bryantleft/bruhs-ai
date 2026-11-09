use super::*;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

#[tokio::test]
async fn another_hosts_disconnect_fences_a_pending_callback() {
    let provider = ServiceProvider::Google;
    let (manager, _server) = setup(provider).await;
    let other = Connections(Arc::new(Inner {
        configs: manager.0.configs.clone(),
        store: manager.0.store.clone(),
        attempts: Mutex::new(HashMap::new()),
    }));
    let started = manager.start(provider).await.unwrap();
    other.disconnect(provider).await.unwrap();
    reqwest::get(callback(&started)).await.unwrap();
    assert_eq!(
        manager.attempt(&started.id).unwrap().state,
        AttemptState::Failed
    );
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::Disconnected
    );
}

fn config(provider: ServiceProvider, base: &str) -> oauth::Config {
    oauth::Config {
        provider,
        client_id: "synthetic-client".into(),
        client_secret: Some("synthetic-client-secret".into()),
        auth: format!("{base}/authorize"),
        token: format!("{base}/token"),
        identity: format!("{base}/identity"),
        scopes: vec!["profile.read".into()],
    }
}

fn manager(provider: ServiceProvider, base: &str) -> Connections {
    Connections(Arc::new(Inner {
        configs: [(provider, config(provider, base))].into(),
        store: storage::Store::memory(),
        attempts: Mutex::new(HashMap::new()),
    }))
}

impl Connections {
    pub(crate) async fn test_missing_tokens(provider: ServiceProvider) -> Self {
        let manager = Self::test_connected(provider, "synthetic-lost-token").await;
        let metadata = manager.0.store.metadata(provider).await.unwrap().unwrap();
        manager.0.store.metadata_only(provider, metadata);
        manager
    }

    pub(crate) async fn test_connected(provider: ServiceProvider, value: &str) -> Self {
        let manager = manager(provider, "http://127.0.0.1");
        manager
            .0
            .store
            .lease(provider)
            .await
            .unwrap()
            .write(&Tokens {
                id: "synthetic-connection".into(),
                access_token: value.into(),
                refresh_token: Some("synthetic-refresh".into()),
                expires_at_ms: None,
                account_id: "stable-subject".into(),
                account: "you@example.test".into(),
                scopes: vec!["profile.read".into()],
                registration: manager.0.configs[&provider].registration(),
            })
            .await
            .unwrap();
        manager
    }
}

async fn setup(provider: ServiceProvider) -> (Connections, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "access_token": "access-canary", "refresh_token": "refresh-canary", "expires_in": 3600, "scope": "profile.read", "token_type": "Bearer" })))
        .mount(&server).await;
    let identity = match provider {
        ServiceProvider::Github => serde_json::json!({"id": 42, "login": "synthetic-user"}),
        _ => serde_json::json!({"sub": "stable-subject", "email": "you@example.test"}),
    };
    Mock::given(method("GET"))
        .and(path("/identity"))
        .respond_with(ResponseTemplate::new(200).set_body_json(identity))
        .mount(&server)
        .await;
    (manager(provider, &server.uri()), server)
}

fn callback(start: &ConnectionStart) -> url::Url {
    let auth = url::Url::parse(&start.auth_url).unwrap();
    let params: HashMap<_, _> = auth.query_pairs().into_owned().collect();
    assert_eq!(params["code_challenge_method"], "S256");
    let mut callback = url::Url::parse(&params["redirect_uri"]).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("state", &params["state"])
        .append_pair("code", "synthetic-code");
    callback
}

#[tokio::test]
async fn all_services_complete_browser_authorization_without_leaking_tokens() {
    for provider in ServiceProvider::ALL {
        let (manager, _server) = setup(provider).await;
        let start = manager.start(provider).await.unwrap();
        assert!(!url::Url::parse(&start.auth_url)
            .unwrap()
            .query_pairs()
            .any(|(key, _)| key == "prompt"));
        let response = reqwest::get(callback(&start)).await.unwrap();
        assert!(response.status().is_success());
        let page = response.text().await.unwrap();
        assert!(!page.contains("canary"));
        assert_eq!(
            manager.attempt(&start.id).unwrap().state,
            AttemptState::Connected
        );
        let rows = manager.list().await.unwrap();
        let row = rows.iter().find(|row| row.provider == provider).unwrap();
        assert_eq!(row.state, ConnectionState::Connected);
        let public = serde_json::to_string(&rows).unwrap();
        assert!(!public.contains("access-canary"));
        assert!(!public.contains("refresh-canary"));
        let access = manager
            .access(provider, row.id.as_deref().unwrap())
            .await
            .unwrap();
        assert_eq!(access.value.as_str(), "access-canary");
        drop(access);
    }
}

#[tokio::test]
async fn incorrect_callback_state_cannot_consume_the_valid_attempt() {
    let (manager, _server) = setup(ServiceProvider::Google).await;
    let start = manager.start(ServiceProvider::Google).await.unwrap();
    let mut wrong = callback(&start);
    wrong.set_query(Some("state=wrong&code=private-code-canary"));
    let response = reqwest::get(wrong).await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    assert!(!response
        .text()
        .await
        .unwrap()
        .contains("private-code-canary"));
    assert_eq!(
        manager.attempt(&start.id).unwrap().state,
        AttemptState::Pending
    );
    assert!(reqwest::get(callback(&start))
        .await
        .unwrap()
        .status()
        .is_success());
    assert_eq!(
        manager.attempt(&start.id).unwrap().state,
        AttemptState::Connected
    );
}

#[tokio::test]
async fn cancellation_and_turn_teardown_withdraw_authorization() {
    let (manager, _server) = setup(ServiceProvider::Microsoft).await;
    let (cancel, receiver) = watch::channel(false);
    let start = manager
        .start_scoped(ServiceProvider::Microsoft, Some(receiver))
        .await
        .unwrap();
    cancel.send_replace(true);
    // Let the listener's cancellation path finish rather than attempting a real sign-in.
    tokio::task::yield_now().await;
    manager.cancel(&start.id);
    assert_eq!(
        manager.attempt(&start.id).unwrap().state,
        AttemptState::Cancelled
    );
    assert!(manager
        .0
        .store
        .metadata(ServiceProvider::Microsoft)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn disconnect_invalidates_old_connection_references() {
    let provider = ServiceProvider::Github;
    let manager = Connections::test_connected(provider, "private-value").await;
    let old = manager.connection(provider).await.unwrap().id.unwrap();
    manager.disconnect(provider).await.unwrap();
    assert!(matches!(
        manager.access(provider, &old).await,
        Err(ConnectionError::SignIn)
    ));
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::Disconnected
    );
}

#[tokio::test]
async fn refresh_rotation_is_serialized_and_persists_new_credentials() {
    let provider = ServiceProvider::Github;
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"new-access", "refresh_token":"new-refresh", "expires_in":3600}))).expect(1).mount(&server).await;
    let manager = manager(provider, &server.uri());
    let lease = manager.0.store.lease(provider).await.unwrap();
    lease
        .write(&Tokens {
            id: "connection".into(),
            access_token: "old-access".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at_ms: Some(0),
            account_id: "subject".into(),
            account: "user".into(),
            scopes: vec![],
            registration: manager.0.configs[&provider].registration(),
        })
        .await
        .unwrap();
    drop(lease);
    let first = async {
        let access = manager.access(provider, "connection").await.unwrap();
        assert_eq!(access.value.as_str(), "new-access");
        drop(access);
    };
    let second = async {
        let access = manager.access(provider, "connection").await.unwrap();
        assert_eq!(access.value.as_str(), "new-access");
        drop(access);
    };
    tokio::join!(first, second);
    let saved = manager
        .0
        .store
        .lease(provider)
        .await
        .unwrap()
        .read()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.refresh_token.as_deref(), Some("new-refresh"));
    server.verify().await;
}

#[tokio::test]
async fn provider_errors_never_echo_response_bodies_or_tokens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            serde_json::json!({"error":"invalid_grant", "error_description":"private-canary"}),
        ))
        .mount(&server)
        .await;
    let cfg = config(ServiceProvider::Google, &server.uri());
    let error = oauth::exchange(
        &cfg,
        "private-code",
        "http://127.0.0.1/callback",
        "verifier",
        None,
    )
    .await
    .err()
    .unwrap();
    assert!(matches!(error, ConnectionError::SignIn));
    assert!(!error.to_string().contains("private"));
}

#[tokio::test]
async fn reconnect_recovers_metadata_without_a_keychain_value() {
    let provider = ServiceProvider::Google;
    let (manager, _server) = setup(provider).await;
    manager.0.store.metadata_only(
        provider,
        storage::Metadata {
            id: "incomplete-save".into(),
            account: "you@example.test".into(),
            account_id: Some("stable-subject".into()),
            scopes: vec![],
            state: ConnectionState::Unavailable,
            registration: manager.0.configs[&provider].registration(),
            expires_at_ms: None,
            refresh_available: false,
        },
    );
    let started = manager.start(provider).await.unwrap();
    reqwest::get(callback(&started)).await.unwrap();
    assert_eq!(
        manager.attempt(&started.id).unwrap().state,
        AttemptState::Connected
    );
}

#[tokio::test]
async fn reconnect_cannot_change_a_lost_accounts_stable_identity_even_with_the_same_email() {
    let provider = ServiceProvider::Google;
    let (manager, _server) = setup(provider).await;
    manager.0.store.metadata_only(
        provider,
        storage::Metadata {
            id: "lost-connection".into(),
            account: "you@example.test".into(),
            account_id: Some("different-stable-subject".into()),
            scopes: vec![],
            state: ConnectionState::Unavailable,
            registration: manager.0.configs[&provider].registration(),
            expires_at_ms: None,
            refresh_available: false,
        },
    );
    let start = manager.start(provider).await.unwrap();
    reqwest::get(callback(&start)).await.unwrap();
    let attempt = manager.attempt(&start.id).unwrap();
    assert_eq!(attempt.state, AttemptState::Failed);
    assert_eq!(
        attempt.message.as_deref(),
        Some(ConnectionError::AccountMismatch.to_string().as_str())
    );
    assert!(manager
        .0
        .store
        .lease(provider)
        .await
        .unwrap()
        .read()
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn revoked_github_refresh_requires_reauthentication() {
    let provider = ServiceProvider::Github;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"error":"bad_refresh_token", "error_description":"private-canary"}),
        ))
        .mount(&server)
        .await;
    let manager = manager(provider, &server.uri());
    manager
        .0
        .store
        .lease(provider)
        .await
        .unwrap()
        .write(&Tokens {
            id: "expired".into(),
            access_token: "old".into(),
            refresh_token: Some("refresh".into()),
            expires_at_ms: Some(0),
            account_id: "subject".into(),
            account: "user".into(),
            scopes: vec![],
            registration: manager.0.configs[&provider].registration(),
        })
        .await
        .unwrap();
    assert!(matches!(
        manager.access(provider, "expired").await,
        Err(ConnectionError::SignIn)
    ));
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::NeedsSignIn
    );
}

#[tokio::test]
async fn approved_commands_do_not_inherit_any_service_client_secret() {
    let mut command = tokio::process::Command::new("/bin/sh");
    let checks = PRIVATE_ENV
        .iter()
        .map(|key| format!("test -z \"${{{key}+set}}\""))
        .collect::<Vec<_>>()
        .join(" && ");
    command.args(["-c", &checks]);
    for key in PRIVATE_ENV {
        command.env(key, "synthetic-client-secret-canary");
    }
    protect_command(&mut command);
    let output = command.output().await.unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}
