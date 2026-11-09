//! Real HTTP and SQLite, with an explicitly synthetic local provider executable.
use super::*;
use bruhs_core::meetings::MeetingPatch;
use bruhs_core::MeetingRecords;
use bruhs_core::PeopleStore;
use std::os::unix::fs::PermissionsExt;

fn provider(directory: &Path, fail: bool) {
    let model_result = serde_json::json!({"tldr":"New HTTP summary","action_items":[],"decisions":[],"topics":[],"per_person_observations":[{"speaker_label":"Alice","kind":"topic","text":"New HTTP observation"}]});
    let event = serde_json::json!({"type":"result","subtype":if fail {"error"} else {"success"},"result":model_result.to_string()});
    let script = directory.join("claude");
    let calls = directory
        .join("provider-calls")
        .to_string_lossy()
        .replace('\'', "'\\''");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf 'called\\n' >> '{calls}'\nprintf '%s\\n' '{}'\n",
            event.to_string().replace('\'', "'\\''")
        ),
    )
    .expect("write synthetic provider");
    std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o700))
        .expect("make fixture executable");
}

#[tokio::test]
async fn http_generation_commits_together_and_provider_failure_retains_exact_prior_state() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("summary.db");
    let store = SqliteMeetingStore::open(&database).await.unwrap();
    let mut meeting = Meeting::new("HTTP summary", 0.0);
    meeting.id = "http-summary".into();
    let mut person = bruhs_models::PersonProfile::new("Alice");
    person.observations.push(bruhs_models::PersonObservation {
        at_meeting_id: meeting.id.clone(),
        kind: bruhs_models::ObservationKind::Topic,
        text: "Original HTTP observation".into(),
        created_at: 1,
    });
    let person = store.upsert_person(person).await.unwrap();
    meeting.speakers = vec![bruhs_models::Speaker {
        id: "them".into(),
        label: "Alice".into(),
        person_id: Some(person.id.clone()),
        color: None,
    }];
    let meeting = store.create_meeting(meeting).await.unwrap();
    let mut speech = segment();
    speech.meeting_id = meeting.id.clone();
    speech.speaker_id = Some("them".into());
    store.append_segment(speech).await.unwrap();
    let client = Client::new();
    provider(directory.path(), true);
    let mut server = Server::start_with_provider(
        &database,
        "http://127.0.0.1:0",
        &client,
        Some(directory.path()),
    )
    .await;
    let endpoint = format!("{}/api/meetings/{}/summarize", server.url, meeting.id);
    let failed = client
        .post(&endpoint)
        .json(
            &serde_json::json!({"request_id":"failed-http-generation","replace_observations":true}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        store.get_person(&person.id).await.unwrap(),
        Some(person.clone())
    );
    assert_eq!(
        store.get_meeting(&meeting.id).await.unwrap(),
        Some(meeting.clone())
    );
    provider(directory.path(), false);
    let success = client
        .post(&endpoint)
        .json(&serde_json::json!({"request_id":"completed-http-generation","replace_observations":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        success.status(),
        StatusCode::OK,
        "{}",
        success.text().await.unwrap()
    );
    let saved = store.get_meeting(&meeting.id).await.unwrap().unwrap();
    let updated = store.get_person(&person.id).await.unwrap().unwrap();
    assert_eq!(saved.summary.as_ref().unwrap().tldr, "New HTTP summary");
    assert_eq!(updated.observations.len(), 1);
    assert_eq!(updated.observations[0].text, "New HTTP observation");
    server.stop().await;
    drop(store);
    let reopened = SqliteMeetingStore::open(&database).await.unwrap();
    assert_eq!(
        reopened.get_person(&person.id).await.unwrap(),
        Some(updated.clone())
    );
    assert_eq!(
        reopened.get_meeting(&meeting.id).await.unwrap(),
        Some(saved.clone())
    );
    // A killed/restarted server must answer the original request from its
    // receipt even when the provider would now fail and the meeting changed.
    provider(directory.path(), true);
    let newer = reopened
        .patch_meeting(
            &meeting.id.parse().unwrap(),
            MeetingPatch {
                title: Some("Newer user title".into()),
                status: None,
                ended_at_ms: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    let mut restarted = Server::start_with_provider(
        &database,
        "http://127.0.0.1:0",
        &client,
        Some(directory.path()),
    )
    .await;
    let endpoint = format!("{}/api/meetings/{}/summarize", restarted.url, meeting.id);
    let replay = client.post(&endpoint).json(&serde_json::json!({"request_id":"completed-http-generation","replace_observations":true})).send().await.unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        replay.json::<bruhs_models::MeetingSummary>().await.unwrap(),
        saved.summary.unwrap()
    );
    let receipt = client
        .get(format!(
            "{}/api/meetings/{}/generations/completed-http-generation",
            restarted.url, meeting.id
        ))
        .send()
        .await
        .unwrap()
        .json::<bruhs_models::SummaryGenerationReceipt>()
        .await
        .unwrap();
    assert!(matches!(
        receipt.state,
        bruhs_models::SummaryGenerationState::Succeeded { .. }
    ));
    let failed_replay = client
        .post(&endpoint)
        .json(
            &serde_json::json!({"request_id":"failed-http-generation","replace_observations":true}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(failed_replay.status(), StatusCode::CONFLICT);
    restarted.stop().await;
    assert_eq!(
        reopened.get_meeting(&meeting.id).await.unwrap(),
        Some(newer)
    );
    assert_eq!(
        reopened.get_person(&person.id).await.unwrap(),
        Some(updated)
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("provider-calls"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}
