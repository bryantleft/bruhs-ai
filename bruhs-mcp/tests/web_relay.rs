//! `bruhs-mcp` with a web session's connection file: the same tools, sent to
//! bruhs-server's relay (`/api/workspace-bridge/call`) with the session token.
//! The relay here is a minimal HTTP/1.1 stand-in that records each request.
#![cfg(unix)]

use bruhs_core::workspace_bridge::{
    new_secret, BridgeRequest, BridgeResponse, RelayConnectionFile,
};
use bruhs_models::workspace_bridge::PersonReadSection;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};

/// One request the stand-in relay received.
struct Received {
    path: String,
    authorization: Option<String>,
    request: BridgeRequest,
}

/// Answers each request with the next canned `(status, body)`.
async fn relay(answers: Vec<(u16, String)>) -> (String, mpsc::UnboundedReceiver<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("stand-in relay");
    let origin = format!("http://{}", listener.local_addr().expect("stand-in relay"));
    let (sender, receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        for (status, body) in answers {
            let (stream, _) = listener.accept().await.expect("stand-in relay");
            let mut stream = BufReader::new(stream);
            let mut head = String::new();
            loop {
                let mut line = String::new();
                stream.read_line(&mut line).await.expect("stand-in relay");
                if line == "\r\n" {
                    break;
                }
                head.push_str(&line);
            }
            let header = |name: &str| {
                head.lines().find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case(name)
                        .then(|| value.trim().to_owned())
                })
            };
            let length: usize = header("content-length")
                .expect("stand-in relay")
                .parse()
                .expect("stand-in relay");
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).await.expect("stand-in relay");
            sender
                .send(Received {
                    path: head
                        .split_whitespace()
                        .nth(1)
                        .expect("stand-in relay")
                        .to_owned(),
                    authorization: header("authorization"),
                    request: serde_json::from_slice(&bytes).expect("stand-in relay"),
                })
                .expect("stand-in relay");
            let reason = if status == 200 { "OK" } else { "Error" };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .expect("stand-in relay");
            stream.get_mut().shutdown().await.expect("stand-in relay");
        }
    });
    (origin, receiver)
}

#[tokio::test]
async fn tools_reach_a_web_session_through_the_relay_with_its_token() {
    let token = new_secret().unwrap();
    let answers = vec![
        (
            200,
            serde_json::to_string(&BridgeResponse::Ok(
                json!({"person_id":"p1","section":"observations","records":[],"next_offset":null,"observations_withheld":true}),
            ))
            .unwrap(),
        ),
        (
            401,
            serde_json::to_string(&BridgeResponse::Error(
                "Workspace connection is not authorized.".into(),
            ))
            .unwrap(),
        ),
        (504, "\"The workspace tab did not answer in time.\"".into()),
    ];
    let (origin, mut received) = relay(answers).await;
    let file = RelayConnectionFile::create(&origin, &token).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bruhs-mcp"))
        .env("BRUHS_WORKSPACE_BRIDGE", file.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    let ask = |id: u32, name: &str, arguments: Value| {
        format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
        )
    };

    // The catalog is the connected one, the same as for the native app.
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let catalog: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let names: Vec<&str> = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for tool in [
        "bruhs_read_person",
        "bruhs_propose_workflow",
        "bruhs_workflow_status",
        "bruhs_request_workflow_rollback",
    ] {
        assert!(names.contains(&tool), "{tool} missing from {names:?}");
    }
    assert!(!names.contains(&"bruhs_list_threads"));

    input
        .write_all(
            ask(
                1,
                "bruhs_read_person",
                json!({"person_id":"p1","section":"observations"}),
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(10), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.path, "/api/workspace-bridge/call");
    assert_eq!(first.authorization, Some(format!("Bearer {token}")));
    assert_eq!(
        first.request,
        BridgeRequest::ReadPerson {
            person_id: "p1".into(),
            section: PersonReadSection::Observations,
            offset: 0,
        }
    );
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    assert_ne!(reply["result"]["isError"], true);
    let page: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(page["observations_withheld"], true);

    // A revoked session's refusal reaches the model as the relay worded it.
    input
        .write_all(ask(2, "bruhs_workflow_status", json!({"workflow_id":"w1"})).as_bytes())
        .await
        .unwrap();
    assert!(matches!(
        received.recv().await.unwrap().request,
        BridgeRequest::Status { workflow_id } if workflow_id == "w1"
    ));
    let refused: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(refused["result"]["isError"], true);
    assert_eq!(
        refused["result"]["content"][0]["text"],
        "Workspace connection is not authorized."
    );

    // A timeout claims nothing about a submitted proposal.
    input
        .write_all(
            ask(
                3,
                "bruhs_write_tasks",
                json!({"operation":{"type":"create_task","title":"Draft"},"dry_run":false,"request_id":"r1"}),
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    assert!(matches!(
        received.recv().await.unwrap().request,
        BridgeRequest::Propose { request_id, .. } if request_id == "r1"
    ));
    let unknown: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(unknown["result"]["isError"], true);
    let text = unknown["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("reuse the same request_id"), "{text}");
    assert!(text.contains("No completion is claimed"), "{text}");

    drop(input);
    tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_web_connection_file_naming_another_host_is_refused_at_startup() {
    let directory = tempfile_directory();
    let path = directory.join("connection.json");
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(
            &mut file,
            json!({"url":"http://example.com:3001","token":new_secret().unwrap()})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    }
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_bruhs-mcp"))
        .env("BRUHS_WORKSPACE_BRIDGE", &path)
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unavailable or not private"), "{stderr}");
    assert!(!stderr.contains("example.com"), "the cause is not echoed");
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir(&directory).unwrap();
}

fn tempfile_directory() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "bruhs-mcp-web-relay-{}",
        new_secret().expect("scratch directory")
    ));
    std::fs::create_dir(&directory).expect("scratch directory");
    directory
}
