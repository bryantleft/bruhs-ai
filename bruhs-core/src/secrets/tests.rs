use super::*;

const PRIVATE: &str = "synthetic-credential-canary";

#[tokio::test]
async fn management_does_not_wait_for_an_unread_turn_stream() {
    let mut fixture = Fixture::new().await;
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    fixture.provide(&request, true).await.unwrap();
    call.await.unwrap().unwrap();
    let id = fixture.manager.list_credentials().await.unwrap()[0]
        .id
        .clone();
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    while fixture
        ._sender
        .try_send(OrchestratorEvent::Query(QueryEvent::Text("queued".into())))
        .is_ok()
    {}
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        fixture.manager.delete_credential(&id),
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
    let id = fixture.manager.list_credentials().await.unwrap()[0]
        .id
        .clone();
    let call = fixture.call("exit 0");
    let request = fixture.request().await;
    fixture
        .manager
        .replace_credential(&id, "private-rotated".into())
        .await
        .unwrap();
    assert!(fixture.request().await.saved_available);
    assert!(!call.is_finished());
    fixture.manager.delete_credential(&id).await.unwrap();
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
        let manager = SecretManager::new(store);
        let directory = tempfile::tempdir().unwrap();
        let (tx, events) = mpsc::channel(32);
        let scope = manager.turn(ThreadId::new_v4(), directory.path().into(), tx.clone());
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
    let scope = manager.turn(ThreadId::new_v4(), PathBuf::from("/tmp"), tx);
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
