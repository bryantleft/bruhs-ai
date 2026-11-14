use super::*;
use bruhs_models::thread_link::ThreadReply;
use serde_json::{json, Value};
enum Prepared {
    Reply(Value),
    Proposal(BoardProposal),
}
impl WhiteboardLibrary {
    pub fn answer(
        &mut self,
        request: BoardRequest,
        requester: BoardRequester,
        cx: &mut Context<'_, Self>,
    ) -> Task<ThreadReply> {
        let project = self.project.clone();
        let root =
            paths::desktop_state_file().and_then(|p| p.parent().map(|p| p.join("whiteboards")));
        let current = self
            .seats
            .iter()
            .map(|(id, seat)| {
                (
                    id.clone(),
                    seat.view.read(cx).input.borrow().editor.document().clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let writers = self
            .seats
            .iter()
            .filter_map(|(id, seat)| {
                seat.writer
                    .as_ref()
                    .map(|writer| (id.clone(), writer.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let proposal = if let BoardRequest::Status { proposal_id } = &request {
            self.proposals
                .iter()
                .find(|p| &p.id == proposal_id && p.requester == requester)
                .cloned()
        } else {
            None
        };
        let status = proposal
            .as_ref()
            .and_then(|p| self.statuses.get(&p.id).cloned())
            .unwrap_or(ProposalStatus::Pending);
        let work=cx.background_spawn(async move{
            request.validate().map_err(str::to_string)?;
            let store=BoardStore::open(root.ok_or("The native board profile is unavailable.")?).map_err(|e|e.to_string())?;
            let load=|id:&str|->Result<BoardDocument,String>{
                let persisted=if let Some(writer)=writers.get(id){writer.authority().map_err(|e|e.to_string())?}else{store.load(id).map_err(|e|e.to_string())?};
                shared_board(&persisted,&project).map_err(|e|e.to_string())?;
                if let Some(cached)=current.get(id){shared_board(cached,&project).map_err(|e|e.to_string())?;if !writers.contains_key(id)&&cached.revision!=persisted.revision{return Err(BoardError::Conflict.to_string());}Ok(cached.clone())}else{Ok(persisted)}
            };
            match request{
                BoardRequest::List=>{let mut summaries=store.list(&project).map_err(|e|e.to_string())?.into_iter().map(|s|(s.id.clone(),s)).collect::<BTreeMap<_,_>>();for board in current.values(){if let Some(saved)=summaries.get_mut(&board.id){if saved.agent_read&&board.agent_read{*saved=board.summary();}else{saved.agent_read=false;}}}Ok(Prepared::Reply(json!({"boards":summaries.into_values().filter(|b|b.agent_read&&b.project_path==project).take(100).collect::<Vec<_>>(),"store":"native profile"})))},
                BoardRequest::Status{proposal_id}=>{let proposal=proposal.ok_or("This proposal does not belong to this thread and turn.")?;shared_board(&load(&proposal.board_id)?,&project).map_err(|e|e.to_string())?;Ok(Prepared::Reply(json!({"proposal_id":proposal_id,"status":status})))},
                BoardRequest::Read{board_id,cursor,limit}=>{let board=load(&board_id)?;shared_board(&board,&project).map_err(|e|e.to_string())?;Ok(Prepared::Reply(board_page(&board,cursor.as_deref(),limit).map_err(|e|e.to_string())?))},
                BoardRequest::Propose{board_id,base_revision,description,edits_json}=>{
                    let board=load(&board_id)?;shared_board(&board,&project).map_err(|e|e.to_string())?;if board.revision!=base_revision{return Err(BoardError::Conflict.to_string());}
                    let mut ops=proposal_operations(&board,&edits_json,now()).map_err(|e|e.to_string())?;
                    for op in &mut ops{if let BoardOperation::Source{source}=op{source.sha256=bruhs_core::whiteboard::source_fingerprint(std::path::Path::new(&project),&source.path).map_err(|e|e.to_string())?;source.stale=false;}}
                    Ok(Prepared::Proposal(BoardProposal::new(&board,requester,description,ops).map_err(|e|e.to_string())?))
                }
            }
        });
        cx.spawn(async move|this,cx|{
            let prepared=match work.await{Ok(p)=>p,Err(e)=>return ThreadReply::refused(e)};
            match prepared{
                Prepared::Reply(value)=>board_tool_reply(value),
                Prepared::Proposal(proposal)=>this.update(cx,|this,cx|{
                    if let Some(seat)=this.seats.get(&proposal.board_id){if let Err(e)=proposal.validate_for(seat.view.read(cx).input.borrow().editor.document()){return ThreadReply::refused(e.to_string());}}
                    let value=json!({"proposal_id":proposal.id,"board_id":proposal.board_id,"status":"pending","applied":false,"owner_action":"Open Whiteboards and review the before/after preview. Only the owner can accept or reject."});
                    this.statuses.insert(proposal.id.clone(),ProposalStatus::Pending);this.proposals.push(proposal);this.notice=Some("An agent proposal is ready. Open its board to review the exact edit.".into());cx.notify();board_tool_reply(value)
                }).unwrap_or_else(|_|ThreadReply::refused("The owner window closed before the proposal was delivered.")),
            }
        })
    }
    pub(super) fn review(&mut self, proposal: BoardProposal, cx: &mut Context<'_, Self>) {
        let Some(seat) = self.seats.get(&proposal.board_id) else {
            return;
        };
        let current = seat.view.read(cx).input.borrow().editor.document().clone();
        let payloads = seat.payloads.clone();
        let previous = seat.view.read(cx).images();
        let work = cx.background_spawn(async move {
            let candidate = proposal
                .preview(&current, now())
                .map_err(|e| e.to_string())?;
            let images = files::prepare_images(
                &PortableBoard {
                    board: candidate.clone(),
                    payloads: (*payloads).clone(),
                },
                previous,
            )?;
            let editor = BoardEditor::new(candidate).map_err(|e| e.to_string())?;
            Ok::<_, String>((proposal, editor, images))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((proposal, editor, images)) => {
                    let current = this
                        .seats
                        .get(&proposal.board_id)
                        .map(|s| s.view.read(cx).input.borrow().editor.document().clone());
                    if current
                        .as_ref()
                        .is_none_or(|b| proposal.validate_for(b).is_err())
                    {
                        this.statuses.insert(proposal.id, ProposalStatus::Stale);
                        this.fail(BoardError::Conflict, cx);
                        return;
                    }
                    let preview = cx.new(|_| WhiteboardView::new(editor));
                    preview.update(cx, |view, cx| {
                        view.frozen = true;
                        view.install_images(images, cx);
                        view.fit(cx);
                    });
                    this.reviewing = Some((proposal, preview));
                    cx.notify();
                }
                Err(e) => this.fail(e, cx),
            });
        })
        .detach();
    }
    pub(super) fn accept(&mut self, proposal: BoardProposal, cx: &mut Context<'_, Self>) {
        if self.statuses.get(&proposal.id) != Some(&ProposalStatus::Pending) {
            self.fail("This proposal is no longer pending.", cx);
            return;
        }
        let Some(seat) = self.seats.get(&proposal.board_id) else {
            return;
        };
        let Some(writer) = seat.writer.clone() else {
            self.fail("This board is read-only in this window.", cx);
            return;
        };
        let current = seat.view.read(cx).input.borrow().editor.document().clone();
        if writer.status().map_or(true, |s| {
            s.busy || s.error.is_some() || s.revision != current.revision
        }) {
            self.fail("Save the current board before accepting this proposal.", cx);
            return;
        }
        let candidate = match proposal.preview(&current, now()) {
            Ok(b) => b,
            Err(e) => {
                self.statuses
                    .insert(proposal.id.clone(), ProposalStatus::Stale);
                self.fail(e, cx);
                return;
            }
        };
        let stamp = candidate.updated_at_ms;
        match writer.submit(Arc::new(candidate), seat.payloads.clone(), false) {
            Ok(true) => {}
            Ok(false) => {
                self.fail("The proposal could not start a new save.", cx);
                return;
            }
            Err(e) => {
                self.fail(e, cx);
                return;
            }
        }
        seat.view.update(cx, |view, cx| {
            view.frozen = true;
            cx.notify();
        });
        let work = cx.background_spawn(async move { writer.run().map_err(|e| e.to_string()) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(seat) = this.seats.get(&proposal.board_id) {
                    seat.view.update(cx, |view, cx| {
                        view.frozen = false;
                        match result {
                            Ok(version) => {
                                match proposal.accept(&mut view.input.borrow_mut().editor, stamp) {
                                    Ok(()) => {
                                        this.statuses.insert(
                                            proposal.id.clone(),
                                            ProposalStatus::Accepted {
                                                revision: version.revision(),
                                            },
                                        );
                                        this.reviewing = None;
                                    }
                                    Err(e) => {
                                        view.error = Some(format!(
                                            "The accepted revision is saved. Reopen the board: {e}"
                                        ))
                                    }
                                }
                            }
                            Err(e) => view.error = Some(e),
                        }
                        cx.notify();
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }
}
