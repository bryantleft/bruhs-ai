use super::*;
use bruhs_core::meetings::MeetingPatch;
use bruhs_core::MeetingRecords;
use bruhs_core::SummaryGenerationStore;
use bruhs_models::SummaryGenerationState;

fn identified() -> SummarizeBody {
    SummarizeBody {
        request_id: Some("owned-request-1".into()),
        ..replace()
    }
}

#[tokio::test]
async fn successful_retry_returns_original_receipt_without_model_or_new_mutations() {
    let (store, meeting, segments, _) = fixture().await;
    let body = identified();
    let summary = generate_and_commit(
        &store,
        meeting.clone(),
        segments.clone(),
        body.clone(),
        |_, _, _| async { Ok((generated(), vec![])) },
    )
    .await
    .unwrap();
    store
        .patch_meeting(
            &meeting.id.parse().unwrap(),
            MeetingPatch {
                title: Some("Edited after generation".into()),
                status: None,
                ended_at_ms: None,
            },
        )
        .await
        .unwrap();
    let before = store.get_meeting(&meeting.id).await.unwrap();
    let replay = generate_and_commit(
        &store,
        meeting.clone(),
        segments,
        body.clone(),
        |_, _, _| async { panic!("A replay must not invoke a model") },
    )
    .await
    .unwrap();
    assert_eq!(replay, summary);
    assert_eq!(store.get_meeting(&meeting.id).await.unwrap(), before);
    let mut wrong = body;
    wrong.extract_observations = false;
    assert!(matches!(
        generate_and_commit(&store, meeting, vec![], wrong, |_, _, _| async {
            panic!("Changed options must not invoke a model")
        })
        .await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn failed_generation_is_terminal_and_retry_does_not_call_model_again() {
    let (store, meeting, segments, _) = fixture().await;
    let body = identified();
    assert!(generate_and_commit(
        &store,
        meeting.clone(),
        segments.clone(),
        body.clone(),
        |_, _, _| async { Err(summarize::SummarizeError::NoResponse) }
    )
    .await
    .is_err());
    assert_eq!(
        store
            .summary_generation(&meeting.id, "owned-request-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        SummaryGenerationState::Failed
    );
    assert!(matches!(
        generate_and_commit(&store, meeting, segments, body, |_, _, _| async {
            panic!("Failed identities are never automatically replayed")
        })
        .await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn pending_request_is_discoverable_and_blocks_same_and_different_identities() {
    let (store, meeting, segments, _) = fixture().await;
    let body = identified();
    store
        .begin_summary_generation(generation_request(&meeting.id, &body))
        .await
        .unwrap();
    assert_eq!(
        store
            .pending_summary_generation(&meeting.id)
            .await
            .unwrap()
            .unwrap()
            .request
            .request_id,
        "owned-request-1"
    );
    for body in [
        body,
        SummarizeBody {
            request_id: Some("another".into()),
            ..replace()
        },
    ] {
        assert!(matches!(
            generate_and_commit(
                &store,
                meeting.clone(),
                segments.clone(),
                body,
                |_, _, _| async { panic!("Unresolved request must prevent another invocation") }
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
}

#[tokio::test]
async fn abandoned_http_task_leaves_pending_receipt_instead_of_replaying() {
    let (store, meeting, segments, _) = fixture().await;
    let store = Arc::new(store);
    let id = meeting.id.clone();
    let ready = Arc::new(tokio::sync::Notify::new());
    let entered = ready.notified();
    let task_store = store.clone();
    let signal = ready.clone();
    let task = tokio::spawn(async move {
        generate_and_commit(
            task_store.as_ref(),
            meeting,
            segments,
            identified(),
            |_, _, _| async move {
                signal.notify_one();
                std::future::pending().await
            },
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), entered)
        .await
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        store
            .summary_generation(&id, "owned-request-1")
            .await
            .unwrap()
            .unwrap()
            .state,
        SummaryGenerationState::Pending
    );
    assert!(store
        .get_meeting(&id)
        .await
        .unwrap()
        .unwrap()
        .summary
        .is_none());
}

#[tokio::test]
async fn actual_http_receipt_read_cancel_and_completed_replay_are_provider_free() {
    let (store, meeting, segments, _) = fixture().await;
    generate_and_commit(
        &store,
        meeting.clone(),
        segments,
        identified(),
        |_, _, _| async { Ok((generated(), vec![])) },
    )
    .await
    .unwrap();
    let pending = SummarizeBody {
        request_id: Some("pending-request".into()),
        ..replace()
    };
    store
        .begin_summary_generation(generation_request(&meeting.id, &pending))
        .await
        .unwrap();
    let state = AppState::builder().meetings(Arc::new(store)).build();
    let app = axum::Router::new()
        .route(
            "/api/meetings/{id}/summarize",
            axum::routing::post(post_summarize),
        )
        .route(
            "/api/meetings/{id}/generations",
            axum::routing::get(get_pending_generation),
        )
        .route(
            "/api/meetings/{id}/generations/{request_id}",
            axum::routing::get(get_generation).delete(cancel_generation),
        )
        .with_state(Arc::new(state));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    struct Server(tokio::task::JoinHandle<()>);
    impl Drop for Server {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _server = Server(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let base = format!("{origin}/api/meetings/{}", meeting.id);
    let response = client
        .get(format!("{base}/generations/owned-request-1"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let receipt: bruhs_models::SummaryGenerationReceipt = response.json().await.unwrap();
    assert_eq!(
        receipt.state,
        SummaryGenerationState::Succeeded {
            summary: generated()
        }
    );
    let response = client.post(format!("{base}/summarize")).json(&serde_json::json!({"request_id":"owned-request-1","extract_observations":true,"replace_observations":true})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<MeetingSummary>().await.unwrap(),
        generated()
    );
    let receipt = client
        .get(format!("{base}/generations"))
        .send()
        .await
        .unwrap()
        .json::<bruhs_models::SummaryGenerationReceipt>()
        .await
        .unwrap();
    assert_eq!(receipt.request.request_id, "pending-request");
    let receipt = client
        .delete(format!("{base}/generations/pending-request"))
        .send()
        .await
        .unwrap()
        .json::<bruhs_models::SummaryGenerationReceipt>()
        .await
        .unwrap();
    assert_eq!(receipt.state, SummaryGenerationState::Cancelled);
    assert_eq!(
        client
            .get(format!(
                "{origin}/api/meetings/wrong/generations/owned-request-1"
            ))
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert!(client
        .post(format!("{base}/summarize"))
        .json(&serde_json::json!({"request_id":7}))
        .send()
        .await
        .unwrap()
        .status()
        .is_client_error());

    // Fencing an identity the server never saw records a cancelled tombstone,
    // so the delayed request is refused without invoking the provider.
    let fence = serde_json::json!({"extract_observations":true,"replace_observations":false});
    let response = client
        .delete(format!("{base}/generations/unsent-request"))
        .json(&fence)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let receipt: bruhs_models::SummaryGenerationReceipt = response.json().await.unwrap();
    assert_eq!(receipt.request.request_id, "unsent-request");
    assert_eq!(receipt.state, SummaryGenerationState::Cancelled);
    let late = client
        .post(format!("{base}/summarize"))
        .json(&serde_json::json!({"request_id":"unsent-request","extract_observations":true,"replace_observations":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(late.status(), reqwest::StatusCode::CONFLICT);
    assert!(client
        .get(format!("{base}/generations"))
        .send()
        .await
        .unwrap()
        .json::<Option<bruhs_models::SummaryGenerationReceipt>>()
        .await
        .unwrap()
        .is_none());
    // Fencing a completed identity with different options is a conflict, and the
    // completed receipt is unchanged.
    let mismatched = client
        .delete(format!("{base}/generations/owned-request-1"))
        .json(&serde_json::json!({"extract_observations":false,"replace_observations":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(mismatched.status(), reqwest::StatusCode::CONFLICT);
    let response = client
        .delete(format!("{base}/generations/owned-request-1"))
        .json(&serde_json::json!({"extract_observations":true,"replace_observations":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .json::<bruhs_models::SummaryGenerationReceipt>()
            .await
            .unwrap()
            .state,
        SummaryGenerationState::Succeeded {
            summary: generated()
        }
    );
    assert_eq!(
        client
            .delete(format!("{origin}/api/meetings/wrong/generations/unsent-2"))
            .json(&fence)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::NOT_FOUND
    );
    assert!(client
        .delete(format!("{base}/generations/unsent-3"))
        .json(&serde_json::json!({"extract_observations":false,"replace_observations":true}))
        .send()
        .await
        .unwrap()
        .status()
        .is_client_error());
}
