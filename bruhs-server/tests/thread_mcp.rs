//! Exercise the real host binary's thread MCP mode without a provider or an app.
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::Command,
};

#[tokio::test]
async fn host_thread_mcp_mode_has_clean_stdio_and_relays_one_call() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connection = json!({
        "port": listener.local_addr().unwrap().port(),
        "token": "synthetic-capability",
        "read_only": false,
    });
    let broker = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let length = usize::try_from(stream.read_u32().await.unwrap()).unwrap();
        let mut frame = vec![0; length];
        stream.read_exact(&mut frame).await.unwrap();
        let request: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(request["token"], "synthetic-capability");
        assert_eq!(
            request["operation"],
            json!({"op": "message", "thread_id": "abcd1234", "message": "Report back"})
        );
        let reply = br#"{"ok":true,"text":"Sent to thread abcd1234."}"#;
        stream
            .write_u32(u32::try_from(reply.len()).unwrap())
            .await
            .unwrap();
        stream.write_all(reply).await.unwrap();
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_bruhs-server"))
        .arg("--thread-mcp")
        .env("BRUHS_THREAD_BROKER", connection.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
            .await
            .unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).await.unwrap();
        let initialized: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(initialized["result"]["serverInfo"]["name"], "bruhs-threads");
        stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"message_thread\",\"arguments\":{\"thread_id\":\"abcd1234\",\"message\":\"Report back\"}}}\n").await.unwrap();
        line.clear();
        stdout.read_line(&mut line).await.unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], 2);
        assert_eq!(response["result"]["isError"], false);
        assert_eq!(
            response["result"]["content"][0]["text"],
            "Sent to thread abcd1234."
        );
        assert!(!line.contains("synthetic-capability"));
        stdin.shutdown().await.unwrap();
        drop(stdin);
        assert!(child.wait().await.unwrap().success());
        let mut extra = String::new();
        stdout.read_to_string(&mut extra).await.unwrap();
        assert!(extra.is_empty());
    })
    .await;
    if result.is_err() {
        let _ = child.kill().await;
    }
    result.unwrap();
    broker.await.unwrap();
}
