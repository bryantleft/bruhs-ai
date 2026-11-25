//! Build the same task event while batching small JSON serializer writes.

use bruhs_core::BackgroundTask;
use serde::Serialize;

use crate::sse::Event;

// The constructor sweep covered 256 through 16,384 bytes. Eight KiB batches
// per-field framing work without a full JSON temporary. See the buffered task
// event section in docs/performance-goal-2026-11-09.md for CPU/peak tradeoffs.
const BUFFER_BYTES: usize = 8 * 1024;

pub(super) fn encode(tasks: Vec<BackgroundTask>) -> Result<Event, serde_json::Error> {
    // The exact empty payload needs no staging allocation.
    if tasks.is_empty() {
        return Ok(Event::default().data("[]"));
    }
    buffered(&tasks, BUFFER_BYTES)
}

fn buffered<T: Serialize + ?Sized>(data: &T, capacity: usize) -> Result<Event, serde_json::Error> {
    Event::default().json_data_buffered(data, capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::Sse;
    use axum::response::IntoResponse as _;
    use serde::ser::{SerializeSeq as _, Serializer};
    use serde_json::json;
    use std::convert::Infallible;

    async fn frame(event: Event) -> Vec<u8> {
        let body = Sse::new(futures_util::stream::iter([Ok::<_, Infallible>(event)]))
            .into_response()
            .into_body();
        axum::body::to_bytes(body, usize::MAX)
            .await
            .expect("complete in-memory event")
            .to_vec()
    }

    fn reference_frame<T: Serialize>(value: &T, named: bool) -> Result<Vec<u8>, serde_json::Error> {
        let json = serde_json::to_string(value)?;
        let mut event = Event::default().data(json);
        if named {
            event = event.event("tasks");
        }
        Ok(event.finalize().to_vec())
    }

    #[tokio::test]
    async fn buffering_preserves_every_raw_frame_byte_at_small_and_large_boundaries() {
        let values = [
            json!([]),
            json!({"text": "aΩ😀中\r\n\0z", "empty": [], "nested": [null, true, -42]}),
            json!({"large": "Ω".repeat(32768)}),
        ];
        for value in values {
            for capacity in [0, 1, 2, 3, 4, 7, 256, BUFFER_BYTES] {
                for named in [false, true] {
                    let old = reference_frame(&value, named).expect("reference event");
                    let new = buffered(&value, capacity).expect("buffered event");
                    let new = if named { new.event("tasks") } else { new };
                    assert_eq!(frame(new).await, old);
                }
            }
        }
    }

    #[tokio::test]
    async fn buffering_preserves_pretty_raw_json_line_framing() {
        let raw = serde_json::value::RawValue::from_string(
            "{\r\n\"text\":\"Ω😀\",\n\"n\":null\n}".to_owned(),
        )
        .expect("raw JSON");
        let expected = reference_frame(&raw, false).expect("reference raw JSON");
        for capacity in [0, 1, 2, 3, 4, 7, BUFFER_BYTES] {
            assert_eq!(
                frame(buffered(&raw, capacity).expect("buffered raw JSON")).await,
                expected
            );
        }
    }

    struct Failing;
    impl Serialize for Failing {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(None)?;
            sequence.serialize_element("accepted prefix")?;
            Err(serde::ser::Error::custom("fixture serialization refused"))
        }
    }

    #[test]
    fn partial_serialization_failure_returns_an_error_without_an_event() {
        let old = serde_json::to_string(&Failing).expect_err("reference fails");
        for capacity in [0, 1, BUFFER_BYTES] {
            let new = buffered(&Failing, capacity).expect_err("buffered event fails");
            assert_eq!(new.to_string(), old.to_string());
        }
    }
}
