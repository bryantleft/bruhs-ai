use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn oversized_input_replies_before_delimiter_and_discards_embedded_commands() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bruhs-mcp"))
        .env_remove("BRUHS_WORKSPACE_BRIDGE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let oversized = vec![b' '; 64 * 1024 + 1];
    input.write_all(&oversized).await.unwrap();
    let reply: Value = serde_json::from_str(
        &tokio::time::timeout(Duration::from_secs(10), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reply["error"]["code"], -32600);
    assert!(reply["id"].is_null());

    // This valid command is still part of the rejected frame, not a new one.
    let embedded = json!({"jsonrpc":"2.0","id":99,"method":"ping"});
    let next = json!({"jsonrpc":"2.0","id":2,"method":"ping"});
    input
        .write_all(format!("{embedded}\n{next}\n").as_bytes())
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(
        &tokio::time::timeout(Duration::from_secs(10), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(reply["id"], 2);
    assert_eq!(reply["result"], json!({}));
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
async fn invalid_utf8_reports_parse_error_and_does_not_close_the_session() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bruhs-mcp"))
        .env_remove("BRUHS_WORKSPACE_BRIDGE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    input.write_all(b"\xff\n").await.unwrap();
    let ping = json!({"jsonrpc":"2.0","id":3,"method":"ping"});
    input
        .write_all(format!("{ping}\n").as_bytes())
        .await
        .unwrap();
    for (id, code) in [(Value::Null, Some(-32700)), (json!(3), None)] {
        let reply: Value = serde_json::from_str(
            &tokio::time::timeout(Duration::from_secs(10), output.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(reply["id"], id);
        assert_eq!(reply["error"]["code"].as_i64(), code);
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
async fn a_closed_stdout_pipe_ends_the_server_with_a_failure_status() {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bruhs-mcp"))
        .env_remove("BRUHS_WORKSPACE_BRIDGE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    // The client goes away: its read end of our stdout closes.
    drop(child.stdout.take().unwrap());
    let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    // The server may already have exited; a failed write is fine here.
    let _ = input.write_all(format!("{ping}\n").as_bytes()).await;
    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{:?}", output.status);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cannot write to stdout"), "{stderr}");
}
