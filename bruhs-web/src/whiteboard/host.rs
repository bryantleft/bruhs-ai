//! App-owned board sessions survive view changes and serialize saves per board.
use crate::state::{AppView, OneShot, SetActiveView};
use bruhs_models::{num::ToIntSaturating as _, whiteboard::*};
use leptos::prelude::*;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

pub(crate) struct Seat {
    pub input: BoardInput,
    pub payloads: Rc<BTreeMap<String, AssetPayload>>,
    pub saved: Option<u64>,
    pub writing: bool,
    pub save_failed: bool,
    pub frozen: bool,
    pub error: Option<String>,
}
pub(crate) type Session = Rc<RefCell<Seat>>;
#[derive(Clone, Copy)]
pub(crate) struct WhiteboardHost {
    seats: StoredValue<BTreeMap<String, Session>, LocalStorage>,
    pub changed: RwSignal<u64>,
    pub catalog_changed: RwSignal<u64>,
    pub create: OneShot,
    active_view: WriteSignal<AppView>,
    pub mermaid: RwSignal<Option<String>>,
    pub proposals: RwSignal<Vec<BoardProposal>>,
    pub statuses: RwSignal<BTreeMap<String, ProposalStatus>>,
}
pub(crate) fn now() -> i64 {
    js_sys::Date::now().to_i64_saturating()
}
impl WhiteboardHost {
    pub fn install() {
        provide_context(Self {
            seats: StoredValue::new_local(BTreeMap::new()),
            changed: RwSignal::new(0),
            catalog_changed: RwSignal::new(0),
            create: OneShot::new(),
            active_view: expect_context::<SetActiveView>().0,
            mermaid: RwSignal::new(None),
            proposals: RwSignal::new(Vec::new()),
            statuses: RwSignal::new(BTreeMap::new()),
        });
    }
    pub fn open_mermaid(self, text: String) {
        self.mermaid.set(Some(text));
        self.route(None);
    }
    pub fn notify(self) {
        self.changed.update(|n| *n = n.wrapping_add(1));
    }
    pub fn seat(self, id: &str) -> Option<Session> {
        self.seats.with_value(|seats| seats.get(id).cloned())
    }
    pub fn summaries(self) -> Vec<BoardSummary> {
        self.seats.with_value(|seats| {
            seats
                .values()
                .map(|s| s.borrow().input.editor.document().summary())
                .collect()
        })
    }
    pub fn insert(self, portable: PortableBoard, saved: Option<u64>) -> BoardResult<Session> {
        portable.validate()?;
        let id = portable.board.id.clone();
        let seat = Rc::new(RefCell::new(Seat {
            input: BoardInput::new(BoardEditor::new(portable.board)?),
            payloads: Rc::new(portable.payloads),
            saved,
            writing: false,
            save_failed: false,
            frozen: false,
            error: None,
        }));
        self.seats.update_value(|seats| {
            seats.insert(id, seat.clone());
        });
        self.notify();
        Ok(seat)
    }
    pub async fn load(self, id: &str, project: &str) -> Result<Session, String> {
        let seat = if let Some(seat) = self.seat(id) {
            seat
        } else {
            let portable = crate::storage::whiteboards::load(id)
                .await
                .map_err(|e| e.to_string())?;
            let revision = portable.board.revision;
            if let Some(seat) = self.seat(id) {
                seat
            } else {
                self.insert(portable, Some(revision))
                    .map_err(|e| e.to_string())?
            }
        };
        if seat.borrow().input.editor.document().project_path != project {
            return Err(
                "This board belongs to another working folder. Select its project to open it."
                    .into(),
            );
        }
        self.check_sources(id);
        Ok(seat)
    }
    pub fn route(self, id: Option<String>) {
        self.active_view.set(AppView::Whiteboards { board_id: id });
    }
    fn invalidate(self, id: &str, revision: u64) {
        let stale = self.proposals.with_untracked(|items| {
            items
                .iter()
                .filter(|p| p.board_id == id && p.base_revision != revision)
                .map(|p| p.id.clone())
                .collect::<Vec<_>>()
        });
        if stale.is_empty() {
            return;
        }
        self.statuses.update(|statuses| {
            for id in stale {
                if statuses.get(&id) == Some(&ProposalStatus::Pending) {
                    statuses.insert(id, ProposalStatus::Stale);
                }
            }
        });
    }
    pub fn edit(self, id: &str, edit: impl FnOnce(&mut BoardInput) -> BoardResult<()>) {
        let Some(seat) = self.seat(id) else { return };
        let mut state = seat.borrow_mut();
        if state.frozen {
            return;
        }
        let revision = state.input.editor.document().revision;
        match edit(&mut state.input) {
            Ok(()) => {}
            Err(error) => state.error = Some(error.to_string()),
        }
        let next = state.input.editor.document().revision;
        let dirty = next != revision;
        drop(state);
        self.notify();
        if dirty {
            self.invalidate(id, next);
            self.save(id, false);
        }
    }
    pub fn check_sources(self, id: &str) {
        let Some(seat) = self.seat(id) else {
            return;
        };
        let board = seat.borrow().input.editor.document().clone();
        if board.sources.is_empty() {
            return;
        }
        let id = id.to_string();
        leptos::task::spawn_local(async move {
            let mut operations = Vec::new();
            for mut source in board.sources.into_values() {
                let stale =
                    crate::api::whiteboard_source_fingerprint(&board.project_path, &source.path)
                        .await
                        .map_or(true, |hash| hash != source.sha256);
                if stale != source.stale {
                    source.stale = stale;
                    operations.push(BoardOperation::Source { source });
                }
            }
            if operations.is_empty() {
                return;
            }
            self.edit(&id, |input| {
                if input.editor.document().revision != board.revision {
                    return Err(BoardError::Conflict);
                }
                input.editor.edit(operations, now())
            });
        });
    }
    pub fn insert_image(self, id: &str, asset: Asset, payload: AssetPayload) -> BoardResult<()> {
        let seat = self.seat(id).ok_or(BoardError::Missing)?;
        let mut state = seat.borrow_mut();
        if state.frozen {
            return Err(BoardError::Invalid(
                "Wait for the current owner action before adding an image.".into(),
            ));
        }
        let input = &mut state.input;
        let shape_id = uuid::Uuid::new_v4().to_string();
        let scale = (320.0 / f64::from(asset.width)).min(1.0);
        let shape = Shape::node(
            shape_id.clone(),
            ShapeKind::Image {
                asset_id: asset.id.clone(),
            },
            Rect::new(
                input.editor.camera.x + 40.0,
                input.editor.camera.y + 40.0,
                f64::from(asset.width) * scale,
                f64::from(asset.height) * scale,
            ),
            input.editor.next_z()?,
            String::new(),
        );
        input.editor.edit(
            vec![
                BoardOperation::Asset { asset },
                BoardOperation::shape(shape),
            ],
            now(),
        )?;
        input.editor.selection = std::collections::BTreeSet::from([shape_id]);
        Rc::make_mut(&mut state.payloads).insert(payload.sha256.clone(), payload);
        drop(state);
        self.save(id, false);
        self.notify();
        Ok(())
    }
    pub fn save(self, id: &str, retry: bool) {
        let Some(seat) = self.seat(id) else { return };
        {
            let mut state = seat.borrow_mut();
            if state.writing || state.frozen || state.save_failed && !retry {
                return;
            }
            if state.saved == Some(state.input.editor.document().revision) {
                return;
            }
            state.writing = true;
            state.error = None;
            state.save_failed = false;
        }
        self.notify();
        leptos::task::spawn_local(async move {
            loop {
                let (board, expected, payloads) = {
                    let state = seat.borrow();
                    (
                        state.input.editor.document().clone(),
                        state.saved,
                        state.payloads.clone(),
                    )
                };
                let result = crate::storage::whiteboards::save(&board, expected, &payloads).await;
                let mut state = seat.borrow_mut();
                match result {
                    Ok(()) => {
                        state.saved = Some(board.revision);
                        self.catalog_changed.update(|n| *n = n.wrapping_add(1));
                        if state.input.editor.document().revision == board.revision {
                            state.writing = false;
                            drop(state);
                            self.notify();
                            break;
                        }
                    }
                    Err(error) => {
                        state.writing = false;
                        state.error = Some(error.to_string());
                        state.save_failed = true;
                        drop(state);
                        self.notify();
                        break;
                    }
                }
                drop(state);
                self.notify();
            }
        });
    }
    pub fn create_board(self, project: String, title: String) -> BoardResult<String> {
        let board = BoardDocument::empty(uuid::Uuid::new_v4().to_string(), project, title, now());
        let id = board.id.clone();
        self.insert(
            PortableBoard {
                board,
                payloads: BTreeMap::new(),
            },
            None,
        )?;
        self.save(&id, false);
        self.route(Some(id.clone()));
        Ok(id)
    }
    pub fn save_copy(self, id: &str) -> BoardResult<String> {
        let seat = self.seat(id).ok_or(BoardError::Missing)?;
        let state = seat.borrow();
        let portable = PortableBoard {
            board: state.input.editor.document().clone(),
            payloads: state
                .payloads
                .iter()
                .filter(|(hash, _)| {
                    state
                        .input
                        .editor
                        .document()
                        .assets
                        .values()
                        .any(|a| &a.sha256 == *hash)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
        .import_copy(state.input.editor.document().project_path.clone(), now())?;
        drop(state);
        let id = portable.board.id.clone();
        self.insert(portable, None)?;
        self.save(&id, false);
        self.route(Some(id.clone()));
        Ok(id)
    }
    pub fn accept(self, proposal: BoardProposal) {
        if !self
            .statuses
            .with_untracked(|s| s.get(&proposal.id) == Some(&ProposalStatus::Pending))
            || !self
                .proposals
                .with_untracked(|items| items.contains(&proposal))
        {
            return;
        }

        let Some(seat) = self.seat(&proposal.board_id) else {
            return;
        };
        let candidate = {
            let mut state = seat.borrow_mut();
            if state.writing
                || state.frozen
                || state.saved != Some(state.input.editor.document().revision)
            {
                state.error = Some("Save the current board before accepting this proposal.".into());
                drop(state);
                self.notify();
                return;
            }
            match proposal.preview(state.input.editor.document(), now()) {
                Ok(candidate) => {
                    state.frozen = true;
                    Ok((candidate, state.saved, state.payloads.clone()))
                }
                Err(error) => Err(error),
            }
        };
        let (candidate, expected, payloads) = match candidate {
            Ok(value) => value,
            Err(error) => {
                seat.borrow_mut().error = Some(error.to_string());
                self.statuses.update(|s| {
                    s.insert(proposal.id.clone(), ProposalStatus::Stale);
                });
                self.notify();
                return;
            }
        };
        self.notify();
        leptos::task::spawn_local(async move {
            let result = crate::storage::whiteboards::save(&candidate, expected, &payloads).await;
            let mut state = seat.borrow_mut();
            state.frozen = false;
            match result {
                Ok(()) => match proposal.accept(&mut state.input.editor, candidate.updated_at_ms) {
                    Ok(()) => {
                        state.saved = Some(candidate.revision);
                        self.statuses.update(|s| {
                            s.insert(
                                proposal.id.clone(),
                                ProposalStatus::Accepted {
                                    revision: candidate.revision,
                                },
                            );
                        });
                        self.invalidate(&proposal.board_id, candidate.revision);
                    }
                    Err(error) => {
                        state.error = Some(format!(
                            "The accepted revision was saved but the editor could not adopt it: {error}. Reopen the board."
                        ));
                    }
                },
                Err(error) => state.error = Some(error.to_string()),
            }
            drop(state);
            self.notify();
        });
    }
}
