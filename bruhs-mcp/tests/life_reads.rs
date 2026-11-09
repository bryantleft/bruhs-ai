#![cfg(unix)]

use bruhs_core::workspace_bridge::{BridgeResponse, BridgeSession};
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn life_sections_schema_errors_and_exact_strings_cross_actual_stdio() {
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
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"tools/list\"}\n")
        .await
        .unwrap();
    let catalog: Value = serde_json::from_str(
        &tokio::time::timeout(Duration::from_secs(10), output.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    for (name, kind, sections) in [
        (
            "bruhs_read_finance",
            "read_finance",
            &[
                "accounts",
                "transactions",
                "budgets",
                "income_streams",
                "holdings",
            ][..],
        ),
        (
            "bruhs_read_health",
            "read_health",
            &["workouts", "sleep", "recovery", "habits"][..],
        ),
    ] {
        let matches: Vec<_> = catalog["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|tool| tool["name"] == name)
            .collect();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["inputSchema"]["additionalProperties"], false);
        assert_eq!(
            matches[0]["inputSchema"]["properties"]["section"]["enum"],
            json!(sections)
        );
        for section in sections {
            let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{"section":section,"query":"Local","offset":50}}});
            input
                .write_all(format!("{request}\n").as_bytes())
                .await
                .unwrap();
            let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
                .await
                .unwrap()
                .unwrap();
            let actual = serde_json::to_value(pending.request).unwrap();
            assert_eq!(
                actual,
                json!({"kind":kind,"section":section,"query":"Local","offset":50})
            );
            pending.respond.send(BridgeResponse::Ok(json!({"section":section,"records":[{"amount_minor":"-9007199254740993"}],"next_offset":null}))).unwrap();
            let reply: Value = serde_json::from_str(
                &tokio::time::timeout(Duration::from_secs(10), output.next_line())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
            )
            .unwrap();
            let content: Value =
                serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(content["records"][0]["amount_minor"], "-9007199254740993");
        }
        let request = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":{"allow_agent_context":true}}});
        input
            .write_all(format!("{request}\n").as_bytes())
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
        assert_eq!(reply["result"]["isError"], true);
        assert!(app.try_recv().is_err());
        let request = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":{}}});
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
            .await
            .unwrap()
            .unwrap();
        pending
            .respond
            .send(BridgeResponse::Error(
                "Life agent context is disabled.".into(),
            ))
            .unwrap();
        let reply: Value = serde_json::from_str(
            &tokio::time::timeout(Duration::from_secs(10), output.next_line())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(reply["result"]["isError"], true);
        assert_eq!(
            reply["result"]["content"][0]["text"],
            "Life agent context is disabled."
        );
    }
    drop(input);
    assert!(tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
}
