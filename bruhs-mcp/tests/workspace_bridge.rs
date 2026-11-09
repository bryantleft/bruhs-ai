#![cfg(unix)]

use bruhs_core::workspace_bridge::{
    BridgeRequest, BridgeResponse, BridgeSession, MeetingReadSection, PersonReadSection,
};
use bruhs_models::AgentDomain;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test]
async fn meeting_tool_schema_and_exact_section_cross_the_real_stdio_bridge() {
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
    let listing = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
    input
        .write_all(format!("{listing}\n").as_bytes())
        .await
        .unwrap();
    let response: Value =
        serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let tool = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_read_meeting")
        .unwrap();
    assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        tool["inputSchema"]["properties"]["section"]["enum"],
        json!(["summary", "transcript"])
    );
    let read = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bruhs_read_meeting","arguments":{"meeting_id":"exact-meeting","section":"transcript","offset":50}}});
    input
        .write_all(format!("{read}\n").as_bytes())
        .await
        .unwrap();
    let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(pending.request, BridgeRequest::ReadMeeting { meeting_id, section: MeetingReadSection::Transcript, offset: 50 } if meeting_id == "exact-meeting")
    );
    pending.respond.send(BridgeResponse::Ok(json!({"meeting_id":"exact-meeting","source":"workspace_cache","records":[{"text":"Actual selected page"}],"next_offset":null}))).unwrap();
    let response: Value =
        serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let content: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["meeting_id"], "exact-meeting");
    assert_eq!(content["records"][0]["text"], "Actual selected page");
    drop(input);
    tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn stdio_rollback_tool_requests_review_without_an_execution_command() {
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
    let catalog: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let tool = catalog["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_request_workflow_rollback")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["dry_run"]["default"],
        true
    );
    assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    for dry_run in [true, false] {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bruhs_request_workflow_rollback","arguments":{"workflow_id":"owned","dry_run":dry_run}}});
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(pending.request, BridgeRequest::RequestRollback { workflow_id, dry_run: preview } if workflow_id == "owned" && preview == dry_run)
        );
        pending
            .respond
            .send(BridgeResponse::Ok(
                json!({"rolled_back":false,"requires_user_approval":true}),
            ))
            .unwrap();
        let reply: Value =
            serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
        let result: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(result["rolled_back"], false);
        assert_eq!(result["requires_user_approval"], true);
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
async fn stdio_batch_tool_preserves_preview_and_proposal_boundaries() {
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
    let catalog = json!({"jsonrpc":"2.0","id":0,"method":"tools/list"});
    input
        .write_all(format!("{catalog}\n").as_bytes())
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let tool = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_propose_workflow")
        .unwrap();
    assert_eq!(
        tool["inputSchema"]["properties"]["operations"]["items"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        10
    );
    let note_tool = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_write_meetings")
        .unwrap();
    let note_schema = &note_tool["inputSchema"]["properties"]["operation"];
    assert_eq!(
        note_schema["properties"]["type"]["const"],
        "create_meeting_note"
    );
    assert_eq!(note_schema["properties"]["body"]["default"], "");
    assert_eq!(note_schema["additionalProperties"], false);
    for name in ["bruhs_read_meeting_notes", "bruhs_read_meeting_note"] {
        let tool = reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        assert_eq!(tool["inputSchema"]["additionalProperties"], false);
    }
    let chat_tool = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_write_chat")
        .unwrap();
    assert_eq!(
        chat_tool["inputSchema"]["properties"]["operation"]["properties"]["type"]["const"],
        "create_chat_thread"
    );
    assert_eq!(
        chat_tool["inputSchema"]["properties"]["operation"]["additionalProperties"],
        false
    );
    let history_tool = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_read_chat_messages")
        .unwrap();
    assert_eq!(
        history_tool["inputSchema"]["required"],
        json!(["thread_id"])
    );
    assert_eq!(history_tool["inputSchema"]["additionalProperties"], false);
    let person_tool = reply["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "bruhs_read_person")
        .unwrap();
    assert_eq!(person_tool["inputSchema"]["required"], json!(["person_id"]));
    assert_eq!(
        person_tool["inputSchema"]["properties"]["section"]["enum"],
        json!(["profile", "observations", "relationship_items"])
    );
    assert_eq!(person_tool["inputSchema"]["additionalProperties"], false);
    for dry_run in [true, false] {
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bruhs_propose_workflow","arguments":{
            "title":"One review", "request_id":"same-plan", "dry_run":dry_run,
            "operations":[{"type":"create_task","title":"First"},{"type":"add_capture","content":"Second"},{"type":"add_finance_transaction","merchant":"Receipt","category":"Manual","amount_minor":"-9007199254740993"},{"type":"log_workout","activity":"Walk","minutes":37},{"type":"create_chat_thread","title":"Reviewed chat 🗒"},{"type":"create_person_reminder","person_id":"person-1","person":"Alex","reminder":"Send brief"}]
        }}});
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
            .await
            .unwrap()
            .unwrap();
        let proposal = match pending.request {
            BridgeRequest::PreviewWorkflow { proposal } if dry_run => proposal,
            BridgeRequest::ProposeWorkflow {
                request_id,
                proposal,
            } if !dry_run => {
                assert_eq!(request_id, "same-plan");
                proposal
            }
            _ => panic!("Unexpected request or execution command"),
        };
        assert_eq!(proposal.title, "One review");
        assert_eq!(proposal.operations[0].domain(), AgentDomain::Tasks);
        assert_eq!(proposal.operations[1].domain(), AgentDomain::Capture);
        assert!(matches!(
            &proposal.operations[2],
            bruhs_models::WorkflowOperation::AddFinanceTransaction {
                amount_minor: -9_007_199_254_740_993,
                ..
            }
        ));
        assert!(matches!(
            &proposal.operations[3],
            bruhs_models::WorkflowOperation::LogWorkout { minutes: 37, .. }
        ));
        assert_eq!(proposal.operations.len(), 6);
        assert!(matches!(
            &proposal.operations[5],
            bruhs_models::WorkflowOperation::CreatePersonReminder { person_id, person, reminder }
                if person_id == "person-1" && person == "Alex" && reminder == "Send brief"
        ));
        assert!(matches!(
            &proposal.operations[4],
            bruhs_models::WorkflowOperation::CreateChatThread { title } if title == "Reviewed chat 🗒"
        ));
        pending
            .respond
            .send(BridgeResponse::Ok(
                json!({"executed":false,"requires_user_approval":true}),
            ))
            .unwrap();
        let reply: Value =
            serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
        let result: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(result["executed"], false);
    }
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}

#[tokio::test]
async fn stdio_process_forwards_to_authenticated_workspace_without_local_fallback() {
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
    let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"bruhs_read_tasks","arguments":{"query":"synthetic"}}});
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(pending.request, BridgeRequest::Read { domain: AgentDomain::Tasks, query, offset: 0 } if query == "synthetic")
    );
    pending
        .respond
        .send(BridgeResponse::Ok(
            json!({"records":[{"id":"app-record","title":"synthetic actual app response"}]}),
        ))
        .unwrap();
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["id"], 1);
    let content: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["records"][0]["id"], "app-record");

    let history = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"bruhs_read_chat_messages","arguments":{"thread_id":"app-chat","offset":50}}});
    input
        .write_all(format!("{history}\n").as_bytes())
        .await
        .unwrap();
    let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(pending.request, BridgeRequest::ReadChatMessages { thread_id, offset: 50 } if thread_id == "app-chat")
    );
    pending.respond.send(BridgeResponse::Ok(json!({"thread_id":"app-chat","records":[{"role":"user","content":"actual page"}],"next_offset":null}))).unwrap();
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let content: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["records"][0]["content"], "actual page");
    assert_eq!(content["thread_id"], "app-chat");

    let person = json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"bruhs_read_person","arguments":{"person_id":"exact-person","section":"observations","offset":50}}});
    input
        .write_all(format!("{person}\n").as_bytes())
        .await
        .unwrap();
    let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(pending.request, BridgeRequest::ReadPerson { person_id, section: PersonReadSection::Observations, offset: 50 } if person_id == "exact-person")
    );
    pending.respond.send(BridgeResponse::Ok(json!({"person_id":"exact-person","section":"observations","records":[{"text":"Actual People context"}],"next_offset":null}))).unwrap();
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    let content: Value =
        serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(content["records"][0]["text"], "Actual People context");
    assert_eq!(content["person_id"], "exact-person");

    for (name, arguments) in [
        (
            "bruhs_read_meeting_notes",
            json!({"query":"Plan", "offset":50}),
        ),
        ("bruhs_read_meeting_note", json!({"note_id":"exact-note"})),
    ] {
        let request = json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":name,"arguments":arguments}});
        input
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        let pending = tokio::time::timeout(Duration::from_secs(10), app.recv())
            .await
            .unwrap()
            .unwrap();
        match pending.request {
            BridgeRequest::ReadMeetingNotes { query, offset } => {
                assert_eq!(query, "Plan");
                assert_eq!(offset, 50);
            }
            BridgeRequest::ReadMeetingNote { note_id } => assert_eq!(note_id, "exact-note"),
            other => panic!("Unexpected note request: {other:?}"),
        }
        pending
            .respond
            .send(BridgeResponse::Ok(
                json!({"records":[{"id":"exact-note", "body":"Exact 🗒\ntext"}]}),
            ))
            .unwrap();
        let reply: Value =
            serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
        let content: Value =
            serde_json::from_str(reply["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(content["records"][0]["body"], "Exact 🗒\ntext");
    }

    let grant = json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"bruhs_set_agent_capability","arguments":{"domain":"tasks","access":"write","allowed":true}}});
    input
        .write_all(format!("{grant}\n").as_bytes())
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    // There is no such tool: the request is invalid, and nothing reached the app.
    assert_eq!(reply["error"]["code"], -32602);
    assert!(reply.get("result").is_none());
    assert!(app.try_recv().is_err());

    drop(session);
    input
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let reply: Value = serde_json::from_str(&output.next_line().await.unwrap().unwrap()).unwrap();
    assert_eq!(reply["result"]["isError"], true);
    child.kill().await.unwrap();
    child.wait().await.unwrap();
}
