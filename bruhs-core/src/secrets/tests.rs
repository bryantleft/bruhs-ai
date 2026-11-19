use super::*;

const PRIVATE: &str = "synthetic-credential-canary";

#[tokio::test]
async fn a_missing_account_token_keeps_the_operation_pending_for_reconnect() {
    use bruhs_models::connections::{ConnectionState, ServiceProvider};
    let provider = ServiceProvider::Google;
    let connections = crate::connections::Connections::test_missing_tokens(provider).await;
    let mut fixture = Fixture::with_manager(
        SecretManager::new(SecretStore::in_memory()).with_connections(connections),
    )
    .await;
    let scope = fixture.scope.clone();
    let call = tokio::spawn(async move {
        scope
            .request_from(
                SecretOperation {
                    name: "Google".into(),
                    reason: "Read calendar".into(),
                    env_var: "TOKEN".into(),
                    command: vec!["/usr/bin/true".into()],
                },
                Some(provider),
            )
            .await
    });
    let request = fixture.request().await;
    let id = request.connection.unwrap().id.unwrap();
    assert!(fixture
        .manager
        .resolve(
            &request.thread_id,
            &request.id,
            SecretDecision::UseConnection { id }
        )
        .await
        .is_err());
    assert!(!call.is_finished());
    assert_eq!(
        fixture
            .manager
            .connections()
            .connection(provider)
            .await
            .unwrap()
            .state,
        ConnectionState::NeedsSignIn
    );
    fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::Decline)
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().status, "declined");
}

#[tokio::test]
async fn withdrawing_one_tool_call_cancels_its_login_without_ending_the_turn() {
    use bruhs_models::connections::{AttemptState, ServiceProvider};
    let provider = ServiceProvider::Google;
    let connections = crate::connections::Connections::test_connected(provider, PRIVATE).await;
    let mut fixture = Fixture::with_manager(
        SecretManager::new(SecretStore::in_memory()).with_connections(connections),
    )
    .await;
    let scope = fixture.scope.clone();
    let call = tokio::spawn(async move {
        scope
            .request_from(
                SecretOperation {
                    name: "Google".into(),
                    reason: "Read calendar".into(),
                    env_var: "TOKEN".into(),
                    command: vec!["/usr/bin/true".into()],
                },
                Some(provider),
            )
            .await
    });
    let request = fixture.request().await;
    let login = fixture
        .manager
        .connect_request(&request.thread_id, &request.id, provider)
        .await
        .unwrap();
    call.abort();
    let _ = call.await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while fixture
            .manager
            .connections()
            .attempt(&login.id)
            .unwrap()
            .state
            != AttemptState::Cancelled
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(fixture.scope.active.load(Ordering::Acquire));
    assert!(fixture
        .manager
        .connections()
        .connection(provider)
        .await
        .unwrap()
        .id
        .is_some());
}

#[tokio::test]
async fn a_connected_account_still_requires_approval_and_never_returns_token_output() {
    use bruhs_models::connections::ServiceProvider;
    let provider = ServiceProvider::Github;
    let connections = crate::connections::Connections::test_connected(provider, PRIVATE).await;
    let mut fixture = Fixture::with_manager(
        SecretManager::new(SecretStore::in_memory()).with_connections(connections),
    )
    .await;
    let scope = fixture.scope.clone();
    let call = tokio::spawn(async move {
        scope.request_from(SecretOperation {
        name: "GitHub".into(), reason: "Verify private account use".into(), env_var: "SERVICE_TOKEN".into(),
        command: vec!["/bin/sh".into(), "-c".into(), "printf '%s' \"$SERVICE_TOKEN\"; printf '%s' \"$SERVICE_TOKEN\" >&2; test -n \"$SERVICE_TOKEN\"".into()],
    }, Some(provider)).await
    });
    let request = fixture.request().await;
    let public = serde_json::to_string(&request).unwrap();
    assert!(!public.contains(PRIVATE));
    assert!(
        !call.is_finished(),
        "A saved account cannot auto-approve a command"
    );
    assert!(fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::UseSaved)
        .await
        .is_err());
    let id = request.connection.as_ref().unwrap().id.clone().unwrap();
    fixture
        .manager
        .resolve(
            &request.thread_id,
            &request.id,
            SecretDecision::UseConnection { id },
        )
        .await
        .unwrap();
    let result = call.await.unwrap();
    assert_eq!(result.exit_code, Some(0));
    assert!(!serde_json::to_string(&result).unwrap().contains(PRIVATE));
}

#[tokio::test]
async fn stale_account_approval_cannot_run_after_disconnect() {
    use bruhs_models::connections::ServiceProvider;
    let provider = ServiceProvider::Google;
    let connections = crate::connections::Connections::test_connected(provider, PRIVATE).await;
    let mut fixture = Fixture::with_manager(
        SecretManager::new(SecretStore::in_memory()).with_connections(connections),
    )
    .await;
    let scope = fixture.scope.clone();
    let call = tokio::spawn(async move {
        scope
            .request_from(
                SecretOperation {
                    name: "Google".into(),
                    reason: "Read calendar".into(),
                    env_var: "TOKEN".into(),
                    command: vec!["/usr/bin/true".into()],
                },
                Some(provider),
            )
            .await
    });
    let request = fixture.request().await;
    let id = request.connection.unwrap().id.unwrap();
    fixture
        .manager
        .connections()
        .disconnect(provider)
        .await
        .unwrap();
    assert!(fixture
        .manager
        .resolve(
            &request.thread_id,
            &request.id,
            SecretDecision::UseConnection { id }
        )
        .await
        .is_err());
    assert!(!call.is_finished());
    fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::Decline)
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().status, "declined");
}

#[tokio::test]
async fn management_does_not_wait_for_an_unread_turn_stream() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    fixture.provide(&request, true).await.unwrap();
    call.await.unwrap().unwrap();
    let id = fixture.manager.vault_entries().await.unwrap()[0].id.clone();
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    while fixture
        ._sender
        .try_send(OrchestratorEvent::Query(QueryEvent::Text("queued".into())))
        .is_ok()
    {}
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.manager.vault_delete(&id),
    )
    .await;
    assert!(
        result.is_ok(),
        "Settings must not wait for a paused thread's event stream"
    );
    result.unwrap().unwrap();
    assert!(!call.is_finished());
    while fixture.events.try_recv().is_ok() {}
    fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::Decline)
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap().status, "declined");
}

#[tokio::test]
async fn management_updates_pending_metadata_without_approving_any_command() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    fixture.provide(&request, true).await.unwrap();
    call.await.unwrap().unwrap();
    let id = fixture.manager.vault_entries().await.unwrap()[0].id.clone();
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    fixture
        .manager
        .vault_update(
            &id,
            VaultUpdate {
                secret: bruhs_models::vault::VaultSecretInput::Value {
                    value: Some("private-rotated".into()),
                },
            },
        )
        .await
        .unwrap();
    assert!(fixture.request().await.saved_available);
    assert!(!call.is_finished());
    fixture.manager.vault_delete(&id).await.unwrap();
    assert!(!fixture.request().await.saved_available);
    assert!(!call.is_finished());
    fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::Decline)
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap().status, "declined");
}

struct Fixture {
    manager: SecretManager,
    scope: Arc<SecretScope>,
    events: mpsc::Receiver<OrchestratorEvent>,
    broker: SecretBroker,
    _sender: mpsc::Sender<OrchestratorEvent>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_store(SecretStore::in_memory()).await
    }

    async fn with_store(store: SecretStore) -> Self {
        Self::with_manager(
            SecretManager::new(store)
                .with_connections(crate::connections::Connections::in_memory()),
        )
        .await
    }

    async fn with_manager(manager: SecretManager) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let (tx, events) = mpsc::channel(32);
        let scope = manager.turn(
            ThreadId::new_v4(),
            directory.path().into(),
            tx.clone(),
            false,
        );
        let broker = scope.start().await.unwrap();
        Self {
            manager,
            scope,
            events,
            broker,
            _sender: tx,
            _directory: directory,
        }
    }

    fn call(&self, script: &str) -> JoinHandle<std::io::Result<OperationResult>> {
        let connection = self.broker.connection.clone();
        let operation = SecretOperation {
            name: "Service key".into(),
            reason: "Run a test operation".into(),
            env_var: "TEST_CREDENTIAL".into(),
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        };
        tokio::spawn(async move { mcp::call(&connection, operation).await })
    }

    async fn request(&mut self) -> SecretRequest {
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(5), self.events.recv())
                .await
                .unwrap()
                .unwrap();
            if let OrchestratorEvent::Query(QueryEvent::SecretRequest(request)) = event {
                return request;
            }
        }
    }

    async fn provide(&self, request: &SecretRequest, save: bool) -> Result<bool, SecretError> {
        self.manager
            .resolve(
                &request.thread_id,
                &request.id,
                SecretDecision::Provide {
                    value: PRIVATE.into(),
                    save,
                },
            )
            .await
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.scope.revoke();
    }
}

#[tokio::test]
async fn credential_runs_only_the_reviewed_operation_and_output_never_reaches_the_model() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("test -n \"$TEST_CREDENTIAL\" || exit 9; printf '%s' \"$TEST_CREDENTIAL\"; printf '%s' \"$TEST_CREDENTIAL\" >&2; exit 7");
    let request = fixture.request().await;
    assert!(!call.is_finished(), "operation waits for private input");
    assert!(fixture.provide(&request, false).await.unwrap());
    let result = call.await.unwrap().unwrap();
    assert_eq!(result.exit_code, Some(7));
    assert_eq!(result.status, "completed");
    let wire = serde_json::to_string(&result).unwrap();
    assert!(!wire.contains(PRIVATE));
    assert!(!wire.contains("stdout"));
    assert!(!serde_json::to_string(&request).unwrap().contains(PRIVATE));
    assert!(fixture
        .manager
        .store
        .get(&storage_key(fixture._directory.path(), "Service key"))
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn saved_credentials_require_approval_for_every_use_and_can_be_forgotten() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("test -n \"$TEST_CREDENTIAL\"");
    let request = fixture.request().await;
    fixture.provide(&request, true).await.unwrap();
    assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
    let call = fixture.call("test -n \"$TEST_CREDENTIAL\"");
    let request = fixture.request().await;
    assert!(request.saved_available);
    assert!(!call.is_finished());
    fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::UseSaved)
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    assert!(!fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::ForgetSaved)
        .await
        .unwrap());
    assert!(!fixture.request().await.saved_available);
    assert!(!call.is_finished(), "forget must not approve a command");
    assert!(matches!(
        fixture
            .manager
            .resolve(&request.thread_id, &request.id, SecretDecision::UseSaved)
            .await,
        Err(SecretError::NotSaved)
    ));
    fixture
        .manager
        .resolve(&request.thread_id, &request.id, SecretDecision::Decline)
        .await
        .unwrap();
    assert_eq!(call.await.unwrap().unwrap().status, "declined");
}

#[tokio::test]
async fn wrong_thread_empty_value_and_replay_cannot_execute_the_operation() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    assert!(matches!(
        fixture
            .manager
            .resolve(&ThreadId::new_v4(), &request.id, SecretDecision::Decline)
            .await,
        Err(SecretError::NotPending)
    ));
    assert!(matches!(
        fixture
            .manager
            .resolve(
                &request.thread_id,
                &request.id,
                SecretDecision::Provide {
                    value: String::new(),
                    save: false
                }
            )
            .await,
        Err(SecretError::Invalid(_))
    ));
    assert!(!call.is_finished());
    fixture.provide(&request, false).await.unwrap();
    assert!(matches!(
        fixture.provide(&request, false).await,
        Err(SecretError::NotPending)
    ));
    assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
}

#[tokio::test]
async fn turn_teardown_revokes_pending_forms_and_broker_connections() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    drop(fixture.scope.guard());
    assert!(matches!(
        fixture.provide(&request, false).await,
        Err(SecretError::NotPending)
    ));
    let result = call.await.unwrap();
    assert!(result.is_err() || result.unwrap().status == "cancelled");
    assert!(fixture.manager.lock().is_empty());
}

#[tokio::test]
async fn disconnected_model_client_withdraws_the_private_form() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    call.abort();
    let _ = call.await;
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), fixture.events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event, OrchestratorEvent::Query(QueryEvent::SecretResolved { ref id }) if id == &request.id)
        {
            break;
        }
    }
    assert!(matches!(
        fixture.provide(&request, false).await,
        Err(SecretError::NotPending)
    ));
}

#[test]
fn storage_is_scoped_to_project_and_credential_name() {
    assert_ne!(
        storage_key(std::path::Path::new("/a"), "key"),
        storage_key(std::path::Path::new("/b"), "key")
    );
    assert_ne!(
        storage_key(std::path::Path::new("/a"), "key"),
        storage_key(std::path::Path::new("/a"), "other")
    );
}

#[tokio::test]
async fn storage_failure_keeps_the_request_retryable_and_use_once_still_works() {
    let mut fixture = Fixture::with_store(SecretStore::unavailable()).await;
    let call = fixture.call("test -n \"$TEST_CREDENTIAL\"");
    let request = fixture.request().await;
    assert!(request.storage_unavailable);
    assert!(matches!(
        fixture.provide(&request, true).await,
        Err(SecretError::Storage)
    ));
    assert!(!call.is_finished());
    fixture.provide(&request, false).await.unwrap();
    assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
}

#[tokio::test]
async fn secret_scope_does_not_keep_a_finished_turns_event_stream_alive() {
    let manager = SecretManager::new(SecretStore::in_memory());
    let (tx, mut rx) = mpsc::channel(1);
    let scope = manager.turn(ThreadId::new_v4(), PathBuf::from("/tmp"), tx, false);
    let _guard = scope.guard();
    assert!(rx.recv().await.is_none());
}

#[cfg(unix)]
#[tokio::test]
async fn turn_cancellation_terminates_a_running_credential_command() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("echo $$ > child-pid; sleep 600 & wait");
    let request = fixture.request().await;
    fixture.provide(&request, false).await.unwrap();
    let pid_file = fixture._directory.path().join("child-pid");
    let pid: i32 = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(value) = tokio::fs::read_to_string(&pid_file).await {
                if let Ok(pid) = value.trim().parse::<i32>() {
                    break pid;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(fixture.scope.guard());
    let _ = call.await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            // SAFETY: signal 0 probes only the owned child's PID and changes no process state.
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// Thread requests matched against the vault (D17): shared keys, 1Password
/// references, the match order, and refreshes after vault changes. Every
/// value is a random canary; 1Password is a stand-in `op`.
#[cfg(unix)]
mod vault_matching {
    use super::*;
    use bruhs_models::vault::{
        CredentialScope, LoginInput, TotpInput, VaultCreate, VaultSecretInput,
    };
    use std::{os::unix::fs::PermissionsExt as _, path::Path};

    fn canary(label: &str) -> String {
        format!("{label}-{}", uuid::Uuid::new_v4().simple())
    }

    fn entry(name: &str, scope: CredentialScope, secret: VaultSecretInput) -> VaultCreate {
        VaultCreate {
            name: name.into(),
            scope,
            secret,
        }
    }

    fn value(value: &str) -> VaultSecretInput {
        VaultSecretInput::Value {
            value: Some(value.into()),
        }
    }

    fn reference() -> VaultSecretInput {
        VaultSecretInput::OnePassword {
            reference: "op://P/Key/value".into(),
        }
    }

    /// Answers `op://P/Key/value` with `folder/value` while
    /// `folder/signed-in` exists.
    fn stand_in(folder: &Path) -> std::path::PathBuf {
        let path = folder.join("op");
        let dir = folder.display();
        std::fs::write(
            &path,
            format!(
                r#"#!/bin/sh
[ -e '{dir}/signed-in' ] || {{ echo '[ERROR] account is not signed in' >&2; exit 1; }}
case "$4" in
  op://P/Key/value) /bin/cat '{dir}/value' ;;
  *) echo "[ERROR] isn't an item" >&2; exit 1 ;;
esac
"#
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A fixture whose manager reads 1Password through a stand-in in `op`.
    async fn fixture(op: &Path) -> Fixture {
        Fixture::with_manager(
            SecretManager::new(SecretStore::in_memory())
                .with_connections(crate::connections::Connections::in_memory())
                .with_one_password(OnePassword::at(
                    stand_in(op),
                    std::time::Duration::from_secs(10),
                )),
        )
        .await
    }

    impl Fixture {
        /// A request for `name` whose command writes `$name` to `out` in
        /// the project folder.
        fn call_for(&self, name: &str) -> JoinHandle<std::io::Result<OperationResult>> {
            let connection = self.broker.connection.clone();
            let operation = SecretOperation {
                name: name.into(),
                reason: "Run a test operation".into(),
                env_var: name.into(),
                command: vec![
                    "/bin/sh".into(),
                    "-c".into(),
                    format!("printf '%s' \"${name}\" > out"),
                ],
            };
            tokio::spawn(async move { mcp::call(&connection, operation).await })
        }

        fn project(&self) -> String {
            std::fs::canonicalize(self._directory.path())
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned()
        }

        fn written(&self) -> String {
            std::fs::read_to_string(self._directory.path().join("out")).unwrap()
        }

        async fn decide(
            &self,
            request: &SecretRequest,
            decision: SecretDecision,
        ) -> Result<bool, SecretError> {
            self.manager
                .resolve(&request.thread_id, &request.id, decision)
                .await
        }

        /// Every event this turn has sent so far, as text.
        fn drain(&mut self, seen: &mut Vec<String>) {
            while let Ok(event) = self.events.try_recv() {
                seen.push(format!("{event:?}"));
            }
        }

        /// The next form shown, recording every event on the way.
        async fn shown(&mut self, seen: &mut Vec<String>) -> SecretRequest {
            loop {
                let event =
                    tokio::time::timeout(std::time::Duration::from_secs(5), self.events.recv())
                        .await
                        .unwrap()
                        .unwrap();
                seen.push(format!("{event:?}"));
                if let OrchestratorEvent::Query(QueryEvent::SecretRequest(request)) = event {
                    seen.push(serde_json::to_string(&request).unwrap());
                    return request;
                }
            }
        }
    }

    /// The Global key scenario. Returns its canary and every event text.
    async fn global_key(seen: &mut Vec<String>) -> String {
        let op = tempfile::tempdir().unwrap();
        let mut fixture = fixture(op.path()).await;
        let secret = canary("deploy");
        let shared = fixture
            .manager
            .vault_create(entry("DEPLOY_KEY", CredentialScope::Global, value(&secret)))
            .await
            .unwrap();
        let call = fixture.call_for("DEPLOY_KEY");
        let request = fixture.shown(seen).await;
        assert!(request.saved_available);
        assert_eq!(request.saved_source, Some(SavedSource::Global));
        assert!(matches!(
            fixture.decide(&request, SecretDecision::ForgetSaved).await,
            Err(SecretError::Invalid(FORGET_SHARED))
        ));
        assert!(!call.is_finished(), "forget must not approve a command");
        assert!(fixture
            .manager
            .vault_entry(&shared.id)
            .await
            .unwrap()
            .is_some());
        assert!(fixture
            .decide(&request, SecretDecision::UseSaved)
            .await
            .unwrap());
        assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
        assert_eq!(fixture.written(), secret);
        fixture.drain(seen);
        secret
    }

    /// The 1Password scenario. Returns its canary and every event text.
    async fn one_password_key(seen: &mut Vec<String>) -> String {
        let op = tempfile::tempdir().unwrap();
        let mut fixture = fixture(op.path()).await;
        let secret = canary("token");
        std::fs::write(op.path().join("value"), &secret).unwrap();
        fixture
            .manager
            .vault_create(entry("TOKEN", CredentialScope::Global, reference()))
            .await
            .unwrap();
        let call = fixture.call_for("TOKEN");
        let request = fixture.shown(seen).await;
        assert!(request.saved_available);
        assert_eq!(request.saved_source, Some(SavedSource::OnePassword));
        // Signed out: nothing runs and the form stays to try again.
        assert!(matches!(
            fixture.decide(&request, SecretDecision::UseSaved).await,
            Err(SecretError::OnePassword(OnePasswordError::NeedsOwner))
        ));
        assert!(!call.is_finished());
        assert!(fixture.manager.lock().contains_key(&request.id));
        std::fs::write(op.path().join("signed-in"), "").unwrap();
        assert!(fixture
            .decide(&request, SecretDecision::UseSaved)
            .await
            .unwrap());
        assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
        assert_eq!(fixture.written(), secret);
        fixture.drain(seen);
        secret
    }

    #[tokio::test]
    async fn a_global_key_serves_any_project_and_cannot_be_forgotten_from_a_thread() {
        global_key(&mut Vec::new()).await;
    }

    #[tokio::test]
    async fn a_one_password_entry_reads_at_use_time() {
        one_password_key(&mut Vec::new()).await;
    }

    #[tokio::test]
    async fn synthetic_values_never_reach_events() {
        let mut seen = Vec::new();
        let canaries = [
            global_key(&mut seen).await,
            one_password_key(&mut seen).await,
        ];
        assert!(seen.len() >= 4, "both forms were seen: {}", seen.len());
        for text in &seen {
            for canary in &canaries {
                assert!(!text.contains(canary.as_str()), "a value reached an event");
            }
        }
    }

    #[tokio::test]
    async fn logins_and_bruh_entries_never_answer_a_thread() {
        let op = tempfile::tempdir().unwrap();
        let mut fixture = fixture(op.path()).await;
        let mut seen = Vec::new();
        fixture
            .manager
            .vault_create(entry(
                "SITE",
                CredentialScope::Global,
                VaultSecretInput::Login {
                    url: "https://shop.test/in".into(),
                    fields: LoginInput::Saved {
                        username: "owner@shop.test".into(),
                        password: Some(canary("password")),
                        totp: TotpInput::Keep,
                    },
                },
            ))
            .await
            .unwrap();
        fixture
            .manager
            .vault_create(entry(
                "SITE",
                CredentialScope::Bruh {
                    agent_id: "ash".into(),
                },
                value(&canary("bruh")),
            ))
            .await
            .unwrap();
        let call = fixture.call_for("SITE");
        let request = fixture.shown(&mut seen).await;
        assert!(!request.saved_available);
        assert_eq!(request.saved_source, None);
        assert!(matches!(
            fixture.decide(&request, SecretDecision::UseSaved).await,
            Err(SecretError::NotSaved)
        ));
        fixture
            .decide(&request, SecretDecision::Decline)
            .await
            .unwrap();
        assert_eq!(call.await.unwrap().unwrap().status, "declined");
    }

    #[tokio::test]
    async fn the_project_answers_first_and_vault_changes_refresh_open_forms() {
        let op = tempfile::tempdir().unwrap();
        let mut fixture = fixture(op.path()).await;
        let mut seen = Vec::new();
        let (shared_value, project_value) = (canary("shared"), canary("project"));
        std::fs::write(op.path().join("value"), &project_value).unwrap();
        std::fs::write(op.path().join("signed-in"), "").unwrap();
        let shared = fixture
            .manager
            .vault_create(entry("KEY", CredentialScope::Global, value(&shared_value)))
            .await
            .unwrap();
        let project = fixture
            .manager
            .vault_create(entry(
                "KEY",
                CredentialScope::Project {
                    path: fixture.project(),
                },
                reference(),
            ))
            .await
            .unwrap();
        let call = fixture.call_for("KEY");
        let request = fixture.shown(&mut seen).await;
        assert_eq!(request.saved_source, Some(SavedSource::OnePassword));
        assert!(matches!(
            fixture.decide(&request, SecretDecision::ForgetSaved).await,
            Err(SecretError::Invalid(FORGET_SHARED))
        ));
        // Deleting the project's entry offers the shared one instead.
        fixture.manager.vault_delete(&project.id).await.unwrap();
        let refreshed = fixture.shown(&mut seen).await;
        assert_eq!(refreshed.id, request.id);
        assert_eq!(refreshed.saved_source, Some(SavedSource::Global));
        assert!(refreshed.saved_available);
        // A new value for the shared key keeps it offered.
        let rotated = canary("rotated");
        fixture
            .manager
            .vault_update(
                &shared.id,
                VaultUpdate {
                    secret: value(&rotated),
                },
            )
            .await
            .unwrap();
        let refreshed = fixture.shown(&mut seen).await;
        assert_eq!(refreshed.saved_source, Some(SavedSource::Global));
        assert!(fixture
            .decide(&refreshed, SecretDecision::UseSaved)
            .await
            .unwrap());
        assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
        assert_eq!(fixture.written(), rotated);
        // Deleting the last match leaves nothing to offer.
        let call = fixture.call_for("KEY");
        let request = fixture.shown(&mut seen).await;
        assert_eq!(request.saved_source, Some(SavedSource::Global));
        fixture.manager.vault_delete(&shared.id).await.unwrap();
        let refreshed = fixture.shown(&mut seen).await;
        assert!(!refreshed.saved_available);
        assert_eq!(refreshed.saved_source, None);
        assert!(matches!(
            fixture.decide(&refreshed, SecretDecision::UseSaved).await,
            Err(SecretError::NotSaved)
        ));
        fixture
            .decide(&refreshed, SecretDecision::Decline)
            .await
            .unwrap();
        assert_eq!(call.await.unwrap().unwrap().status, "declined");
        fixture.drain(&mut seen);
        for text in &seen {
            for secret in [&shared_value, &project_value, &rotated] {
                assert!(!text.contains(secret.as_str()));
            }
        }
    }

    /// The desktop changes the vault through bruhs-server, another process
    /// with its own manager over the same store. Its own open thread forms
    /// hear of the change only when the page tells its manager (D20).
    #[tokio::test]
    async fn a_change_made_by_another_process_refreshes_forms_once_told() {
        let op = tempfile::tempdir().unwrap();
        let store = SecretStore::in_memory();
        let mut desktop = Fixture::with_manager(
            SecretManager::new(store.clone())
                .with_connections(crate::connections::Connections::in_memory())
                .with_one_password(OnePassword::at(
                    stand_in(op.path()),
                    std::time::Duration::from_secs(10),
                )),
        )
        .await;
        let server = SecretManager::new(store);
        let mut seen = Vec::new();
        let shared = server
            .vault_create(entry(
                "KEY",
                CredentialScope::Global,
                value(&canary("shared")),
            ))
            .await
            .unwrap();
        let call = desktop.call_for("KEY");
        let request = desktop.shown(&mut seen).await;
        assert_eq!(request.saved_source, Some(SavedSource::Global));
        // The server deletes it; the desktop's form still offers it until
        // the desktop's manager matches again.
        server.vault_delete(&shared.id).await.unwrap();
        assert!(
            desktop.events.try_recv().is_err(),
            "no refresh across processes"
        );
        desktop.manager.forget_attention([shared.id.as_str()]);
        desktop.manager.refresh_saved_requests("KEY").await;
        let refreshed = desktop.shown(&mut seen).await;
        assert_eq!(refreshed.id, request.id);
        assert!(!refreshed.saved_available);
        assert_eq!(refreshed.saved_source, None);
        desktop
            .decide(&refreshed, SecretDecision::Decline)
            .await
            .unwrap();
        assert_eq!(call.await.unwrap().unwrap().status, "declined");
    }

    #[tokio::test]
    async fn a_project_key_is_its_own_and_forgetting_it_falls_back_to_the_shared_one() {
        let op = tempfile::tempdir().unwrap();
        let mut fixture = fixture(op.path()).await;
        let mut seen = Vec::new();
        fixture
            .manager
            .vault_create(entry(
                "KEY",
                CredentialScope::Global,
                value(&canary("shared")),
            ))
            .await
            .unwrap();
        fixture
            .manager
            .vault_create(entry(
                "KEY",
                CredentialScope::Project {
                    path: fixture.project(),
                },
                value(&canary("project")),
            ))
            .await
            .unwrap();
        let call = fixture.call_for("KEY");
        let request = fixture.shown(&mut seen).await;
        assert_eq!(request.saved_source, Some(SavedSource::Project));
        assert!(!fixture
            .decide(&request, SecretDecision::ForgetSaved)
            .await
            .unwrap());
        let refreshed = fixture.shown(&mut seen).await;
        assert_eq!(refreshed.saved_source, Some(SavedSource::Global));
        assert!(!call.is_finished(), "forget must not approve a command");
        fixture
            .decide(&refreshed, SecretDecision::Decline)
            .await
            .unwrap();
        assert_eq!(call.await.unwrap().unwrap().status, "declined");
    }

    #[tokio::test]
    async fn save_securely_never_doubles_a_project_1password_entry() {
        let op = tempfile::tempdir().unwrap();
        let mut fixture = fixture(op.path()).await;
        let mut seen = Vec::new();
        let project = fixture
            .manager
            .vault_create(entry(
                "TOKEN",
                CredentialScope::Project {
                    path: fixture.project(),
                },
                reference(),
            ))
            .await
            .unwrap();
        let call = fixture.call_for("TOKEN");
        let request = fixture.shown(&mut seen).await;
        assert_eq!(request.saved_source, Some(SavedSource::OnePassword));
        let typed = canary("typed");
        assert!(matches!(
            fixture
                .decide(
                    &request,
                    SecretDecision::Provide {
                        value: typed.clone(),
                        save: true
                    }
                )
                .await,
            Err(SecretError::Invalid(bruhs_models::vault::DUPLICATE_NAME))
        ));
        assert!(!call.is_finished());
        let entries = fixture.manager.vault_entries().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, project.id);
        assert!(fixture
            .decide(
                &request,
                SecretDecision::Provide {
                    value: typed.clone(),
                    save: false
                }
            )
            .await
            .unwrap());
        assert_eq!(call.await.unwrap().unwrap().exit_code, Some(0));
        assert_eq!(fixture.written(), typed);
    }

    #[tokio::test]
    async fn deleting_entries_drops_their_attention() {
        let op = tempfile::tempdir().unwrap();
        let fixture = fixture(op.path()).await;
        let shared = fixture
            .manager
            .vault_create(entry("TOKEN", CredentialScope::Global, reference()))
            .await
            .unwrap();
        let own = fixture
            .manager
            .vault_create(entry(
                "OWN",
                CredentialScope::Bruh {
                    agent_id: "ash".into(),
                },
                reference(),
            ))
            .await
            .unwrap();
        // Signed out: both reads fail and leave attention.
        for id in [&shared.id, &own.id] {
            assert!(fixture.manager.resolve_value(id).await.is_err());
        }
        assert_eq!(fixture.manager.attention.lock().unwrap().len(), 2);
        fixture.manager.vault_delete(&shared.id).await.unwrap();
        assert!(!fixture
            .manager
            .attention
            .lock()
            .unwrap()
            .contains_key(&shared.id));
        let gone = HashSet::from(["ash".to_owned()]);
        assert_eq!(fixture.manager.forget_bruh_entries(&gone).await.unwrap(), 1);
        assert!(fixture.manager.attention.lock().unwrap().is_empty());
    }
}

/// Ruling A1 M9: broker files go under the credentials folder, which bruh
/// runs are denied, for an in-memory store too; only a test build's stays
/// in this process's temporary folder.
#[test]
fn broker_files_go_under_the_credentials_folder() {
    let catalog = || Ok(std::path::PathBuf::from("/data/credentials"));
    assert_eq!(
        super::broker_folder_in(false, catalog()).unwrap(),
        std::path::Path::new("/data/credentials/brokers")
    );
    assert!(super::broker_folder_in(true, catalog())
        .unwrap()
        .starts_with(std::env::temp_dir()));
    assert!(super::broker_folder_in(false, Err(SecretError::Catalog)).is_err());
    // This test build's in-memory store is isolated.
    let manager = SecretManager::new(SecretStore::in_memory());
    assert!(manager
        .broker_folder()
        .unwrap()
        .starts_with(std::env::temp_dir()));
}
