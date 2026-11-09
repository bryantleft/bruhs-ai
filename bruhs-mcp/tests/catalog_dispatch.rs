//! `tools/list` must never advertise a tool the active mode cannot dispatch.
#![expect(
    clippy::unwrap_used,
    reason = "clippy's test exemption does not cover shared helper functions"
)]
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{ChildStdin, ChildStdout};

async fn next(output: &mut Lines<BufReader<ChildStdout>>) -> Value {
    let line = tokio::time::timeout(Duration::from_secs(10), output.next_line())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

async fn send(input: &mut ChildStdin, request: Value) {
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
}

fn assert_dispatched(name: &str, reply: &Value) {
    // Unknown tools are -32602; an unconnected tool is an isError result.
    assert!(reply.get("error").is_none(), "{name} is unknown: {reply}");
    let text = reply["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("Unknown tool"), "{name}: {text}");
    assert!(!text.contains("not connected"), "{name}: {text}");
    assert!(
        !text.contains("requires the native workspace"),
        "{name}: {text}"
    );
}

#[tokio::test]
async fn every_standalone_tool_dispatches() {
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
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
    )
    .await;
    let catalog = next(&mut output).await;
    let tools = catalog["result"]["tools"].as_array().unwrap();
    assert!(!tools.is_empty());
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        send(
            &mut input,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}),
        )
        .await;
        assert_dispatched(name, &next(&mut output).await);
    }
    drop(input);
    assert!(child.wait().await.unwrap().success());
}

#[cfg(unix)]
#[tokio::test]
async fn every_bridge_tool_dispatches() {
    use bruhs_core::workspace_bridge::{BridgeResponse, BridgeSession};
    let (session, mut app) = BridgeSession::start().unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_bruhs-mcp"))
        .env("BRUHS_WORKSPACE_BRIDGE", session.connection_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap()).lines();
    send(
        &mut input,
        json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}),
    )
    .await;
    let catalog = next(&mut output).await;
    let tools = catalog["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    // Standalone-only tools are not advertised to a bridged client.
    for name in [
        "bruhs_list_threads",
        "bruhs_list_tasks",
        "bruhs_agent_audit",
    ] {
        assert!(!names.contains(&name), "{name}");
    }
    assert!(names.contains(&"bruhs_read_meeting") && names.contains(&"bruhs_propose_workflow"));
    for name in names {
        send(
            &mut input,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}),
        )
        .await;
        // Either the arguments are rejected at once, or the app is asked.
        let reply = loop {
            tokio::select! {
                reply = next(&mut output) => break reply,
                pending = app.recv() => {
                    let pending = pending.unwrap();
                    pending.respond.send(BridgeResponse::Ok(json!({"records":[]}))).unwrap();
                }
            }
        };
        assert_dispatched(name, &reply);
    }
    drop(input);
    assert!(child.wait().await.unwrap().success());
    drop(session);
}
