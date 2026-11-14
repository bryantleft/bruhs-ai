use super::*;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, BufReader, ReadBuf};

fn standalone() -> Backend {
    Backend::Standalone(Arc::new(AppState::new()))
}

/// Serves `requests` (one per line) and returns every response frame.
async fn exchange(requests: &[&str]) -> Vec<Value> {
    let mut input = requests.join("\n").into_bytes();
    input.push(b'\n');
    let mut output = Vec::new();
    serve(
        StdioInput::new(BufReader::new(input.as_slice())),
        &mut output,
        &standalone(),
    )
    .await
    .unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

async fn one(request: &str) -> Value {
    let mut replies = exchange(&[request]).await;
    assert_eq!(replies.len(), 1, "{replies:?}");
    replies.remove(0)
}

#[tokio::test]
async fn an_invalid_jsonrpc_version_is_an_invalid_request() {
    for request in [
        r#"{"jsonrpc":"1.0","id":4,"method":"ping"}"#,
        r#"{"id":4,"method":"ping"}"#,
    ] {
        let reply = one(request).await;
        assert_eq!(reply["error"]["code"], -32600, "{request}");
        assert_eq!(reply["id"], 4);
        assert!(reply.get("result").is_none());
    }
}

#[tokio::test]
async fn a_tool_call_without_a_name_is_invalid_params() {
    for request in [
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":5}}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{}}}"#,
    ] {
        let reply = one(request).await;
        assert_eq!(reply["error"]["code"], -32602, "{request}");
        assert_eq!(reply["id"], 1);
    }
}

#[tokio::test]
async fn an_unknown_tool_is_invalid_params_but_bad_arguments_are_tool_errors() {
    let reply = one(
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bruhs_send_message","arguments":{}}}"#,
    )
    .await;
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown tool"));

    // One rule for every kind of bad argument: missing, malformed, oversized.
    for arguments in [
        json!({}),
        json!({"thread_id": 3}),
        json!({"thread_id": ""}),
        json!({"thread_id": "x".repeat(200)}),
    ] {
        let request = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bruhs_list_tasks","arguments":arguments}});
        let reply = one(&request.to_string()).await;
        assert!(reply.get("error").is_none(), "{arguments}: {reply}");
        assert_eq!(reply["result"]["isError"], true, "{arguments}");
        assert_eq!(reply["result"]["content"][0]["type"], "text");
    }
}

#[tokio::test]
async fn initialize_reports_this_crates_version() {
    let reply = one(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).await;
    assert_eq!(
        reply["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::test]
async fn notifications_and_blank_lines_get_no_reply_and_unknown_methods_are_not_found() {
    let replies = exchange(&[
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "   ",
        r#"{"jsonrpc":"2.0","id":9,"method":"nope"}"#,
    ])
    .await;
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["error"]["code"], -32601);
}

#[tokio::test]
async fn standalone_tools_list_is_the_standalone_catalog() {
    let reply = one(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).await;
    let names: Vec<&str> = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"bruhs_read_chat") && names.contains(&"bruhs_list_threads"));
    assert!(!names.contains(&"bruhs_read_meeting") && !names.contains(&"bruhs_send_message"));
}

/// A writer standing in for a closed pipe.
struct ClosedPipe;

impl AsyncWrite for ClosedPipe {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, _: &[u8]) -> Poll<io::Result<usize>> {
        Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn a_write_to_a_closed_pipe_ends_the_loop_with_an_error() {
    // Two requests are queued; the loop must stop at the first failed write
    // rather than carry on answering into the void.
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
    let result = serve(
        StdioInput::new(BufReader::new(input.as_slice())),
        &mut ClosedPipe,
        &standalone(),
    )
    .await;
    assert!(
        matches!(result, Err(ServeError::Write(ref e)) if e.kind() == io::ErrorKind::BrokenPipe),
        "{result:?}"
    );
}

struct BrokenReader;

impl AsyncRead for BrokenReader {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("stdin failed")))
    }
}

#[tokio::test]
async fn a_stdin_read_error_ends_the_loop_with_an_error() {
    let mut output = Vec::new();
    let result = serve(
        StdioInput::new(BufReader::new(BrokenReader)),
        &mut output,
        &standalone(),
    )
    .await;
    assert!(matches!(result, Err(ServeError::Read(_))), "{result:?}");
    assert!(output.is_empty());
}

#[tokio::test]
async fn end_of_input_is_a_clean_exit() {
    let mut output = Vec::new();
    serve(
        StdioInput::new(BufReader::new(&b""[..])),
        &mut output,
        &standalone(),
    )
    .await
    .unwrap();
    assert!(output.is_empty());
}

#[tokio::test]
async fn an_oversized_frame_is_answered_and_the_loop_continues() {
    let mut input = vec![b'x'; crate::stdio_input::MAX_REQUEST_BYTES + 10];
    input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n");
    let mut output = Vec::new();
    serve(
        StdioInput::new(BufReader::new(input.as_slice())),
        &mut output,
        &standalone(),
    )
    .await
    .unwrap();
    let replies: Vec<Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies[0]["error"]["code"], -32600);
    assert_eq!(replies[1]["id"], 1);
}

#[test]
fn a_partly_serialized_response_becomes_one_complete_internal_error_frame() {
    struct Unserializable;
    impl Serialize for Unserializable {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            use serde::ser::SerializeMap;
            let mut map = serializer.serialize_map(Some(2))?;
            map.serialize_entry("partial_response", &"never emit this prefix".repeat(1024))?;
            Err(serde::ser::Error::custom("no"))
        }
    }
    let mut frame = b"previous response must also be removed".to_vec();
    encode_frame(&Unserializable, &Some(json!(12)), &mut frame);
    assert_eq!(frame.last(), Some(&b'\n'));
    let reply: Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(reply["id"], 12);
    assert_eq!(reply["error"]["code"], -32603);
    assert!(reply.get("partial_response").is_none());
    encode_frame(&json!({"next":"complete"}), &None, &mut frame);
    assert_eq!(frame, b"{\"next\":\"complete\"}\n");
}

#[tokio::test]
async fn write_frame_emits_one_newline_terminated_json_line() {
    let mut output = Vec::new();
    let mut buffer = Vec::new();
    write_frame(
        &mut output,
        &JsonRpcResponse::success(Some(json!(1)), json!({})),
        &mut buffer,
    )
    .await
    .unwrap();
    assert_eq!(output, b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
}

#[tokio::test]
async fn repeated_frames_keep_the_buffer_allocation_and_replace_complete_output() {
    let response = JsonRpcResponse::success(
        Some(json!("call\n1")),
        json!({"text":"Café ΟΣ 👩🏽‍💻 \"quoted\"\n\\\0".repeat(32768)}),
    );
    let mut expected = serde_json::to_vec(&response).unwrap();
    expected.push(b'\n');
    let mut output = Vec::new();
    let mut buffer = Vec::new();
    write_frame(&mut output, &response, &mut buffer)
        .await
        .unwrap();
    let allocation = buffer.as_ptr();
    let capacity = buffer.capacity();
    for _ in 0..3 {
        output.clear();
        write_frame(&mut output, &response, &mut buffer)
            .await
            .unwrap();
        assert_eq!(output, expected);
        assert_eq!(buffer.as_ptr(), allocation);
        assert_eq!(buffer.capacity(), capacity);
    }
    output.clear();
    write_frame(
        &mut output,
        &JsonRpcResponse::success(Some(json!(2)), json!({})),
        &mut buffer,
    )
    .await
    .unwrap();
    assert_eq!(output, b"{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n");
    assert_eq!(buffer.as_ptr(), allocation);
    assert_eq!(buffer.capacity(), capacity);
}

#[test]
fn consumed_tool_text_retains_only_one_largest_empty_allocation() {
    use crate::protocol::{ContentType, ToolContent, ToolResult};

    let mut text = String::with_capacity(1024 * 1024);
    text.push_str("previous private response ΟΣ");
    let allocation = text.as_ptr();
    let capacity = text.capacity();
    let mut result = ToolResult::ok(text);
    result.content.push(ToolContent {
        content_type: ContentType::Text,
        text: "small block".into(),
    });
    let mut buffer = Vec::new();
    recycle_text(
        JsonRpcResponse::success(Some(json!(1)), result.into_value()),
        &mut buffer,
    );
    assert!(buffer.is_empty());
    assert_eq!(buffer.as_ptr(), allocation);
    assert_eq!(buffer.capacity(), capacity);
    for result in [json!({}), ToolResult::err("small error").into_value()] {
        recycle_text(JsonRpcResponse::success(None, result), &mut buffer);
        assert!(buffer.is_empty());
        assert_eq!(buffer.as_ptr(), allocation);
        assert_eq!(buffer.capacity(), capacity);
    }
}

#[tokio::test]
async fn a_flush_failure_stops_before_writing_the_next_response() {
    #[derive(Default)]
    struct FlushFailure {
        bytes: Vec<u8>,
        flushes: usize,
    }
    impl AsyncWrite for FlushFailure {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<io::Result<usize>> {
            self.bytes.extend_from_slice(bytes);
            Poll::Ready(Ok(bytes.len()))
        }
        fn poll_flush(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.flushes += 1;
            Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
    let mut output = FlushFailure::default();
    let result = serve(
        StdioInput::new(BufReader::new(input.as_slice())),
        &mut output,
        &standalone(),
    )
    .await;
    assert!(
        matches!(result, Err(ServeError::Write(ref e)) if e.kind() == io::ErrorKind::BrokenPipe)
    );
    assert_eq!(output.flushes, 1);
    assert_eq!(
        output.bytes,
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n"
    );
}

#[test]
#[ignore = "manual optimized frame-encoding comparison; no I/O or providers"]
fn large_response_frame_encoding_cost() {
    use std::hint::black_box;
    use std::time::Instant;

    let pattern = "line Café ΟΣ 👩🏽‍💻 \"quoted\"\n\\\0";
    for target_bytes in [1024_usize, 64 * 1024, 1024 * 1024, 8 * 1024 * 1024] {
        let response = JsonRpcResponse::success(
            Some(json!(1)),
            json!({"content":[{"type":"text","text":pattern.repeat(target_bytes.div_ceil(pattern.len()))}]}),
        );
        let mut expected = serde_json::to_vec(&response).unwrap();
        expected.push(b'\n');
        for (order, versions) in [
            ("forward", ["allocate", "reuse"]),
            ("reverse", ["reuse", "allocate"]),
        ] {
            for version in versions {
                let mut buffer = Vec::new();
                let mut samples = Vec::new();
                for iteration in 0..35 {
                    let start = Instant::now();
                    if version == "allocate" {
                        let mut frame = serde_json::to_vec(black_box(&response)).unwrap();
                        frame.push(b'\n');
                        drop(black_box(frame));
                    } else {
                        encode_frame(black_box(&response), &response.id, &mut buffer);
                        black_box(buffer.as_slice());
                    }
                    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                    if iteration >= 5 {
                        samples.push(elapsed_ms);
                    }
                }
                encode_frame(&response, &response.id, &mut buffer);
                assert_eq!(buffer, expected);
                println!(
                    "MCP_FRAME_COST {}",
                    json!({"target_text_bytes":target_bytes,"encoded_bytes":expected.len(),"order":order,"version":version,"samples_ms":samples,"warmups":5,"scope":"Encoding only. Allocating variant includes frame destruction; reused variant retains capacity across requests. Response construction, I/O and cold first allocation excluded."})
                );
            }
        }
    }
}
