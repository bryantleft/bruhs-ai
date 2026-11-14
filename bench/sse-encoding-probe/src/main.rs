use std::convert::Infallible;
use std::hint::black_box;
#[cfg(not(feature = "count-allocations"))]
use std::time::Instant;

use axum::response::{IntoResponse as _, Sse, sse::Event};
use serde::Serialize;
use serde::ser::{SerializeSeq as _, Serializer};
use serde_json::json;
use sha2::{Digest as _, Sha256};

// Use the actual production wire-model definitions without constructing any
// application state or enabling its provider/storage dependencies.
#[cfg(feature = "count-allocations")]
mod allocations;
mod buffered;
#[expect(
    dead_code,
    reason = "the production module also defines unrelated TaskEvent variants"
)]
#[path = "../../../bruhs-core/src/models/task.rs"]
mod task_model;
use task_model::{BackgroundTask, TaskKind, TaskStatus};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Method {
    JsonData,
    BulkData,
    #[serde(rename = "buffer-256")]
    Buffer256,
    #[serde(rename = "buffer-1024")]
    Buffer1024,
    #[serde(rename = "buffer-4096")]
    Buffer4096,
    #[serde(rename = "buffer-8192")]
    Buffer8192,
    #[serde(rename = "buffer-16384")]
    Buffer16384,
    #[serde(rename = "simd-buffer-8192")]
    SimdBuffer8192,
    #[serde(rename = "byte-buffer-8192")]
    ByteBuffer8192,
}

const METHODS: [Method; 9] = [
    Method::JsonData,
    Method::BulkData,
    Method::Buffer256,
    Method::Buffer1024,
    Method::Buffer4096,
    Method::Buffer8192,
    Method::Buffer16384,
    Method::SimdBuffer8192,
    Method::ByteBuffer8192,
];

fn construct<T: Serialize + ?Sized>(
    tasks: &T,
    method: Method,
    named: bool,
) -> Result<Event, String> {
    let event = match method {
        Method::JsonData => Event::default()
            .json_data(tasks)
            .map_err(|error| error.to_string())?,
        Method::BulkData => {
            Event::default().data(serde_json::to_string(tasks).map_err(|error| error.to_string())?)
        }
        Method::Buffer256 => buffered::event::<_, false>(tasks, 256)?,
        Method::Buffer1024 => buffered::event::<_, false>(tasks, 1024)?,
        Method::Buffer4096 => buffered::event::<_, false>(tasks, 4096)?,
        Method::Buffer8192 => buffered::event::<_, false>(tasks, 8192)?,
        Method::Buffer16384 => buffered::event::<_, false>(tasks, 16384)?,
        Method::SimdBuffer8192 => buffered::event::<_, true>(tasks, 8192)?,
        Method::ByteBuffer8192 => byte_event(tasks, 8192)?,
    };
    Ok(if named { event.event("tasks") } else { event })
}

fn byte_event<T: Serialize + ?Sized>(data: &T, capacity: usize) -> Result<Event, String> {
    use std::io::Write as _;
    let mut event = Event::default().into_data_writer();
    {
        let mut writer = std::io::BufWriter::with_capacity(capacity, event.as_io_write());
        serde_json::to_writer(&mut writer, data).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
    }
    Ok(event.into_event())
}

async fn byte_contracts() {
    let raw = serde_json::value::RawValue::from_string(
        "{\r\n\"text\":\"Ω😀\",\n\"n\":null\n}".to_owned(),
    )
    .expect("pretty raw JSON");
    for capacity in [0, 1, 2, 3, 4, 7, 256, 8192] {
        let old = Event::default()
            .json_data(&raw)
            .expect("reference raw JSON");
        assert_eq!(
            frame_bytes(byte_event(&raw, capacity).expect("buffered raw JSON")).await,
            frame_bytes(old).await
        );
        let text = json!({"text": "aΩ😀中\r\n\0z", "empty": []});
        let old = Event::default().json_data(&text).expect("reference JSON");
        assert_eq!(
            frame_bytes(byte_event(&text, capacity).expect("buffered JSON")).await,
            frame_bytes(old).await
        );
    }
}

fn tasks(records: usize, shape: &str) -> Vec<BackgroundTask> {
    (0..records).map(|index| {
        let escaped = shape == "escaped";
        BackgroundTask {
            id: format!("00000000-0000-4000-8000-{index:012x}"),
            thread_id: if index + 1 == records { "http/sparse Ω" } else { "http-background" }.to_owned(),
            parent_task_id: None,
            kind: TaskKind::Shell,
            label: if escaped { format!("fixture-{index:06}\"\\\r\nΩ\0😀") } else { format!("fixture-{index:06}") },
            status: TaskStatus::Running,
            started_at: 1_700_000_000_000_i64 + i64::try_from(index).expect("bounded fixture index"),
            ended_at: None,
            exit_code: None,
            meta: if escaped { json!({"fixture_ordinal": index, "text": "\"\\\r\nΩ\0😀", "nested": [null, true, -1234567]}) }
                else { json!({"fixture_ordinal": index, "text": "private HTTP fixture Ω"}) },
            log_tail: match shape {
                "large-log" => "line Ω\r\n".repeat(8192),
                "large-ascii" => "x".repeat(65536),
                "large-unicode" => "Ω".repeat(32768),
                _ => String::new(),
            },
        }
    }).collect()
}

fn expected(tasks: &[BackgroundTask], named: bool) -> Vec<u8> {
    let mut frame = b"data: ".to_vec();
    frame.extend_from_slice(&serde_json::to_vec(tasks).expect("reference model JSON"));
    frame.push(b'\n');
    if named {
        frame.extend_from_slice(b"event: tasks\n");
    }
    frame.push(b'\n');
    frame
}

async fn frame_bytes(event: Event) -> Vec<u8> {
    let body = Sse::new(futures_util::stream::iter([Ok::<_, Infallible>(event)]))
        .into_response()
        .into_body();
    let frame = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("complete in-memory SSE body");
    frame.to_vec()
}

async fn validate(event: Event, expected: &[u8]) {
    assert_eq!(
        frame_bytes(event).await,
        expected,
        "every raw SSE byte, including escapes and named-field ordering"
    );
}

struct Failing;
impl Serialize for Failing {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(None)?;
        sequence.serialize_element("accepted prefix")?;
        Err(serde::ser::Error::custom("fixture serialization refused"))
    }
}

fn error_contract() {
    for named in [false, true] {
        let old =
            construct(&Failing, Method::JsonData, named).expect_err("partial serialization fails");
        for method in METHODS {
            let candidate =
                construct(&Failing, method, named).expect_err("partial serialization fails");
            assert_eq!(old, candidate);
        }
        assert_eq!(old, "fixture serialization refused");
    }
}

#[tokio::test]
async fn buffering_preserves_raw_frames_across_utf8_chunk_boundaries() {
    buffered::contracts().await;
    byte_contracts().await;
}

#[test]
fn every_encoder_propagates_a_partial_serialization_failure() {
    error_contract();
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    error_contract();
    buffered::contracts().await;
    byte_contracts().await;
    let reverse = std::env::args().nth(1).as_deref() == Some("reverse");
    let mut cases = Vec::new();
    for shape in ["ordinary", "escaped"] {
        for records in [0, 1, 2, 8, 32, 200, 10_001] {
            for named in [false, true] {
                cases.push((shape, records, named));
            }
        }
    }
    // One large existing-model log field exercises expansion and extra-buffer
    // lifetime without making every model in the large-count fixture enormous.
    for shape in ["large-log", "large-ascii", "large-unicode"] {
        for named in [false, true] {
            cases.push((shape, 1, named));
        }
    }
    let mut methods = METHODS;
    if reverse {
        cases.reverse();
        methods.reverse();
    }
    for (shape, records, named) in cases {
        let tasks = tasks(records, shape);
        let expected = expected(&tasks, named);
        let digest = format!("{:x}", Sha256::digest(&expected));
        for method in methods {
            validate(
                construct(&tasks, method, named).expect("warm reference event"),
                &expected,
            )
            .await;
            #[cfg(not(feature = "count-allocations"))]
            {
                let mut samples_ms = Vec::new();
                for index in 0..35 {
                    let start = Instant::now();
                    let event =
                        construct(black_box(&tasks), method, named).expect("event construction");
                    let elapsed = start.elapsed().as_secs_f64() * 1_000.0;
                    if index >= 5 {
                        samples_ms.push(elapsed);
                    }
                    // Finalization, full-byte validation, hashing and event drop
                    // are outside the event-construction interval.
                    validate(black_box(event), &expected).await;
                }
                println!(
                    "SSE_ENCODING {}",
                    json!({"shape": shape, "records": records, "named": named, "method": method, "samples_ms": samples_ms, "warmups": 5, "raw_frame_sha256": digest, "raw_frame_bytes": expected.len()})
                );
            }
            #[cfg(feature = "count-allocations")]
            {
                allocations::start();
                let event = construct(black_box(&tasks), method, named)
                    .expect("counted event construction");
                drop(black_box(event));
                let counts = allocations::finish();
                validate(
                    construct(&tasks, method, named).expect("unmeasured allocation validation"),
                    &expected,
                )
                .await;
                println!(
                    "SSE_ALLOC {}",
                    json!({"shape": shape, "records": records, "named": named, "method": method, "counts": counts, "raw_frame_sha256": digest, "raw_frame_bytes": expected.len()})
                );
            }
        }
    }
    println!(
        "SSE_CONTRACTS complete raw frames,UTF-8/CR/LF/quotes/NUL,empty arrays,named-field order andpartial-error propagation passed"
    );
}
