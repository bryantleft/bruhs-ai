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
mod context_read_tests;
#[cfg(all(test, unix))]
mod domain_tests;
#[cfg(all(test, unix))]
mod generic_read_tests;
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
#[cfg(test)]
mod people_page_tests;
#[cfg(all(test, unix))]
mod people_read_tests;
#[cfg(all(test, unix))]
mod people_tests;
#[cfg(all(test, unix))]
mod person_merge_tests;
#[cfg(all(test, unix))]
mod projected_read_tests;
#[cfg(unix)]
mod requests;
#[cfg(all(test, unix))]
mod review_card_tests;
mod rollback_review;
#[cfg(all(test, unix))]
mod rollback_tests;
#[cfg(test)]
mod test_fixture;
#[cfg(test)]
mod transcript_view_tests;
#[cfg(all(test, unix))]
mod workflow_performance_tests;
#[cfg(test)]
use rollback_review::RollbackReview;
#[cfg(all(test, unix))]
mod tests;

#[cfg(unix)]
use bruhs_models::workspace_bridge::{
    mcp_client_config, CONNECTION_FAILED_NOTE as MCP_CONNECTION_FAILED_NOTE,
    COPIED_NOTE as MCP_COPIED_NOTE, SESSION_ACTIVE_NOTE as MCP_SESSION_ACTIVE_NOTE,
    SESSION_OFF_NOTE as MCP_OFF_NOTE,
};

/// Web `McpConnectionPanel`'s card and title.
fn mcp_bridge_panel() -> Stateful<Div> {
    super::automations_ui::web_card(super::automations_ui::neutral_ring())
        .id("mcp-connection")
        .role(Role::Region)
        .aria_label(MCP_PANEL_TITLE)
        .child(super::automations_ui::card_title(MCP_PANEL_TITLE))
}

const MCP_PANEL_TITLE: &str = "MCP workspace connection";

#[derive(Default)]
pub(super) struct WorkspaceBridge {
    /// This session's proposals and undo requests (shared with the web).
    reviews: bruhs_models::workspace_bridge::BridgeReviews,
    #[cfg(unix)]
    state: ConnectionState,
    #[cfg(unix)]
    worker: Option<gpui::Task<()>>,
    /// Copy MCP config was pressed in this session: show what it copied.
    #[cfg(unix)]
    copied: bool,
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
    /// Whether the connected client proposed `workflow_id`.
    pub(super) fn owns(&self, workflow_id: &str) -> bool {
        self.reviews
            .proposals
            .values()
            .any(|(id, _)| id == workflow_id)
    }

    /// Workflows whose undo a client asked for, in a stable order.
    pub(super) fn rollback_requests(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .reviews
            .rollback_reviews
            .keys()
            .filter(|id| self.reviews.rollback_pending(id))
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    pub(super) fn pending_review(&self, state: &AutomationState) -> bool {
        self.reviews.pending_review(state)
    }
}

impl Workspace {
    #[cfg(unix)]
    pub(super) fn enable_mcp_bridge(&mut self, cx: &mut Context<'_, Self>) {
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
                                    this.mcp_bridge.reviews = Default::default();
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

    /// `BRUHS_DESKTOP_SMOKE_SCENE=mcp-copied`: the connection on and its
    /// config copied, for the web comparison, without touching the clipboard.
    #[cfg(unix)]
    pub(super) fn smoke_mcp_copied(&mut self, cx: &mut Context<'_, Self>) {
        self.enable_mcp_bridge(cx);
        self.mcp_bridge.copied = true;
    }

    /// `BRUHS_DESKTOP_SMOKE_SCENE=mcp-undo-requested`: the newest workflow
    /// with an undoable step, as if the connected client had proposed it and
    /// then asked to undo it. Only what the review shows; nothing is approved.
    #[cfg(unix)]
    pub(super) fn smoke_mcp_undo_requested(&mut self, cx: &mut Context<'_, Self>) {
        self.enable_mcp_bridge(cx);
        let Some(workflow) = self
            .automation_state
            .workflows
            .iter()
            .rev()
            .find(|workflow| workflow.can_rollback())
        else {
            return;
        };
        let id = workflow.id.clone();
        let proposal = bruhs_models::workspace_bridge::WorkflowProposal {
            title: workflow.title.clone(),
            operations: workflow
                .steps
                .iter()
                .map(|step| step.operation.clone())
                .collect(),
        };
        self.mcp_bridge
            .reviews
            .proposals
            .insert("smoke-undo".into(), (id.clone(), proposal));
        self.mcp_bridge
            .reviews
            .rollback_reviews
            .insert(id, bruhs_models::workspace_bridge::RollbackReview::Pending);
        cx.notify();
    }

    #[cfg(not(unix))]
    pub(super) fn render_mcp_bridge(&self, _cx: &Context<'_, Self>) -> Stateful<Div> {
        mcp_bridge_panel().child(
            super::automations_ui::body_sm(
                "Local MCP connection is not available on this platform yet.",
                theme::rambutan_500(),
            )
            .mt_1(),
        )
    }

    #[cfg(unix)]
    pub(super) fn render_mcp_bridge(&self, cx: &Context<'_, Self>) -> Stateful<Div> {
        let connected = matches!(self.mcp_bridge.state, ConnectionState::Connected { .. });
        let starting = matches!(self.mcp_bridge.state, ConnectionState::Starting(_));
        let busy = connected || starting;
        mcp_bridge_panel()
            .child(
                super::automations_ui::body_sm(
                    if connected {
                        MCP_SESSION_ACTIVE_NOTE
                    } else {
                        MCP_OFF_NOTE
                    },
                    theme::rambutan_500(),
                )
                .mt_1(),
            )
            .when(
                matches!(self.mcp_bridge.state, ConnectionState::Failed),
                |panel| {
                    panel.child(
                        super::automations_ui::body_sm(
                            MCP_CONNECTION_FAILED_NOTE,
                            theme::lychee_300(),
                        )
                        .id("mcp-connection-error")
                        .mt_2()
                        .role(Role::Alert),
                    )
                },
            )
            .child(
                div()
                    .mt_3()
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
                        .aria_label(if busy {
                            "Disable MCP connection"
                        } else {
                            "Enable MCP connection for this session"
                        })
                        .child(if busy {
                            "Disable connection"
                        } else {
                            "Enable for this session"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if busy {
                                this.mcp_bridge = WorkspaceBridge::default();
                                cx.notify();
                            } else {
                                this.mcp_bridge.copied = false;
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
                            .debug_selector(|| "desktop-mcp-copy-config".into())
                            .aria_label("Copy MCP client configuration")
                            .child("Copy MCP config")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let ConnectionState::Connected { session, .. } =
                                    &this.mcp_bridge.state
                                {
                                    let config = mcp_client_config(
                                        &session.connection_path().to_string_lossy(),
                                    );
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        config.to_string(),
                                    ));
                                    this.mcp_bridge.copied = true;
                                    cx.notify();
                                }
                            })),
                        )
                    }),
            )
            .when(connected && self.mcp_bridge.copied, |panel| {
                panel.child(
                    super::automations_ui::label_sm(MCP_COPIED_NOTE, theme::pandan_300())
                        .id("mcp-copied")
                        .debug_selector(|| "desktop-mcp-copied".into())
                        .mt_2()
                        .role(Role::Status),
                )
            })
    }
}
