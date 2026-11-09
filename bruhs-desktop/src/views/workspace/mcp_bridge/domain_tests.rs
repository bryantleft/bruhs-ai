use super::super::load_desktop_state_from_path;
use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{BridgeRequest, BridgeResponse};
use bruhs_models::{AgentAccess, AgentDomain, WorkflowOperation};
use gpui::TestAppContext;

#[gpui::test]
async fn edited_document_or_draft_blocks_native_undo(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        for domain in [AgentDomain::Knowledge, AgentDomain::Inbox] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
        }
    });
    connect(&workspace, cx).await;
    for (index, operation) in [
        WorkflowOperation::CreateKnowledgeNote {
            title: "Document".into(),
            body: "Original".into(),
        },
        WorkflowOperation::DraftInboxMessage {
            recipient: "Ada".into(),
            body: "Original".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(matches!(
            call(
                &workspace,
                cx,
                BridgeRequest::Propose {
                    request_id: format!("edited-{index}"),
                    operation
                }
            )
            .await,
            BridgeResponse::Ok(_)
        ));
        for selector in ["desktop-start-workflow", "desktop-run-workflow-step"] {
            cx.run_until_parked();
            let button = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(button.center(), gpui::Modifiers::default());
        }
        workspace.update_in(cx, |w, _, cx| {
            if index == 0 {
                w.knowledge_documents[0].body_markdown = "User edit".into();
            } else {
                w.inbox_conversations[0].upsert_draft("User edit", 999);
            }
            w.persist_and_notify(cx);
        });
        cx.run_until_parked();
        let button = cx.debug_bounds("desktop-undo-workflow").unwrap();
        cx.simulate_click(button.center(), gpui::Modifiers::default());
        workspace.update_in(cx, |w, _, _| {
            assert!(w.automation_error.is_some());
            assert_eq!(
                w.automation_state.workflows.last().unwrap().status,
                bruhs_models::WorkflowStatus::Completed
            );
            if index == 0 {
                assert_eq!(w.knowledge_documents[0].body_markdown, "User edit");
            } else {
                assert_eq!(w.inbox_conversations[0].draft_body(), "User edit");
            }
        });
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn documents_and_inbox_drafts_persist_and_undo_through_real_ipc(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let directory =
        std::env::temp_dir().join(format!("bruhs-domain-workflows-{}", uuid::Uuid::new_v4()));
    let path = directory.join("state.json");
    workspace.update_in(cx, |w, _, _| {
        w.persistence_test_path = Some(path.clone());
        for domain in [AgentDomain::Knowledge, AgentDomain::Inbox] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Write, true);
        }
    });
    connect(&workspace, cx).await;
    for (index, operation) in [
        WorkflowOperation::CreateKnowledgeNote {
            title: "Reviewed knowledge".into(),
            body: "Exact document body".into(),
        },
        WorkflowOperation::DraftInboxMessage {
            recipient: "Ada".into(),
            body: "Exact draft body".into(),
        },
    ]
    .into_iter()
    .enumerate()
    {
        let domain = operation.domain();
        let BridgeResponse::Ok(proposal) = call(
            &workspace,
            cx,
            BridgeRequest::Propose {
                request_id: format!("domain-{index}"),
                operation,
            },
        )
        .await
        else {
            panic!("proposal")
        };
        for selector in ["desktop-start-workflow", "desktop-run-workflow-step"] {
            cx.condition(&workspace, |w, _| w.save_status.is_saved())
                .await;
            cx.run_until_parked();
            let button = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(button.center(), gpui::Modifiers::default());
        }
        cx.condition(&workspace, |w, _| w.save_status.is_saved())
            .await;
        let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
        let workflow = saved.automation_state.workflows.last().unwrap();
        assert_eq!(workflow.id, proposal["workflow_id"]);
        assert_eq!(workflow.status, bruhs_models::WorkflowStatus::Completed);
        let BridgeResponse::Ok(read) = call(
            &workspace,
            cx,
            BridgeRequest::Read {
                domain,
                query: String::new(),
                offset: 0,
            },
        )
        .await
        else {
            panic!("read")
        };
        assert_eq!(read["records"].as_array().unwrap().len(), 1);
        let receipt: serde_json::Value =
            serde_json::from_str(workflow.steps[0].target_snapshot.as_ref().unwrap()).unwrap();
        assert_eq!(receipt, read["records"][0]);
        match domain {
            AgentDomain::Knowledge => assert_eq!(
                saved.knowledge_documents[0].body_markdown,
                "Exact document body"
            ),
            AgentDomain::Inbox => {
                assert_eq!(
                    saved.inbox_conversations[0].draft_body(),
                    "Exact draft body"
                );
                assert_eq!(
                    saved.inbox_conversations[0].messages[0].send_state,
                    bruhs_models::MessageSendState::Draft
                );
            }
            _ => unreachable!(),
        }
        cx.run_until_parked();
        let undo = cx.debug_bounds("desktop-undo-workflow").unwrap();
        cx.simulate_click(undo.center(), gpui::Modifiers::default());
        cx.condition(&workspace, |w, _| w.save_status.is_saved())
            .await;
        let saved = load_desktop_state_from_path(&path).unwrap().unwrap();
        assert!(saved.knowledge_documents.is_empty());
        assert!(saved.inbox_conversations.is_empty());
        assert_eq!(
            saved.automation_state.workflows.last().unwrap().status,
            bruhs_models::WorkflowStatus::RolledBack
        );
    }
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
    std::fs::remove_dir_all(directory).unwrap();
}
