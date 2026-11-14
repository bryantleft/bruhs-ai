use super::host::{now, Session, WhiteboardHost};
use bruhs_models::{thread_link::ThreadReply, whiteboard::*};
use leptos::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;
impl WhiteboardHost {
    pub async fn answer(
        self,
        request: BoardRequest,
        requester: BoardRequester,
        project: String,
    ) -> ThreadReply {
        match self.agent_answer(request, requester, project).await {
            Ok(value) => board_tool_reply(value),
            Err(e) => ThreadReply::refused(e),
        }
    }
    async fn agent_seat(self, id: &str, project: &str) -> Result<Session, String> {
        let persisted = crate::storage::whiteboards::load(id)
            .await
            .map_err(|e| e.to_string())?;
        shared_board(&persisted.board, project).map_err(|e| e.to_string())?;
        if let Some(seat) = self.seat(id) {
            let state = seat.borrow();
            shared_board(state.input.editor.document(), project).map_err(|e| e.to_string())?;
            if state.saved != Some(persisted.board.revision) {
                return Err("This board changed in another tab. Preserve your current edits and reopen it before using agent tools.".into());
            }
            drop(state);
            Ok(seat)
        } else {
            let revision = persisted.board.revision;
            self.insert(persisted, Some(revision))
                .map_err(|e| e.to_string())
        }
    }
    async fn agent_answer(
        self,
        request: BoardRequest,
        requester: BoardRequester,
        project: String,
    ) -> Result<Value, String> {
        request.validate().map_err(str::to_string)?;
        if project.is_empty() {
            return Err("The calling thread has no working folder.".into());
        }
        match request {
            BoardRequest::List => {
                let mut boards = crate::storage::whiteboards::list(&project)
                    .await
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .map(|b| (b.id.clone(), b))
                    .collect::<BTreeMap<_, _>>();
                for board in self.summaries() {
                    if board.project_path == project {
                        if let Some(saved) = boards.get_mut(&board.id) {
                            if saved.agent_read && board.agent_read {
                                *saved = board;
                            } else {
                                saved.agent_read = false;
                            }
                        }
                    }
                }
                Ok(
                    json!({"boards":boards.into_values().filter(|b|b.agent_read).take(100).collect::<Vec<_>>(),"store":"web IndexedDB"}),
                )
            }
            BoardRequest::Status { proposal_id } => {
                let proposal = self
                    .proposals
                    .with_untracked(|p| {
                        p.iter()
                            .find(|p| {
                                p.id == proposal_id
                                    && p.requester == requester
                                    && p.project_path == project
                            })
                            .cloned()
                    })
                    .ok_or("This proposal does not belong to this thread and turn.")?;
                self.agent_seat(&proposal.board_id, &project).await?;
                let status = self
                    .statuses
                    .with_untracked(|s| s.get(&proposal_id).cloned())
                    .unwrap_or(ProposalStatus::Pending);
                Ok(json!({"proposal_id":proposal_id,"status":status}))
            }
            BoardRequest::Read {
                board_id,
                cursor,
                limit,
            } => {
                let seat = self.agent_seat(&board_id, &project).await?;
                let state = seat.borrow();
                let board = shared_board(state.input.editor.document(), &project)
                    .map_err(|e| e.to_string())?;
                board_page(board, cursor.as_deref(), limit).map_err(|e| e.to_string())
            }
            BoardRequest::Propose {
                board_id,
                base_revision,
                description,
                edits_json,
            } => {
                let seat = self.agent_seat(&board_id, &project).await?;
                let board = {
                    let state = seat.borrow();
                    shared_board(state.input.editor.document(), &project)
                        .map_err(|e| e.to_string())?
                        .clone()
                };
                if board.revision != base_revision {
                    return Err(BoardError::Conflict.to_string());
                }
                let mut operations =
                    proposal_operations(&board, &edits_json, now()).map_err(|e| e.to_string())?;
                for operation in &mut operations {
                    if let BoardOperation::Source { source } = operation {
                        source.sha256 =
                            crate::api::whiteboard_source_fingerprint(&project, &source.path)
                                .await
                                .map_err(|e| e.to_string())?;
                        source.stale = false;
                    }
                }
                // Recheck persisted sharing after source I/O, before delivering the preview.
                self.agent_seat(&board_id, &project).await?;
                let state = seat.borrow();
                if state.input.editor.document() != &board {
                    return Err(BoardError::Conflict.to_string());
                }
                let proposal = BoardProposal::new(&board, requester, description, operations)
                    .map_err(|e| e.to_string())?;
                let id = proposal.id.clone();
                drop(state);
                self.statuses.update(|s| {
                    s.insert(id.clone(), ProposalStatus::Pending);
                });
                self.proposals.update(|p| p.push(proposal));
                self.notify();
                Ok(
                    json!({"proposal_id":id,"board_id":board_id,"status":"pending","applied":false,"owner_action":"Open Whiteboards, select this board and review the before/after preview. Only the owner can accept or reject."}),
                )
            }
        }
    }
}
