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
    let mut config = oauth::Config::for_client(
        provider,
        "synthetic-client".into(),
        Some("synthetic-client-secret".into()),
        None,
    )
    .unwrap();
    config.auth = format!("{base}/authorize");
    config.token = format!("{base}/token");
    config.identity = format!("{base}/identity");
    config
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
}

async fn setup(provider: ServiceProvider) -> (Connections, MockServer) {
    // OAuth shares a process-wide HTTP client, while each test has its own runtime.
    // Dedicated origins keep its connection pool separate from recycled mock servers.
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "access_token": "access-canary", "refresh_token": "refresh-canary", "expires_in": 3600, "scope": config(provider, &server.uri()).scopes.join(" "), "token_type": "Bearer" })))
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
        let attempt = manager.attempt(&start.id).unwrap();
        assert_eq!(
            attempt.state,
            AttemptState::Connected,
            "{} authorization failed: {:?}",
            provider.id(),
            attempt.message
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

#[test]
fn mail_connections_have_separate_scopes_and_registration_identity() {
    let client = |provider| {
        oauth::Config::for_client(provider, "same-client".into(), None, Some("common")).unwrap()
    };
    let google = client(ServiceProvider::Google);
    let gmail = client(ServiceProvider::Gmail);
    let microsoft = client(ServiceProvider::Microsoft);
    let outlook = client(ServiceProvider::Outlook);
    assert!(google
        .scopes
        .iter()
        .any(|scope| scope.ends_with("calendar.events.readonly")));
    assert!(!google.scopes.iter().any(|scope| scope.contains("gmail")));
    assert!(microsoft
        .scopes
        .iter()
        .any(|scope| scope.ends_with("Calendars.ReadBasic")));
    assert!(!microsoft.scopes.iter().any(|scope| scope.contains("Mail.")));
    assert!(!microsoft
        .scopes
        .iter()
        .any(|scope| scope.ends_with("User.Read")));
    assert!(oauth::require_mail_permissions(ServiceProvider::Gmail, &gmail.scopes).is_ok());
    assert!(oauth::require_mail_permissions(ServiceProvider::Outlook, &outlook.scopes).is_ok());
    assert_ne!(google.registration(), gmail.registration());
    assert_ne!(microsoft.registration(), outlook.registration());
    assert_ne!(ServiceProvider::Google.id(), ServiceProvider::Gmail.id());
    assert_ne!(
        ServiceProvider::Microsoft.id(),
        ServiceProvider::Outlook.id()
    );
}

#[test]
fn outlook_requests_and_requires_its_own_profile_read_permission() {
    let config = oauth::Config::for_client(
        ServiceProvider::Outlook,
        "same-client".into(),
        None,
        Some("common"),
    )
    .unwrap();
    let mut old = config.clone();
    old.scopes.retain(|scope| !scope.ends_with("User.Read"));
    assert_eq!(old.registration(), config.registration());
    let url = url::Url::parse(
        &config
            .auth_url("http://127.0.0.1/callback", "state", "synthetic-verifier")
            .unwrap(),
    )
    .unwrap();
    let scopes = url.query_pairs().find(|(key, _)| key == "scope").unwrap().1;
    assert!(scopes
        .split_whitespace()
        .any(|scope| scope == "https://graph.microsoft.com/User.Read"));
    let mail_only: Vec<String> = ["openid", "profile", "email", "Mail.ReadWrite", "Mail.Send"]
        .map(String::from)
        .into();
    assert!(matches!(
        oauth::require_mail_permissions(ServiceProvider::Outlook, &mail_only),
        Err(ConnectionError::MissingMailPermissions)
    ));
    for profile_scope in ["User.Read", "https://graph.microsoft.com/User.Read"] {
        let mut complete = mail_only.clone();
        complete.push(profile_scope.into());
        assert!(oauth::require_mail_permissions(ServiceProvider::Outlook, &complete).is_ok());
    }
}

#[tokio::test]
async fn outlook_mail_grant_without_profile_read_cannot_complete_authorization() {
    let provider = ServiceProvider::Outlook;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token":"mail-only-canary", "refresh_token":"mail-only-refresh", "expires_in":3600,
            "scope":"openid profile email Mail.ReadWrite Mail.Send", "token_type":"Bearer"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/identity"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"sub":"stable-subject","email":"you@example.test"}),
            ),
        )
        .mount(&server)
        .await;
    let manager = manager(provider, &server.uri());
    let start = manager.start(provider).await.unwrap();
    reqwest::get(callback(&start)).await.unwrap();
    assert_eq!(
        manager.attempt(&start.id).unwrap().state,
        AttemptState::Failed
    );
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::Disconnected
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
async fn renewing_outlook_profile_consent_keeps_mailbox_identity_and_saved_drafts() {
    let provider = ServiceProvider::Outlook;
    let (manager, _server) = setup(provider).await;
    let current = manager.0.configs[&provider].clone();
    let mut legacy = current.clone();
    legacy.scopes.retain(|scope| !scope.ends_with("User.Read"));
    assert_eq!(legacy.registration(), current.registration());
    let legacy_mailbox = crate::mail::MailboxId::from_broker_identity(
        provider,
        &legacy.registration(),
        "stable-subject",
    )
    .unwrap();
    let start = manager.start(provider).await.unwrap();
    reqwest::get(callback(&start)).await.unwrap();
    let connection = manager.connection(provider).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let runtime = crate::mail::MailRuntime::open(directory.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let session = runtime
        .session(&manager, provider, connection.id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(session.authorization().mailbox(), &legacy_mailbox);
    let draft = legacy_mailbox
        .draft("retained-across-profile-consent", None)
        .unwrap();
    let payload = serde_json::json!({"body":"Retain this mailbox draft"}).to_string();
    session
        .save_draft(draft.clone(), 0, payload.clone())
        .await
        .unwrap();
    drop(session);
    let old_id = {
        let lease = manager.0.store.lease(provider).await.unwrap();
        let mut tokens = lease.read().await.unwrap().unwrap();
        tokens.scopes = legacy.scopes.clone();
        let old_id = tokens.id.clone();
        lease.write(&tokens).await.unwrap();
        drop(lease);
        old_id
    };
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::NeedsSignIn
    );
    assert!(matches!(
        manager.access(provider, &old_id).await,
        Err(ConnectionError::MissingMailPermissions)
    ));
    assert_eq!(
        runtime
            .cached_draft(draft.clone())
            .await
            .unwrap()
            .unwrap()
            .1,
        payload
    );
    let renewed = manager.start(provider).await.unwrap();
    let auth = url::Url::parse(&renewed.auth_url).unwrap();
    assert!(auth.query_pairs().any(|(key, value)| key == "scope"
        && value
            .split_whitespace()
            .any(|scope| scope == "https://graph.microsoft.com/User.Read")));
    reqwest::get(callback(&renewed)).await.unwrap();
    assert_eq!(
        manager.attempt(&renewed.id).unwrap().state,
        AttemptState::Connected
    );
    let connection = manager.connection(provider).await.unwrap();
    assert_eq!(connection.state, ConnectionState::Connected);
    let session = runtime
        .session(&manager, provider, connection.id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(session.authorization().mailbox(), &legacy_mailbox);
    assert_eq!(
        runtime.cached_draft(draft).await.unwrap().unwrap().1,
        payload
    );
    drop(session);
}

#[tokio::test]
async fn refreshed_outlook_grant_without_profile_read_keeps_rotation_but_refuses_access() {
    let provider = ServiceProvider::Outlook;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token":"mail-only-canary", "refresh_token":"rotated-mail-only-canary", "expires_in":3600,
            "scope":"openid profile email Mail.ReadWrite Mail.Send", "token_type":"Bearer"
        })))
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
            id: "mail-connection".into(),
            access_token: "expired-canary".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at_ms: Some(1),
            account_id: "stable-subject".into(),
            account: "you@example.test".into(),
            registration: manager.0.configs[&provider].registration(),
            scopes: manager.0.configs[&provider].scopes.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        manager.access(provider, "mail-connection").await,
        Err(ConnectionError::MissingMailPermissions)
    ));
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::NeedsSignIn
    );
    let lease = manager.0.store.lease(provider).await.unwrap();
    let saved = lease.read().await.unwrap().unwrap();
    drop(lease);
    assert_eq!(
        saved.refresh_token.as_deref(),
        Some("rotated-mail-only-canary")
    );
    assert_eq!(saved.access_token, "mail-only-canary");
    assert!(saved
        .scopes
        .iter()
        .all(|scope| !scope.ends_with("User.Read")));
}

#[tokio::test]
async fn missing_mail_permissions_cannot_complete_authorization() {
    for provider in [ServiceProvider::Gmail, ServiceProvider::Outlook] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST")).and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token":"partial-canary", "refresh_token":"partial-refresh", "expires_in":3600,
                "scope":"openid email", "token_type":"Bearer"
            }))).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/identity"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"sub":"stable", "email":"mail@example.test"}),
                ),
            )
            .mount(&server)
            .await;
        let manager = manager(provider, &server.uri());
        let started = manager.start(provider).await.unwrap();
        reqwest::get(callback(&started)).await.unwrap();
        assert_eq!(
            manager.attempt(&started.id).unwrap().state,
            AttemptState::Failed
        );
        assert_eq!(
            manager.connection(provider).await.unwrap().state,
            ConnectionState::Disconnected
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
}

#[tokio::test]
async fn refreshing_a_narrowed_mail_grant_saves_rotation_but_refuses_access() {
    let provider = ServiceProvider::Outlook;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token":"narrow-canary", "refresh_token":"rotated-canary", "expires_in":3600,
            "scope":"openid email Mail.ReadWrite", "token_type":"Bearer"
        })))
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
            id: "mail-connection".into(),
            access_token: "expired-canary".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at_ms: Some(1),
            account_id: "stable".into(),
            account: "mail@example.test".into(),
            registration: manager.0.configs[&provider].registration(),
            scopes: manager.0.configs[&provider].scopes.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        manager.access(provider, "mail-connection").await,
        Err(ConnectionError::MissingMailPermissions)
    ));
    assert_eq!(
        manager.connection(provider).await.unwrap().state,
        ConnectionState::NeedsSignIn
    );
    let lease = manager.0.store.lease(provider).await.unwrap();
    let saved = lease.read().await.unwrap().unwrap();
    drop(lease);
    assert_eq!(saved.refresh_token.as_deref(), Some("rotated-canary"));
    assert_eq!(saved.access_token, "narrow-canary");
}

#[tokio::test]
async fn disconnecting_mail_leaves_the_calendar_connection_and_token_intact() {
    let configs = [ServiceProvider::Google, ServiceProvider::Gmail]
        .map(|provider| (provider, config(provider, "http://127.0.0.1")))
        .into();
    let manager = Connections(Arc::new(Inner {
        configs,
        store: storage::Store::memory(),
        attempts: Mutex::new(HashMap::new()),
    }));
    for provider in [ServiceProvider::Google, ServiceProvider::Gmail] {
        manager
            .0
            .store
            .lease(provider)
            .await
            .unwrap()
            .write(&Tokens {
                id: provider.id().into(),
                access_token: format!("{}-canary", provider.id()),
                refresh_token: None,
                expires_at_ms: None,
                account_id: "stable".into(),
                account: "same@example.test".into(),
                registration: manager.0.configs[&provider].registration(),
                scopes: manager.0.configs[&provider].scopes.clone(),
            })
            .await
            .unwrap();
    }
    manager.disconnect(ServiceProvider::Gmail).await.unwrap();
    assert_eq!(
        manager
            .connection(ServiceProvider::Gmail)
            .await
            .unwrap()
            .state,
        ConnectionState::Disconnected
    );
    assert_eq!(
        manager
            .connection(ServiceProvider::Google)
            .await
            .unwrap()
            .state,
        ConnectionState::Connected
    );
    let access = manager
        .access(ServiceProvider::Google, "google")
        .await
        .unwrap();
    assert_eq!(access.value.as_str(), "google-canary");
    drop(access);
}

#[test]
fn mail_client_secrets_are_removed_from_agent_commands() {
    let mut command = tokio::process::Command::new("unused-fixture");
    for key in [
        "BRUHS_CONNECTION_GMAIL_CLIENT_SECRET",
        "BRUHS_CONNECTION_OUTLOOK_CLIENT_SECRET",
    ] {
        command.env(key, "synthetic-secret");
    }
    protect_command(&mut command);
    for key in [
        "BRUHS_CONNECTION_GMAIL_CLIENT_SECRET",
        "BRUHS_CONNECTION_OUTLOOK_CLIENT_SECRET",
    ] {
        assert!(command
            .as_std()
            .get_envs()
            .any(|(name, value)| name == key && value.is_none()));
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
    let server = MockServer::builder().start().await;
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
    let server = MockServer::builder().start().await;
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
    let server = MockServer::builder().start().await;
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
