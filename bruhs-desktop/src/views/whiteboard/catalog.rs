//! Project board sessions retain their writer and recovery state through navigation.
mod agent;
mod controls;
mod files;
use super::{BoardCanvasEvent, WhiteboardView};
use crate::{paths, theme};
use bruhs_core::whiteboard::{BoardStore, BoardWriter, StoreError};
use bruhs_models::whiteboard::*;
use bruhs_models::whiteboard::{bounds, Asset, Point};
use bruhs_ui::text_field::TextField;
use gpui::{prelude::*, *};
use gpui_component::input::{InputEvent, InputState};
use std::{collections::BTreeMap, sync::Arc};

pub(crate) struct BoardSeat {
    pub view: Entity<WhiteboardView>,
    pub writer: Option<Arc<BoardWriter>>,
    pub payloads: Arc<BTreeMap<String, AssetPayload>>,
    _subscription: Subscription,
}
#[derive(Clone)]
pub(crate) enum WhiteboardEvent {
    OpenLink(BoardLink, String),
}
#[derive(Clone, Copy)]
enum DialogKind {
    Commands,
    Text,
    Rename,
    Connect,
    Link,
    Mermaid,
}
struct BoardDialog {
    kind: DialogKind,
    input: TextField,
    link_kind: Entity<InputState>,
    _subscription: Subscription,
}
pub(crate) struct WhiteboardLibrary {
    project: String,
    store: Option<BoardStore>,
    boards: Vec<BoardSummary>,
    seats: BTreeMap<String, BoardSeat>,
    current: Option<String>,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    import_warnings: Vec<String>,
    dialog: Option<BoardDialog>,
    preview: Option<(PortableBoard, Entity<WhiteboardView>)>,
    reviewing: Option<(BoardProposal, Entity<WhiteboardView>)>,
    pub proposals: Vec<BoardProposal>,
    pub statuses: BTreeMap<String, ProposalStatus>,
    create_when_ready: bool,
    shown: bool,
    _activation: Subscription,
}
impl EventEmitter<WhiteboardEvent> for WhiteboardLibrary {}
impl WhiteboardLibrary {
    pub fn new(
        project: String,
        requested: Option<String>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if this.shown && window.is_window_active() {
                this.check_sources(cx);
            }
        });
        let root =
            paths::desktop_state_file().and_then(|p| p.parent().map(|p| p.join("whiteboards")));
        let project_copy = project.clone();
        let read = cx.background_spawn(async move {
            let root =
                root.ok_or_else(|| "The native profile directory is unavailable.".to_string())?;
            let store = BoardStore::open(root).map_err(|e| e.to_string())?;
            let boards = store.list(&project_copy).map_err(|e| e.to_string())?;
            Ok::<_, String>((store, boards))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((store, boards)) => {
                        this.store = Some(store);
                        this.boards = boards;
                        if this.create_when_ready {
                            this.new_board(window, cx);
                        } else if let Some(id) = requested {
                            this.open(id, window, cx);
                        }
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            project,
            store: None,
            boards: Vec::new(),
            seats: BTreeMap::new(),
            current: None,
            busy: true,
            error: None,
            notice: None,
            import_warnings: Vec::new(),
            dialog: None,
            preview: None,
            reviewing: None,
            proposals: Vec::new(),
            statuses: BTreeMap::new(),
            create_when_ready: false,
            shown: false,
            _activation: activation,
        }
    }
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }
    fn active(&self) -> Option<&BoardSeat> {
        self.current.as_ref().and_then(|id| self.seats.get(id))
    }
    fn document(&self, cx: &App) -> Option<BoardDocument> {
        self.active()
            .map(|seat| seat.view.read(cx).input.borrow().editor.document().clone())
    }
    fn fail(&mut self, error: impl ToString, cx: &mut Context<'_, Self>) {
        self.error = Some(error.to_string());
        cx.notify();
    }
    pub fn new_board(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.busy {
            self.create_when_ready = true;
            return;
        }
        self.create_when_ready = false;
        if self.project.is_empty() {
            self.fail("Open a project folder before creating a board.", cx);
            return;
        }
        let board = BoardDocument::empty(
            uuid::Uuid::new_v4().to_string(),
            self.project.clone(),
            "Untitled board".into(),
            now(),
        );
        self.save_new(
            PortableBoard {
                board,
                payloads: BTreeMap::new(),
            },
            window,
            cx,
        );
    }
    pub fn open(&mut self, id: String, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some(seat) = self.seats.get(&id) {
            seat.view.update(cx, |view, cx| view.focus(window, cx));
            self.current = Some(id);
            self.dialog = None;
            cx.notify();
            return;
        }
        if self.busy {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let project = self.project.clone();
        self.busy = true;
        self.error = None;
        let read = cx.background_spawn(async move {
            let lease = match store.lease(&id) {
                Ok(lease) => Some(Arc::new(lease)),
                Err(StoreError::Busy) => None,
                Err(e) => return Err(e.to_string()),
            };
            let (board, version) = store.snapshot(&id).map_err(|e| e.to_string())?;
            if board.project_path != project {
                return Err("This board belongs to another working folder.".into());
            }
            let portable = store.portable(board).map_err(|e| e.to_string())?;
            let images = files::decode_images(&portable)?;
            let writer = lease.map(|lease| Arc::new(BoardWriter::new(store, lease, version)));
            Ok((portable, writer, images))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((portable, writer, images)) => {
                        this.install(portable, writer, images, window, cx)
                    }
                    Err(e) => this.fail(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn save_new(
        &mut self,
        portable: PortableBoard,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.busy {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let work = cx.background_spawn(async move {
            portable.validate().map_err(|e| e.to_string())?;
            let images = files::decode_images(&portable)?;
            let lease = Arc::new(store.lease(&portable.board.id).map_err(|e| e.to_string())?);
            for asset in portable.board.assets.values() {
                store
                    .put_asset(asset, &portable.payloads[&asset.sha256])
                    .map_err(|e| e.to_string())?;
            }
            let version = lease
                .save(&portable.board, None)
                .map_err(|e| e.to_string())?;
            let writer = Arc::new(BoardWriter::new(store, lease, version));
            Ok::<_, String>((portable, Some(writer), images))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok((portable, writer, images)) => {
                        this.preview = None;
                        this.install(portable, writer, images, window, cx);
                    }
                    Err(e) => this.fail(e, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn install(
        &mut self,
        portable: PortableBoard,
        writer: Option<Arc<BoardWriter>>,
        images: super::canvas::BoardImages,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let check_sources = !portable.board.sources.is_empty();
        let id = portable.board.id.clone();
        let summary = portable.board.summary();
        let editor = match BoardEditor::new(portable.board) {
            Ok(e) => e,
            Err(e) => {
                self.fail(e, cx);
                return;
            }
        };
        let view = cx.new(|_| WhiteboardView::new(editor));
        view.update(cx, |view, cx| {
            view.frozen = writer.is_none();
            view.install_images(images, cx);
            view.fit(cx);
        });
        let board_id = id.clone();
        let subscription = cx.subscribe_in(&view, window, move |this, _, event, window, cx| {
            match event {
                BoardCanvasEvent::Changed => this.save(&board_id, false, cx),
                BoardCanvasEvent::Action(action) => this.action(*action, window, cx),
                BoardCanvasEvent::Commands => {
                    this.show_dialog(DialogKind::Commands, "", window, cx)
                }
                BoardCanvasEvent::BeginText(text) => this.begin_text(text, window, cx),
                BoardCanvasEvent::Image => this.pick_file(true, window, cx),
            }
            cx.notify();
        });
        self.boards.retain(|b| b.id != id);
        self.boards.insert(0, summary);
        self.seats.insert(
            id.clone(),
            BoardSeat {
                view: view.clone(),
                writer,
                payloads: Arc::new(portable.payloads),
                _subscription: subscription,
            },
        );
        self.current = Some(id);
        self.dialog = None;
        view.update(cx, |view, cx| view.focus(window, cx));
        if check_sources {
            self.check_sources(cx);
        }
    }
    fn save(&mut self, id: &str, retry: bool, cx: &mut Context<'_, Self>) {
        let Some(seat) = self.seats.get(id) else {
            return;
        };
        let Some(writer) = seat.writer.clone() else {
            return;
        };
        let board = Arc::new(seat.view.read(cx).input.borrow().editor.document().clone());
        for proposal in self
            .proposals
            .iter()
            .filter(|p| p.board_id == id && p.base_revision != board.revision)
        {
            if self.statuses.get(&proposal.id) == Some(&ProposalStatus::Pending) {
                self.statuses
                    .insert(proposal.id.clone(), ProposalStatus::Stale);
            }
        }
        let summary = board.summary();
        if let Some(item) = self.boards.iter_mut().find(|item| item.id == id) {
            *item = summary;
        }
        let payloads = seat.payloads.clone();
        let view = seat.view.clone();
        let refresh = !view.read(cx).images_current(&board);
        if refresh {
            let previous = view.read(cx).images();
            let portable = PortableBoard {
                board: (*board).clone(),
                payloads: (*payloads).clone(),
            };
            let id = id.to_string();
            let work =
                cx.background_spawn(async move { files::prepare_images(&portable, previous) });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| {
                    if let Some(seat) = this.seats.get(&id) {
                        seat.view.update(cx, |view, cx| match result {
                            Ok(images) => {
                                if images.current(view.input.borrow().editor.document()) {
                                    view.install_images(images, cx);
                                }
                            }
                            Err(e) => {
                                view.error = Some(e);
                                cx.notify();
                            }
                        });
                    }
                });
            })
            .detach();
        }
        let start = writer.submit(board, payloads, retry);
        match start {
            Ok(true) => {
                let work =
                    cx.background_spawn(async move { writer.run().map_err(|e| e.to_string()) });
                cx.spawn(async move |this, cx| {
                    let result = work.await;
                    let _ = this.update(cx, |this, cx| {
                        if let Err(e) = result {
                            this.error = Some(e);
                        }
                        cx.notify();
                    });
                })
                .detach();
            }
            Ok(false) => {}
            Err(e) => self.fail(e, cx),
        }
        cx.notify();
    }
    fn edit(
        &mut self,
        edit: impl FnOnce(&mut BoardInput) -> BoardResult<()>,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some(view) = self.active().map(|s| s.view.clone()) {
            view.update(cx, |view, cx| view.edit(edit, cx));
        }
    }
    fn show_dialog(
        &mut self,
        kind: DialogKind,
        value: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let multiline = matches!(kind, DialogKind::Text | DialogKind::Mermaid);
        let input = TextField::new(multiline, window, cx);
        input.set_value(value.to_string(), window, cx);
        let link_kind = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value("file", window, cx);
            state
        });
        let subscription =
            input.subscribe_in(window, cx, |this, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { secondary, shift } = event {
                    let multiline = this
                        .dialog
                        .as_ref()
                        .is_some_and(|d| matches!(d.kind, DialogKind::Text | DialogKind::Mermaid));
                    if !multiline || *secondary && !shift {
                        this.submit_dialog(window, cx);
                    }
                }
                cx.notify();
            });
        window.focus(&input.focus_handle(cx), cx);
        self.dialog = Some(BoardDialog {
            kind,
            input,
            link_kind,
            _subscription: subscription,
        });
        cx.notify();
    }
    fn begin_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.edit(
            |input| {
                let at = input.editor.camera.world(Point::new(240.0, 160.0));
                input
                    .editor
                    .add_node(ShapeKind::Text, at, text.into(), now())?;
                Ok(())
            },
            cx,
        );
        self.show_dialog(DialogKind::Text, text, window, cx);
    }
    pub fn open_mermaid(
        &mut self,
        text: &str,
        thread_id: Option<String>,
        cx: &mut Context<'_, Self>,
    ) {
        let result = import_mermaid(text, self.project.clone(), "Chat diagram".into(), now())
            .and_then(|mut board| {
                if let Some(id) = thread_id {
                    for shape in board.shapes.values_mut() {
                        shape.links.push(BoardLink::Thread { id: id.clone() });
                    }
                }
                self.set_preview(
                    PortableBoard {
                        board,
                        payloads: BTreeMap::new(),
                    },
                    cx,
                )
            });
        if let Err(error) = result {
            self.fail(error, cx);
        }
    }
    fn set_preview(
        &mut self,
        portable: PortableBoard,
        cx: &mut Context<'_, Self>,
    ) -> BoardResult<()> {
        if portable.board.project_path != self.project {
            return Err(BoardError::Invalid(
                "Import this board as a copy in the current working folder.".into(),
            ));
        }
        self.busy = true;
        let work = cx.background_spawn(async move {
            portable.validate().map_err(|e| e.to_string())?;
            let images = files::decode_images(&portable)?;
            let editor = BoardEditor::new(portable.board.clone()).map_err(|e| e.to_string())?;
            Ok::<_, String>((portable, images, editor))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((portable, images, editor)) => {
                        let view = cx.new(|_| WhiteboardView::new(editor));
                        view.update(cx, |view, cx| {
                            view.frozen = true;
                            view.install_images(images, cx);
                            view.fit(cx);
                        });
                        this.preview = Some((portable, view));
                    }
                    Err(e) => this.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        Ok(())
    }
}
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
