use super::*;
use axum::response::IntoResponse;
use tokio::sync::broadcast;
use tokio_stream::StreamExt;

async fn next_event(
    stream: &mut (impl Stream<Item = Result<Event, Infallible>> + Unpin),
) -> MeetingEvent {
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response = Sse::new(tokio_stream::iter([Ok::<_, Infallible>(event)])).into_response();
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let frame = std::str::from_utf8(&body).unwrap();
    serde_json::from_str(frame.strip_prefix("data: ").unwrap().trim_end()).unwrap()
}

#[tokio::test]
async fn lag_requests_reconciliation_then_delivers_remaining_events_without_recording_error() {
    let (sender, receiver) = broadcast::channel(1);
    let mut stream = Box::pin(events(receiver));
    assert_eq!(next_event(&mut stream).await, MeetingEvent::StreamOpened);
    sender
        .send(MeetingEvent::Started {
            meeting_id: "synthetic".into(),
            started_at_ms: 1.0,
        })
        .unwrap();
    let ended = MeetingEvent::Ended {
        meeting_id: "synthetic".into(),
        ended_at_ms: 2.0,
        status: bruhs_models::MeetingStatus::Ended,
    };
    sender.send(ended.clone()).unwrap();
    assert_eq!(next_event(&mut stream).await, MeetingEvent::StreamOpened);
    assert_eq!(next_event(&mut stream).await, ended);
    drop(sender);
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn repeated_lag_remains_recoverable_on_the_same_stream() {
    let (sender, receiver) = broadcast::channel(1);
    let mut stream = Box::pin(events(receiver));
    assert_eq!(next_event(&mut stream).await, MeetingEvent::StreamOpened);
    for _ in 0..3 {
        for _ in 0..100 {
            sender.send(MeetingEvent::PeopleUpdated).unwrap();
        }
        assert_eq!(next_event(&mut stream).await, MeetingEvent::StreamOpened);
        assert_eq!(next_event(&mut stream).await, MeetingEvent::PeopleUpdated);
    }
}

#[tokio::test]
async fn genuine_capture_errors_keep_their_identity_and_message() {
    let (sender, receiver) = broadcast::channel(1);
    let mut stream = Box::pin(events(receiver));
    assert_eq!(next_event(&mut stream).await, MeetingEvent::StreamOpened);
    let failure = MeetingEvent::CaptureError {
        meeting_id: "synthetic".into(),
        message: "Synthetic hardware failure".into(),
    };
    sender.send(failure.clone()).unwrap();
    assert_eq!(next_event(&mut stream).await, failure);
}
