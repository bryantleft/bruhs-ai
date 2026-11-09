//! User-controlled session boundary; never restored from the workspace file.
use super::*;

#[cfg(all(test, unix))]
mod batch_tests;
#[cfg(all(test, unix))]
mod chat_read_tests;
#[cfg(all(test, unix))]
mod chat_tests;
#[cfg(all(test, unix))]
mod contact_tests;
#[cfg(all(test, unix))]
mod domain_tests;
#[cfg(all(test, unix))]
mod history_tests;
#[cfg(all(test, unix))]
mod life_read_tests;
#[cfg(all(test, unix))]
mod life_tests;
#[cfg(test)]
mod meeting_action_tests;
#[cfg(all(test, unix))]
mod meeting_note_tests;
#[cfg(all(test, unix))]
mod meeting_read_tests;
#[cfg(test)]
mod meeting_stream_tests;
#[cfg(all(test, unix))]
mod people_read_tests;
#[cfg(all(test, unix))]
mod people_tests;
#[cfg(all(test, unix))]
mod person_merge_tests;
#[cfg(unix)]
mod requests;
mod rollback_review;
#[cfg(all(test, unix))]
mod rollback_tests;
#[cfg(test)]
mod transcript_view_tests;
use rollback_review::RollbackReview;
#[cfg(all(test, unix))]
mod tests;

#[cfg(unix)]
const MCP_SESSION_ACTIVE_NOTE: &str =
    "Session active. Granted reads cover Chat summaries/completed messages, tasks, local \
    events, captures, documents, inbox, and People identity/relationship context. Chat \
    drafts and voice fingerprints stay private. Finance and Health reads also require their \
    Life agent-context opt-in; Finance monetary fields and quantities require the separate \
    exact-amount opt-in. Identity labels remain visible; transaction notes stay private. \
    Writes require your review and start. Chat creates empty threads; Inbox creates drafts, \
    never sends. People adds undated local reminders. Life writes create manual records, \
    never payments or provider changes. Disabling disconnects all clients.";
#[cfg(unix)]
const MCP_OFF_NOTE: &str =
    "Off by default. Enable only for a trusted local MCP client. Read grants cover Chat \
    summaries/completed messages, Tasks, Calendar, Capture, Knowledge, Inbox, and People \
    identity/relationship context. Chat drafts and voice fingerprints stay private. Finance \
    and Health reads also require their Life agent-context opt-in; Finance monetary fields \
    and quantities require the separate exact-amount opt-in. Identity labels remain visible; \
    transaction notes stay private. Write proposals support empty Chat threads, manual \
    Finance entries, Health workouts, and undated People reminders. Read and Write grants \
    stay independent.";
/// The `mcpServers` entry a client pastes to reach this session.
#[cfg(unix)]
fn mcp_client_config(connection_path: impl serde::Serialize) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "bruhs": {
                "command": "bruhs-mcp",
                "env": { "BRUHS_WORKSPACE_BRIDGE": connection_path }
            }
        }
    })
}

#[cfg(unix)]
const MCP_CONNECTION_FAILED_NOTE: &str =
    "The local connection is unavailable. Access is disabled; you can retry.";

fn mcp_bridge_panel() -> Div {
    div()
        .mb_3()
        .p_3()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme::longan_800())
        .bg(theme::longan_900())
        .text_size(px(12.0))
        .text_color(theme::rambutan_300())
}

#[derive(Default)]
pub(super) struct WorkspaceBridge {
    rollback_reviews: std::collections::HashMap<String, RollbackReview>,
    #[cfg(unix)]
    state: ConnectionState,
    #[cfg(unix)]
    worker: Option<gpui::Task<()>>,
    #[cfg(unix)]
    proposals:
        std::collections::HashMap<String, (String, bruhs_core::workspace_bridge::WorkflowProposal)>,
}

#[cfg(unix)]
#[derive(Default)]
enum ConnectionState {
    #[default]
    Disabled,
    Starting(String),
    Connected {
        id: String,
        session: bruhs_core::workspace_bridge::BridgeSession,
    },
    Failed,
}

impl WorkspaceBridge {
    pub(super) fn pending_review(&self, state: &AutomationState) -> bool {
        self.pending_review_id(state).is_some()
    }

    pub(super) fn pending_review_id<'a>(&'a self, state: &AutomationState) -> Option<&'a str> {
        if let Some((id, _)) = self.rollback_reviews.iter().find(|(id, review)| {
            **review == RollbackReview::Pending
                && state.workflows.iter().any(|workflow| {
                    workflow.id == **id
                        && workflow.status != bruhs_models::WorkflowStatus::RolledBack
                })
        }) {
            return Some(id.as_str());
        }
        self.pending_proposal_id(state)
    }

    /// A proposal waiting for its preview to be reviewed.
    #[cfg(unix)]
    fn pending_proposal_id<'a>(&'a self, state: &AutomationState) -> Option<&'a str> {
        self.proposals
            .values()
            .find(|(id, _)| {
                state.workflows.iter().any(|workflow| {
                    workflow.id == *id && workflow.status == bruhs_models::WorkflowStatus::Previewed
                })
            })
            .map(|(id, _)| id.as_str())
    }

    /// Proposals need the local socket, which only unix builds open.
    #[cfg(not(unix))]
    fn pending_proposal_id<'a>(&'a self, _state: &AutomationState) -> Option<&'a str> {
        None
    }
}

impl Workspace {
    #[cfg(unix)]
    fn enable_mcp_bridge(&mut self, cx: &mut Context<'_, Self>) {
        if matches!(
            self.mcp_bridge.state,
            ConnectionState::Starting(_) | ConnectionState::Connected { .. }
        ) {
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        self.mcp_bridge.state = ConnectionState::Starting(id.clone());
        let (sender, receiver) = tokio::sync::oneshot::channel();
        cx.global::<DesktopState>().runtime.spawn(async move {
            let _ = sender.send(bruhs_core::workspace_bridge::BridgeSession::start());
        });
        cx.spawn(async move |this, cx| {
            let result = receiver.await;
            let _ = this.update(cx, |this, cx| {
                if !matches!(&this.mcp_bridge.state, ConnectionState::Starting(current) if current == &id) { return; }
                match result {
                    Ok(Ok((session, mut requests))) => {
                        this.mcp_bridge.state = ConnectionState::Connected { id: id.clone(), session };
                        this.mcp_bridge.worker = Some(cx.spawn(async move |this, cx| {
                            while let Some(pending) = requests.recv().await {
                                if pending.respond.is_closed() { continue; }
                                let result = this.update(cx, |this, cx| {
                                    if !matches!(&this.mcp_bridge.state, ConnectionState::Connected { id: current, .. } if current == &id) { return; }
                                    let reply = this.handle_bridge_request(pending.request, cx);
                                    let _ = pending.respond.send(reply);
                                });
                                if result.is_err() { break; }
                            }
                            let _ = this.update(cx, |this, cx| {
                                if matches!(&this.mcp_bridge.state, ConnectionState::Connected { id: current, .. } if current == &id) {
                                    this.mcp_bridge.state = ConnectionState::Failed;
                                    this.mcp_bridge.proposals.clear();
                                    this.mcp_bridge.rollback_reviews.clear();
                                    cx.notify();
                                }
                            });
                        }));
                    }
                    _ => this.mcp_bridge.state = ConnectionState::Failed,
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    #[cfg(not(unix))]
    pub(super) fn render_mcp_bridge(&self, _cx: &Context<'_, Self>) -> Div {
        mcp_bridge_panel().child("Local MCP connection is not available on this platform yet.")
    }

    #[cfg(unix)]
    pub(super) fn render_mcp_bridge(&self, cx: &Context<'_, Self>) -> Div {
        let panel = mcp_bridge_panel();
        let connected = matches!(self.mcp_bridge.state, ConnectionState::Connected { .. });
        let starting = matches!(self.mcp_bridge.state, ConnectionState::Starting(_));
        panel
            .child("MCP workspace connection")
            .child(
                div()
                    .mt_1()
                    .text_color(theme::rambutan_500())
                    .child(if connected {
                        MCP_SESSION_ACTIVE_NOTE
                    } else {
                        MCP_OFF_NOTE
                    }),
            )
            .when(
                matches!(self.mcp_bridge.state, ConnectionState::Failed),
                |panel| {
                    panel.child(
                        div()
                            .id("mcp-connection-error")
                            .role(Role::Alert)
                            .child(MCP_CONNECTION_FAILED_NOTE),
                    )
                },
            )
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        web_button(
                            "mcp-connection-toggle",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .debug_selector(|| "desktop-mcp-connection-toggle".into())
                        .aria_label(if connected || starting {
                            "Disable MCP connection"
                        } else {
                            "Enable MCP connection for this session"
                        })
                        .child(if connected || starting {
                            "Disable connection"
                        } else {
                            "Enable for this session"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if connected || starting {
                                this.mcp_bridge = WorkspaceBridge::default();
                                cx.notify();
                            } else {
                                this.enable_mcp_bridge(cx);
                            }
                        })),
                    )
                    .when(connected, |row| {
                        row.child(
                            web_button(
                                "mcp-copy-config",
                                WebButtonVariant::Outline,
                                WebButtonSize::Small,
                                ButtonState::Enabled,
                            )
                            .aria_label("Copy MCP client configuration")
                            .child("Copy MCP config")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let ConnectionState::Connected { session, .. } =
                                    &this.mcp_bridge.state
                                {
                                    let config = mcp_client_config(session.connection_path());
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        config.to_string(),
                                    ));
                                }
                            })),
                        )
                    }),
            )
    }
}
