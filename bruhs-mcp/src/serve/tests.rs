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
fn an_unserializable_response_becomes_an_internal_error_frame_never_an_empty_one() {
    struct Unserializable;
    impl Serialize for Unserializable {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("no"))
        }
    }
    let frame = encode_frame(&Unserializable, &Some(json!(12)));
    assert_eq!(frame.last(), Some(&b'\n'));
    let reply: Value = serde_json::from_slice(&frame).unwrap();
    assert_eq!(reply["id"], 12);
    assert_eq!(reply["error"]["code"], -32603);
}

#[tokio::test]
async fn write_frame_emits_one_newline_terminated_json_line() {
    let mut output = Vec::new();
    write_frame(
        &mut output,
        &JsonRpcResponse::success(Some(json!(1)), json!({})),
    )
    .await
    .unwrap();
    assert_eq!(output, b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
}
