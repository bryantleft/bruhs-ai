//! Exercise the real host binary's stdio mode without a provider or OS credentials.
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::Command,
};

#[tokio::test]
async fn host_mcp_mode_has_clean_stdio_and_returns_only_operation_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let connection =
        json!({"port": listener.local_addr().unwrap().port(), "token": "synthetic-capability"});
    let broker = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let length = usize::try_from(stream.read_u32().await.unwrap()).unwrap();
        assert!(length < 65536);
        let mut frame = vec![0; length];
        stream.read_exact(&mut frame).await.unwrap();
        let request: Value = serde_json::from_slice(&frame).unwrap();
        assert_eq!(request["token"], "synthetic-capability");
        assert_eq!(request["operation"]["env_var"], "API_KEY");
        let result = br#"{"status":"completed","exit_code":0}"#;
        stream
            .write_u32(u32::try_from(result.len()).unwrap())
            .await
            .unwrap();
        stream.write_all(result).await.unwrap();
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_bruhs-server"))
        .arg("--secret-mcp")
        .env("BRUHS_SECRET_BROKER", connection.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n").await.unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).await.unwrap();
        let initialized: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(initialized["result"]["serverInfo"]["name"], "bruhs-secrets");
        stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"request_secret\",\"arguments\":{\"name\":\"Key\",\"reason\":\"Synthetic host test\",\"env_var\":\"API_KEY\",\"command\":[\"synthetic-command\"]}}}\n").await.unwrap();
        line.clear();
        stdout.read_line(&mut line).await.unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["result"]["isError"], false);
        let status: Value = serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(status, json!({"status":"completed","exit_code":0}));
        assert!(!line.contains("synthetic-capability"));
        stdin.shutdown().await.unwrap();
        drop(stdin);
        assert!(child.wait().await.unwrap().success());
        let mut extra = String::new();
        stdout.read_to_string(&mut extra).await.unwrap();
        assert!(extra.is_empty());
    }).await;
    if result.is_err() {
        let _ = child.kill().await;
    }
    result.unwrap();
    broker.await.unwrap();
}
