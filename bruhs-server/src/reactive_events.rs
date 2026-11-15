//! Reconcile durable app records into automation events, independent of UI subscribers.

use bruhs_core::{
    reactive::{ReactiveError, ReactiveRuntime, SourceEmission},
    AppState,
};
use bruhs_models::{
    reactive::{EventSource, SourceKind, MAX_PAYLOAD_BYTES},
    Meeting, MeetingStatus,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

// Broadcast is a fast wakeup, not delivery. Reconciliation provides a one-minute
// catch-up budget when a broadcast is lost, without requiring an open UI.
const RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
// Two bounded text fields may each expand sixfold under JSON escaping.
// Twelve of sixteen envelope shares cover them; four remain for metadata.
const CONTEXT_BYTES: usize = MAX_PAYLOAD_BYTES / 16;

pub fn start(runtime: ReactiveRuntime, app: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut events = app.subscribe_meeting_events();
        let mut changes = runtime.watch_changes();
        let mut timer = tokio::time::interval(RECONCILE_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut previous = Vec::new();
        loop {
            let (changed, meeting_id) = tokio::select! {
                _ = timer.tick() => (false, None),
                result = events.recv() => {
                    use bruhs_models::MeetingEvent;
                    match result {
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) | Ok(MeetingEvent::StreamOpened) => (false, None),
                        Ok(MeetingEvent::Ended {meeting_id,..} | MeetingEvent::TranscriptionCompleted {meeting_id} | MeetingEvent::SpeakersAssigned {meeting_id} | MeetingEvent::Summary {meeting_id,..}) => (false, Some(meeting_id)),
                        _ => continue,
                    }
                }
                result = changes.changed() => {
                    if result.is_err() { return; }
                    (true, None)
                }
            };
            let sources = match runtime.source_configs().await {
                Ok(sources) => sources
                    .into_iter()
                    .filter(|source| matches!(source.kind, SourceKind::AppEvent { .. }))
                    .collect::<Vec<_>>(),
                Err(error) => {
                    tracing::error!("Could not load app event subscriptions: {error}");
                    continue;
                }
            };
            if changed && sources == previous {
                continue;
            }
            previous = sources.clone();
            if sources.is_empty() {
                continue;
            }
            if let Err(error) = reconcile(&runtime, &app, &sources, meeting_id.as_deref()).await {
                tracing::error!("App event reconciliation failed: {error}");
                for source in &sources {
                    if let Err(status_error) = runtime
                        .source_status(&source.id, Some(error.to_string()))
                        .await
                    {
                        tracing::error!("Could not save app event failure: {status_error}");
                    }
                }
            }
        }
    })
}

pub(crate) async fn reconcile(
    runtime: &ReactiveRuntime,
    app: &AppState,
    sources: &[EventSource],
    meeting_id: Option<&str>,
) -> Result<(), ReactiveError> {
    let records = if let Some(id) = meeting_id {
        app.meetings()
            .get_meeting(id)
            .await
            .map(|meeting| meeting.into_iter().collect())
    } else {
        app.meetings().list_meetings().await
    };
    let meetings = records.map_err(|_| {
        ReactiveError::Storage(
            "Could not read meetings for reactive agents. No event checkpoint advanced.".into(),
        )
    })?;
    let finalized: HashSet<_> = if sources.iter().any(|source| matches!(&source.kind, SourceKind::AppEvent{topic} if topic == "meeting.transcribed")) {
        runtime.app_completions("meeting.transcribed").await?.into_iter().collect()
    } else { HashSet::new() };
    let mut transcripts: HashMap<String, Value> = HashMap::new();
    for source in sources {
        let SourceKind::AppEvent { topic } = &source.kind else {
            continue;
        };
        if ![
            "meeting.completed",
            "meeting.transcribed",
            "meeting.summary",
        ]
        .contains(&topic.as_str())
        {
            runtime.source_status(&source.id, Some("Choose an app event topic: meeting.completed, meeting.transcribed or meeting.summary. Other producers can use a Custom source.".into())).await?;
            continue;
        }
        for meeting in meetings
            .iter()
            .filter(|meeting| meeting.status == MeetingStatus::Ended)
        {
            if topic == "meeting.transcribed" && !finalized.contains(&meeting.id) {
                continue;
            }
            let cursor_key = format!("meeting:{}", meeting.id);
            let saved = runtime.cursor(&source.id, &cursor_key).await?;
            if topic == "meeting.transcribed" && saved.is_some() {
                continue;
            }
            let mut payload = metadata(meeting);
            match topic.as_str() {
                "meeting.summary" => {
                    let Some(summary) = &meeting.summary else {
                        continue;
                    };
                    let summary = serde_json::to_string(summary).map_err(|_| {
                        ReactiveError::Storage("The generated summary could not be encoded.".into())
                    })?;
                    payload["summary_digest"] =
                        json!(format!("{:x}", Sha256::digest(summary.as_bytes())));
                    payload["summary"] = Value::String(bounded(&summary));
                    payload["context_truncated"] = json!(summary.len() > CONTEXT_BYTES);
                }
                "meeting.transcribed" => {
                    if !transcripts.contains_key(&meeting.id) {
                        let segments = app.meetings().list_segments(&meeting.id).await.map_err(|_| ReactiveError::Storage("Could not read the completed transcript. No event checkpoint advanced.".into()))?;
                        let mut text = String::new();
                        let mut digest = Sha256::new();
                        let mut truncated = false;
                        for segment in segments.iter().filter(|segment| !segment.partial) {
                            let bytes = serde_json::to_vec(segment).map_err(|_| {
                                ReactiveError::Storage(
                                    "A transcript segment could not be encoded.".into(),
                                )
                            })?;
                            digest.update(&bytes);
                            let line = format!("[{}ms] {}\n", segment.start_ms, segment.text);
                            if text.len().saturating_add(line.len()) > CONTEXT_BYTES {
                                truncated = true;
                                if text.is_empty() {
                                    text = bounded(&line);
                                }
                                continue;
                            }
                            if !truncated {
                                text.push_str(&line);
                            }
                        }
                        transcripts.insert(meeting.id.clone(), json!({"text":text,"truncated":truncated,"digest":format!("{:x}", digest.finalize())}));
                    }
                    let transcript = &transcripts[&meeting.id];
                    if transcript["text"].as_str().is_none_or(str::is_empty) {
                        continue;
                    }
                    payload["transcript"] = transcript["text"].clone();
                    payload["context_truncated"] = transcript["truncated"].clone();
                    payload["transcript_digest"] = transcript["digest"].clone();
                }
                _ => {}
            }
            let key = if topic == "meeting.completed" {
                format!("{}:{topic}", meeting.id)
            } else {
                let digest = payload
                    .get("summary_digest")
                    .or_else(|| payload.get("transcript_digest"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ReactiveError::Storage(
                            "A completed app event is missing its content receipt.".into(),
                        )
                    })?;
                format!("{}:{topic}:{digest}", meeting.id)
            };
            if saved.is_some_and(|cursor| cursor["key"].as_str() == Some(key.as_str())) {
                continue;
            }
            // Cursor and event receipts commit together. Replay after a crash
            // sees the same key, so it cannot queue a second copy of the work.
            runtime
                .ingest(
                    source,
                    vec![SourceEmission {
                        key: key.clone(),
                        payload,
                    }],
                    &cursor_key,
                    json!({"key":key}),
                )
                .await?;
        }
        runtime.source_status(&source.id, None).await?;
    }
    Ok(())
}

fn metadata(meeting: &Meeting) -> Value {
    json!({"meeting_id":meeting.id,"title":bounded(&meeting.title),"started_at_ms":meeting.started_at_ms,"ended_at_ms":meeting.ended_at_ms,"status":"ended"})
}

fn bounded(text: &str) -> String {
    let mut end = text.len().min(CONTEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::reactive::{ReactiveChange, ReactiveCommand};

    async fn setup(
        topic: &str,
    ) -> (
        tempfile::TempDir,
        ReactiveRuntime,
        AppState,
        EventSource,
        Meeting,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(directory.path().join("events.sqlite3"))
            .await
            .unwrap();
        let app = AppState::new();
        let mut meeting = Meeting::new("Review", 1_000.0);
        meeting.status = MeetingStatus::Ended;
        app.meetings()
            .create_meeting(meeting.clone())
            .await
            .unwrap();
        let source = EventSource {
            id: "meetings".into(),
            name: "Meetings".into(),
            enabled: true,
            kind: SourceKind::AppEvent {
                topic: topic.into(),
            },
        };
        for change in [
            ReactiveChange::SaveAgent {
                agent: bruhs_models::reactive::ReactiveAgent {
                    id: "clerk".into(),
                    name: "Clerk".into(),
                    instructions: "Draft notes".into(),
                    memory: String::new(),
                    model: "sonnet".into(),
                    project_path: directory.path().to_string_lossy().into_owned(),
                    enabled: true,
                },
            },
            ReactiveChange::SaveSource {
                source: source.clone(),
            },
            ReactiveChange::SaveSubscription {
                subscription: bruhs_models::reactive::Subscription {
                    id: "notes".into(),
                    source_id: source.id.clone(),
                    agent_id: "clerk".into(),
                    instructions: "Summarize".into(),
                    enabled: true,
                    auto_run: false,
                    debounce_ms: 0,
                    throttle_ms: 0,
                    on_busy: Default::default(),
                },
            },
        ] {
            runtime
                .command(ReactiveCommand::Apply { change })
                .await
                .unwrap();
        }
        (directory, runtime, app, source, meeting)
    }

    #[tokio::test]
    async fn recording_stop_cannot_trigger_transcribed_before_durable_finalization() {
        let (_directory, runtime, app, source, meeting) = setup("meeting.transcribed").await;
        app.meetings()
            .append_segment(bruhs_models::TranscriptSegment {
                id: "segment-1".into(),
                meeting_id: meeting.id.clone(),
                speaker_id: None,
                start_ms: 0,
                end_ms: 1_000,
                text: "A live pass already has text".into(),
                confidence: 0.9,
                partial: false,
                speaker_confidence: None,
            })
            .await
            .unwrap();
        reconcile(&runtime, &app, std::slice::from_ref(&source), None)
            .await
            .unwrap();
        assert!(runtime.snapshot().await.unwrap().events.is_empty());
        runtime
            .record_app_completion("meeting.transcribed", &meeting.id)
            .await
            .unwrap();
        reconcile(
            &runtime,
            &app,
            std::slice::from_ref(&source),
            Some(&meeting.id),
        )
        .await
        .unwrap();
        reconcile(&runtime, &app, std::slice::from_ref(&source), None)
            .await
            .unwrap();
        let snapshot = runtime.snapshot().await.unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.runs.len(), 1);
        assert_eq!(
            snapshot.events[0].payload["transcript"],
            "[0ms] A live pass already has text\n"
        );
    }

    #[tokio::test]
    async fn summary_tail_changes_are_not_lost_when_context_is_truncated() {
        let (_directory, runtime, app, source, meeting) = setup("meeting.summary").await;
        let id = meeting.id.parse().unwrap();
        let mut summary = bruhs_models::MeetingSummary {
            tldr: "x".repeat(CONTEXT_BYTES + 100),
            action_items: Vec::new(),
            decisions: Vec::new(),
            topics: Vec::new(),
            generated_by: "fixture".into(),
            generated_at: 1,
        };
        app.meetings()
            .patch_meeting_summary(&id, summary.clone())
            .await
            .unwrap();
        reconcile(&runtime, &app, std::slice::from_ref(&source), None)
            .await
            .unwrap();
        summary
            .tldr
            .push_str(" a changed decision beyond the preview");
        app.meetings()
            .patch_meeting_summary(&id, summary)
            .await
            .unwrap();
        reconcile(&runtime, &app, std::slice::from_ref(&source), None)
            .await
            .unwrap();
        let snapshot = runtime.snapshot().await.unwrap();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(
            snapshot.events[0].payload["summary"],
            snapshot.events[1].payload["summary"]
        );
        assert_ne!(
            snapshot.events[0].payload["summary_digest"],
            snapshot.events[1].payload["summary_digest"]
        );
        assert_eq!(snapshot.events[0].payload["context_truncated"], true);
    }

    #[tokio::test]
    async fn reconcile_recovers_missed_events_without_duplicate_or_audio_path() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(dir.path().join("reactive.sqlite3"))
            .await
            .unwrap();
        let app = AppState::new();
        let mut meeting = Meeting::new("Review", 1_000.0);
        meeting.status = MeetingStatus::Ended;
        meeting.audio_path = Some("/private/audio.wav".into());
        app.meetings().create_meeting(meeting).await.unwrap();
        let source = EventSource {
            id: "meetings".into(),
            name: "Finished meetings".into(),
            enabled: true,
            kind: SourceKind::AppEvent {
                topic: "meeting.completed".into(),
            },
        };
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveSource {
                    source: source.clone(),
                },
            })
            .await
            .unwrap();
        // Explicit sources passed here exercise reconciliation without a worker
        // or live provider; ingest still enforces current subscription state.
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent {
                    agent: bruhs_models::reactive::ReactiveAgent {
                        id: "notes".into(),
                        name: "Notes".into(),
                        instructions: "Draft notes".into(),
                        memory: String::new(),
                        model: "sonnet".into(),
                        project_path: dir.path().to_string_lossy().into_owned(),
                        enabled: true,
                    },
                },
            })
            .await
            .unwrap();
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveSubscription {
                    subscription: bruhs_models::reactive::Subscription {
                        id: "meeting-notes".into(),
                        source_id: source.id.clone(),
                        agent_id: "notes".into(),
                        instructions: "Review meeting".into(),
                        enabled: true,
                        auto_run: false,
                        debounce_ms: 0,
                        throttle_ms: 0,
                        on_busy: Default::default(),
                    },
                },
            })
            .await
            .unwrap();
        reconcile(&runtime, &app, std::slice::from_ref(&source), None)
            .await
            .unwrap();
        reconcile(&runtime, &app, std::slice::from_ref(&source), None)
            .await
            .unwrap();
        let snapshot = runtime.snapshot().await.unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.runs.len(), 1);
        assert!(!serde_json::to_string(&snapshot.events)
            .unwrap()
            .contains("audio.wav"));
    }
}
