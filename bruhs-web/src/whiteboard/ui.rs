//! Project catalog and owner controls for the Rust canvas editor.
use super::{
    canvas::BoardCanvas,
    export,
    host::{now, WhiteboardHost},
};
use crate::state::*;
use crate::ui::dialog::Dialog;
use bruhs_models::whiteboard::*;
use leptos::{html, prelude::*};
use std::{collections::BTreeMap, rc::Rc};
use wasm_bindgen::{JsCast as _, JsValue};
pub(super) const BUTTON: &str = "bruhs-control min-h-control shrink-0 rounded-grape border border-longan-700 px-3 py-1 text-label-sm text-rambutan-200 hover:bg-longan-800 focus-visible:outline focus-visible:outline-persimmon-400 disabled:opacity-40";
const FIELD: &str = "min-h-control w-full rounded-grape border border-longan-700 bg-longan-950 px-3 py-2 text-[16px] text-rambutan-100 outline-none focus:border-persimmon-400";

pub(crate) fn project_path() -> Option<String> {
    let active = expect_context::<ActiveThreadId>().0.get_untracked();
    active
        .and_then(|id| {
            expect_context::<ThreadsSignal>()
                .0
                .with_untracked(|threads| {
                    threads.iter().find(|t| t.id == id).and_then(|t| {
                        t.worktree_path
                            .clone()
                            .filter(|s| !s.is_empty())
                            .or_else(|| t.project_path.clone().filter(|s| !s.is_empty()))
                    })
                })
        })
        .or_else(|| {
            expect_context::<ActiveProjectSignal>()
                .0
                .get_untracked()
                .map(|p| p.path)
        })
}
#[component]
pub(crate) fn Whiteboards(requested: Option<String>) -> impl IntoView {
    let host = expect_context::<WhiteboardHost>();
    let error = RwSignal::new(None::<String>);
    let boards = RwSignal::new(Vec::<BoardSummary>::new());
    let loaded = RwSignal::new(false);
    let selected = RwSignal::new(None::<String>);
    let requested = StoredValue::new(requested);
    let generation = RwSignal::new(0u64);
    let open_generation = RwSignal::new(0u64);
    let project = Memo::new(move |_| {
        expect_context::<ActiveThreadId>().0.track();
        expect_context::<ThreadsSignal>().0.track();
        expect_context::<ActiveProjectSignal>().0.track();
        project_path()
    });
    Effect::new(move |_| {
        let project = project.get();
        host.catalog_changed.track();
        generation.update(|g| *g = g.wrapping_add(1));
        let token = generation.get_untracked();
        if let Some(project) = project {
            leptos::task::spawn_local(async move {
                let result = crate::storage::whiteboards::list(&project).await;
                if generation.try_get_untracked() != Some(token) {
                    return;
                }
                match result {
                    Ok(items) => boards.set(items),
                    Err(e) => error.set(Some(e.to_string())),
                }
            });
        }
    });
    Effect::new(move |_| {
        let Some(project) = project.get() else { return };
        open_generation.update(|g| *g = g.wrapping_add(1));
        let token = open_generation.get_untracked();
        if host.create.take() {
            if let Err(e) = host.create_board(project.clone(), "Untitled board".into()) {
                error.set(Some(e.to_string()));
            }
        }
        let Some(id) = requested.get_value() else {
            loaded.set(true);
            return;
        };
        selected.set(None);
        loaded.set(false);
        leptos::task::spawn_local(async move {
            let result = host.load(&id, &project).await;
            if open_generation.try_get_untracked() != Some(token) {
                return;
            }
            match result {
                Ok(_) => selected.set(Some(id)),
                Err(e) => error.set(Some(e)),
            }
            loaded.set(true);
        });
    });
    view! {<section class="flex h-full min-h-0 flex-1 flex-col" aria-label="Whiteboards" data-whiteboards>
        <Show when=move||selected.get().is_none()>
            <header class="flex flex-wrap items-center justify-between gap-3 border-b border-longan-800 px-5 py-4"><div><h1 class="text-h3 text-rambutan-100">"Whiteboards"</h1><p class="mt-1 text-body-sm text-rambutan-500">{move||project.get().unwrap_or_else(||"Choose a project folder".into())}</p></div><button class=BUTTON disabled=move||project.get().is_none() on:click=move|_|{if let Some(project)=project.get_untracked(){if let Err(e)=host.create_board(project,"Untitled board".into()){error.set(Some(e.to_string()));}}}>"New board"</button></header>
            <Show when=move||project.get().is_none()><div class="p-6 text-body-sm text-rambutan-400"><p>"Choose a project folder to create and reopen its boards."</p><button class=BUTTON on:click=move|_|{expect_context::<SetProjectPickerOpen>().0.set(true)}>"Choose project"</button></div></Show>
            <div class="min-h-0 overflow-auto p-4"><For each=move||boards.get() key=|b|b.id.clone() children=move|board|{let id=board.id.clone();view!{<button class="mb-2 flex min-h-[64px] w-full items-center justify-between rounded-grape border border-longan-800 px-4 text-left hover:bg-longan-900" on:click=move|_|host.route(Some(id.clone()))><span class="text-label text-rambutan-200">{board.title}</span><span class="text-label-sm text-rambutan-500">{format!("{} shapes",board.shape_count)}</span></button>}}/></div>
        </Show>
        <Show when=move||!loaded.get()><p role="status" class="p-4 text-body-sm">"Opening local board…"</p></Show>
        {move||selected.get().map(|id|view!{<Editor id=id/>})}
        <super::review::MermaidPreview project=project/>
        {move||error.get().map(|e|view!{<p role="alert" class="p-3 text-body-sm text-lychee-300">{e}</p>})}
    </section>}
}
#[derive(Clone, Copy)]
struct Ui {
    host: WhiteboardHost,
    id: StoredValue<String>,
    dialog: RwSignal<Option<BoardAction>>,
    text: RwSignal<String>,
    kind: RwSignal<String>,
    start: RwSignal<String>,
    end: RwSignal<String>,
    error: RwSignal<Option<String>>,
    preview: RwSignal<Option<PortableBoard>>,
    import_warnings: RwSignal<Vec<String>>,
    query: RwSignal<String>,
    commands: RwSignal<bool>,
    image: NodeRef<html::Input>,
    import: NodeRef<html::Input>,
    canvas: NodeRef<html::Canvas>,
    command_input: NodeRef<html::Input>,
    text_input: NodeRef<html::Textarea>,
    alive: StoredValue<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
impl Ui {
    fn command(self, command: BoardCommand) {
        match command {
            BoardCommand::Action(action) => self.act(action),
            BoardCommand::Tool(tool) => {
                self.commands.set(false);
                if tool == BoardTool::Image {
                    if let Some(input) = self.image.get() {
                        input.click();
                    }
                } else {
                    self.edit(|input| {
                        input.set_tool(tool);
                        Ok(())
                    });
                }
                if let Some(canvas) = self.canvas.get() {
                    let _ = canvas.focus();
                }
            }
        }
    }

    fn edit(self, edit: impl FnOnce(&mut BoardInput) -> BoardResult<()>) {
        self.host.edit(&self.id.get_value(), edit);
    }
    fn fail(self, error: impl ToString) {
        self.error.try_set(Some(error.to_string()));
    }
    fn portable(self) -> BoardResult<PortableBoard> {
        let seat = self
            .host
            .seat(&self.id.get_value())
            .ok_or(BoardError::Missing)?;
        let seat = seat.borrow();
        Ok(PortableBoard {
            board: seat.input.editor.document().clone(),
            payloads: seat
                .payloads
                .iter()
                .filter(|(hash, _)| {
                    seat.input
                        .editor
                        .document()
                        .assets
                        .values()
                        .any(|a| &a.sha256 == *hash)
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        })
    }
    fn act(self, action: BoardAction) {
        self.commands.set(false);
        let mut handled = false;
        self.edit(|input| {
            handled = action.apply(input, now())?;
            Ok(())
        });
        if handled {
            return;
        }
        match action {
            BoardAction::AllBoards => self.host.route(None),
            BoardAction::NewBoard => {
                self.host.create.request();
                self.host.route(None);
            }
            BoardAction::Save => self.host.save(&self.id.get_value(), true),
            BoardAction::SaveCopy => {
                if let Err(e) = self.host.save_copy(&self.id.get_value()) {
                    self.fail(e);
                }
            }
            BoardAction::Fit => {
                if let Some(canvas) = self.canvas.get() {
                    let area = canvas.get_bounding_client_rect();
                    self.edit(|input| input.fit(area.width(), area.height()));
                }
            }
            BoardAction::EditText | BoardAction::Rename => {
                let value = self
                    .host
                    .seat(&self.id.get_value())
                    .map(|seat| {
                        let seat = seat.borrow();
                        let editor = &seat.input.editor;
                        if action == BoardAction::Rename {
                            editor.document().title.clone()
                        } else {
                            editor
                                .selection
                                .first()
                                .and_then(|id| editor.document().shapes.get(id))
                                .map_or(String::new(), |s| s.text.clone())
                        }
                    })
                    .unwrap_or_default();
                self.text.set(value);
                self.dialog.set(Some(action));
            }
            BoardAction::ConnectNext | BoardAction::Link | BoardAction::ImportMermaid => {
                self.text.set(if action == BoardAction::ImportMermaid {
                    "flowchart LR\n  Browser --> API\n  API --> Database".into()
                } else {
                    String::new()
                });
                self.dialog.set(Some(action));
            }
            BoardAction::Import => {
                if let Some(input) = self.import.get() {
                    input.click();
                }
            }
            BoardAction::Copy => {
                let seat = self.host.seat(&self.id.get_value());
                if let Some(seat) = seat {
                    let state = seat.borrow();
                    match state.input.editor.copy_selection() {
                        Ok(board) => {
                            let payloads = state
                                .payloads
                                .iter()
                                .filter(|(hash, _)| {
                                    board.assets.values().any(|a| &a.sha256 == *hash)
                                })
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect();
                            let portable = PortableBoard { board, payloads };
                            match serde_json::to_string(&portable) {
                                Ok(json) => {
                                    leptos::task::spawn_local(async move {
                                        if let Some(window) = web_sys::window() {
                                            if let Err(e) = wasm_bindgen_futures::JsFuture::from(
                                                window.navigator().clipboard().write_text(
                                                    &format!("bruhs-whiteboard:{json}"),
                                                ),
                                            )
                                            .await
                                            {
                                                self.fail(js_error(e));
                                            }
                                        }
                                    });
                                }
                                Err(e) => self.fail(e),
                            }
                        }
                        Err(e) => self.fail(e),
                    }
                }
            }
            BoardAction::Paste => {
                let alive = self.alive.get_value();
                leptos::task::spawn_local(async move {
                    let Some(window) = web_sys::window() else {
                        return;
                    };
                    match wasm_bindgen_futures::JsFuture::from(
                        window.navigator().clipboard().read_text(),
                    )
                    .await
                    {
                        Ok(value) => {
                            if !alive.load(std::sync::atomic::Ordering::Acquire) {
                                return;
                            }
                            match value
                                .as_string()
                                .and_then(|s| {
                                    s.strip_prefix("bruhs-whiteboard:").map(str::to_owned)
                                })
                                .ok_or_else(|| "Copy shapes from a Bruhs board first.".to_string())
                                .and_then(|s| {
                                    serde_json::from_str::<PortableBoard>(&s)
                                        .map_err(|e| e.to_string())
                                }) {
                                Ok(mut portable) => {
                                    if let Err(e) = portable.validate() {
                                        self.fail(e);
                                        return;
                                    }
                                    if let Some(seat) = self.host.seat(&self.id.get_value()) {
                                        let mut seat = seat.borrow_mut();
                                        if portable.board.project_path
                                            != seat.input.editor.document().project_path
                                        {
                                            portable.board.sources.clear();
                                            for shape in portable.board.shapes.values_mut() {
                                                shape.source_id = None;
                                                shape.links.clear();
                                            }
                                        }
                                        Rc::make_mut(&mut seat.payloads).extend(portable.payloads);
                                        drop(seat);
                                        self.edit(|input| {
                                            input.editor.paste(
                                                &portable.board,
                                                Point::new(24.0, 24.0),
                                                now(),
                                            )
                                        });
                                    }
                                }
                                Err(e) => self.fail(e),
                            }
                        }
                        Err(e) => self.fail(js_error(e)),
                    }
                });
            }
            BoardAction::ExportJson
            | BoardAction::ExportSvg
            | BoardAction::ExportMermaid
            | BoardAction::ExportPng => match self.portable() {
                Ok(portable) => {
                    let frame = self.host.seat(&self.id.get_value()).and_then(|s| {
                        let s = s.borrow();
                        s.input
                            .editor
                            .selection
                            .iter()
                            .any(|id| {
                                s.input
                                    .editor
                                    .document()
                                    .shapes
                                    .get(id)
                                    .is_some_and(|shape| {
                                        matches!(shape.kind, ShapeKind::Frame | ShapeKind::Group)
                                    })
                            })
                            .then(|| s.input.editor.selection.clone())
                    });
                    leptos::task::spawn_local(async move {
                        let result = match action {
                            BoardAction::ExportJson => serde_json::to_vec_pretty(&portable)
                                .map_err(|e| e.to_string())
                                .and_then(|bytes| {
                                    export::download(
                                        "board.bruhs-board.json",
                                        "application/json",
                                        &bytes,
                                    )
                                    .map_err(js_error)
                                }),
                            BoardAction::ExportSvg => {
                                export_svg(&portable.board, &portable.payloads, None)
                                    .map_err(|e| e.to_string())
                                    .and_then(|svg| {
                                        export::download(
                                            "board.svg",
                                            "image/svg+xml",
                                            svg.as_bytes(),
                                        )
                                        .map_err(js_error)
                                    })
                            }
                            BoardAction::ExportMermaid => {
                                export_mermaid_selection(&portable.board, frame.as_ref())
                                    .map_err(|e| e.to_string())
                                    .and_then(|text| {
                                        export::download("board.mmd", "text/plain", text.as_bytes())
                                            .map_err(js_error)
                                    })
                            }
                            _ => match export::png(portable).await {
                                Ok(bytes) => export::download("board.png", "image/png", &bytes)
                                    .map_err(js_error),
                                Err(e) => Err(js_error(e)),
                            },
                        };
                        if let Err(e) = result {
                            self.fail(e);
                        }
                    });
                }
                Err(e) => self.fail(e),
            },
            BoardAction::RefreshSources => self.host.check_sources(&self.id.get_value()),
            _ => {}
        }
    }
    fn apply_dialog(self) {
        let Some(action) = self.dialog.get_untracked() else {
            return;
        };
        let text = self.text.get_untracked();
        self.error.set(None);
        match action {
            BoardAction::EditText => self.edit(|input| input.editor.set_text(text, now())),
            BoardAction::Rename => self.edit(|input| {
                input
                    .editor
                    .edit(vec![BoardOperation::Title { title: text }], now())
            }),
            BoardAction::ConnectNext => self.edit(|input| {
                input.editor.connect_next(text, now())?;
                Ok(())
            }),
            BoardAction::Link => {
                let link = match self.kind.get_untracked().as_str() {
                    "thread" => Ok(BoardLink::Thread { id: text }),
                    "file" => self
                        .start
                        .get_untracked()
                        .parse::<u32>()
                        .and_then(|start| {
                            self.end
                                .get_untracked()
                                .parse::<u32>()
                                .map(|end| (start, end))
                        })
                        .map(|(start_line, end_line)| BoardLink::File {
                            path: text,
                            start_line,
                            end_line,
                        })
                        .map_err(|_| "Enter positive line numbers.".to_string()),
                    "task" => Ok(BoardLink::Task { id: text }),
                    "meeting" => Ok(BoardLink::Meeting { id: text }),
                    "person" => Ok(BoardLink::Person { id: text }),
                    "document" => Ok(BoardLink::Document { id: text }),
                    "pr" => Ok(BoardLink::PullRequest { url: text }),
                    _ => Err("Choose a link type.".into()),
                };
                match link {
                    Ok(link) => self.edit(|input| input.editor.link(link, now())),
                    Err(e) => {
                        self.fail(e);
                        return;
                    }
                }
            }
            BoardAction::ImportMermaid => {
                let project = self.portable().map(|p| p.board.project_path);
                match project.and_then(|project| {
                    import_mermaid(&text, project, "Imported diagram".into(), now())
                }) {
                    Ok(board) => {
                        self.preview.set(Some(PortableBoard {
                            board,
                            payloads: BTreeMap::new(),
                        }));
                        return;
                    }
                    Err(e) => {
                        self.fail(e);
                        return;
                    }
                }
            }
            _ => {}
        }
        self.dialog.set(None);
        if let Some(canvas) = self.canvas.get() {
            let _ = canvas.focus();
        }
    }
}
fn js_error(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| format!("Browser operation failed: {error:?}"))
}

#[component]
fn Editor(id: String) -> impl IntoView {
    let host = expect_context::<WhiteboardHost>();
    let ui = Ui {
        host,
        id: StoredValue::new(id),
        dialog: RwSignal::new(None),
        text: RwSignal::new(String::new()),
        kind: RwSignal::new("thread".into()),
        start: RwSignal::new("1".into()),
        end: RwSignal::new("1".into()),
        error: RwSignal::new(None),
        preview: RwSignal::new(None),
        import_warnings: RwSignal::new(Vec::new()),
        query: RwSignal::new(String::new()),
        commands: RwSignal::new(false),
        image: NodeRef::new(),
        import: NodeRef::new(),
        canvas: NodeRef::new(),
        command_input: NodeRef::new(),
        text_input: NodeRef::new(),
        alive: StoredValue::new(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            true,
        ))),
    };
    let alive = ui.alive.get_value();
    on_cleanup(move || alive.store(false, std::sync::atomic::Ordering::Release));
    Effect::new(move |_| {
        if ui.commands.get() {
            if let Some(input) = ui.command_input.get() {
                let _ = input.focus();
            }
        }
        if ui.dialog.get().is_some() {
            if let Some(input) = ui.text_input.get() {
                let _ = input.focus();
            }
        }
    });
    leptos::task::spawn_local(async move {
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            let result = wasm_bindgen_futures::JsFuture::from(
                document.fonts().load("18px MonaspaceArgonVariable"),
            )
            .await;
            if let Err(e) = result {
                ui.fail(js_error(e));
            }
            host.notify();
        }
    });
    let focused_id = ui.id.get_value();
    let source_focus =
        window_event_listener(leptos::ev::focus, move |_| host.check_sources(&focused_id));
    on_cleanup(move || source_focus.remove());
    let painter = StoredValue::new_local(None::<BoardCanvas>);
    Effect::new(move |_| {
        host.changed.track();
        let Some(node) = ui.canvas.get() else { return };
        let Some(parent) = node.parent_element() else {
            return;
        };
        let width = f64::from(parent.client_width());
        let height = f64::from(parent.client_height());
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let Some(seat) = host.seat(&ui.id.get_value()) else {
            return;
        };
        let state = seat.borrow();
        let result=painter.try_update_value(|slot: &mut Option<BoardCanvas>|->Result<(),JsValue>{if slot.is_none(){let raw:&web_sys::HtmlCanvasElement=&node;*slot=Some(BoardCanvas::new(raw.clone(),width,height,web_sys::window().map_or(1.0,|w|w.device_pixel_ratio()))?);}
if let Some(canvas)=slot{let dpr=web_sys::window().map_or(1.0,|w|w.device_pixel_ratio());if canvas.width!=width||canvas.height!=height||canvas.dpr!=dpr{canvas.resize(width,height,dpr)?;}canvas.install_assets(state.input.editor.document(),&state.payloads,Rc::new(move||host.notify()))?;canvas.draw(&state.input.editor)?;if canvas.failed_images()>0{return Err(JsValue::from_str("An image could not be decoded. Replace it or export the portable board for recovery."))}}Ok(())});
        if let Some(Err(e)) = result {
            ui.fail(js_error(e));
        }
    });
    let observer = StoredValue::new_local(
        None::<(
            web_sys::ResizeObserver,
            wasm_bindgen::closure::Closure<dyn FnMut(js_sys::Array, web_sys::ResizeObserver)>,
        )>,
    );
    Effect::new(move |_| {
        if observer.with_value(Option::is_some) {
            return;
        }
        if let Some(node) = ui.canvas.get() {
            if let Some(parent) = node.parent_element() {
                let callback = wasm_bindgen::closure::Closure::wrap(Box::new(
                    move |_: js_sys::Array, _: web_sys::ResizeObserver| host.notify(),
                )
                    as Box<dyn FnMut(_, _)>);
                if let Ok(resize) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref())
                {
                    resize.observe(&parent);
                    observer.set_value(Some((resize, callback)));
                }
            }
        }
    });
    on_cleanup(move || {
        if let Some((observer, _callback)) = observer.try_update_value(Option::take).flatten() {
            observer.disconnect();
        }
        painter.try_update_value(|p| *p = None);
    });
    let position = move |event: &web_sys::PointerEvent| {
        ui.canvas.get().map(|canvas| {
            let r = canvas.get_bounding_client_rect();
            Point::new(
                f64::from(event.client_x()) - r.x(),
                f64::from(event.client_y()) - r.y(),
            )
        })
    };
    let keydown = move |event: web_sys::KeyboardEvent| {
        let command = event.meta_key() || event.ctrl_key();
        let key = event.key();
        let lower = key.to_lowercase();
        if command {
            let action = match lower.as_str() {
                "k" => {
                    event.prevent_default();
                    event.stop_propagation();
                    ui.commands.set(true);
                    return;
                }
                "z" => Some(if event.shift_key() {
                    BoardAction::Redo
                } else {
                    BoardAction::Undo
                }),
                "a" => Some(BoardAction::SelectAll),
                "c" => Some(BoardAction::Copy),
                "v" => Some(BoardAction::Paste),
                "s" => Some(BoardAction::Save),
                "g" => Some(if event.shift_key() {
                    BoardAction::Ungroup
                } else {
                    BoardAction::Group
                }),
                _ => None,
            };
            if let Some(action) = action {
                event.prevent_default();
                event.stop_propagation();
                ui.act(action);
            }
            return;
        }
        match key.as_str() {
            "Escape" => {
                event.prevent_default();
                ui.edit(|input| {
                    input.cancel();
                    Ok(())
                });
            }
            "Delete" | "Backspace" => {
                event.prevent_default();
                ui.act(BoardAction::Delete);
            }
            "Enter" => {
                event.prevent_default();
                let kind = host
                    .seat(&ui.id.get_value())
                    .and_then(|s| s.borrow().input.tool.kind());
                if let Some(kind) = kind {
                    if let Some(canvas) = ui.canvas.get() {
                        let area = canvas.get_bounding_client_rect();
                        ui.edit(|input| {
                            let at = input.editor.camera.world(Point::new(
                                area.width() / 2.0 - 90.0,
                                area.height() / 2.0 - 40.0,
                            ));
                            input.editor.add_node(kind, at, String::new(), now())?;
                            input.set_tool(BoardTool::Select);
                            Ok(())
                        });
                    }
                }
                ui.act(BoardAction::EditText);
            }
            "Tab" => {
                event.prevent_default();
                ui.edit(|input| {
                    input.editor.focus_next(event.shift_key());
                    Ok(())
                });
            }
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" => {
                event.prevent_default();
                let amount = if event.shift_key() { 10.0 } else { 1.0 };
                let delta = match key.as_str() {
                    "ArrowLeft" => Point::new(-amount, 0.0),
                    "ArrowRight" => Point::new(amount, 0.0),
                    "ArrowUp" => Point::new(0.0, -amount),
                    _ => Point::new(0.0, amount),
                };
                ui.edit(|input| input.editor.nudge(delta, now()));
            }
            "c" => {
                event.prevent_default();
                ui.act(BoardAction::ConnectNext);
            }
            _ => {
                if let Some(tool) = BoardTool::ALL.into_iter().find(|tool| tool.key() == lower) {
                    event.prevent_default();
                    if tool == BoardTool::Image {
                        if let Some(input) = ui.image.get() {
                            input.click();
                        }
                    } else {
                        ui.edit(|input| {
                            input.set_tool(tool);
                            Ok(())
                        });
                    }
                } else if key.chars().count() == 1 && !event.alt_key() {
                    event.prevent_default();
                    ui.dialog.set(Some(BoardAction::ConnectNext));
                    ui.text.set(key);
                }
            }
        }
    };
    let project_caption = host
        .seat(&ui.id.get_value())
        .map(|s| s.borrow().input.editor.document().project_path.clone())
        .unwrap_or_default();
    let project_tooltip = project_caption.clone();
    view! {<div class="relative flex min-h-0 flex-1 flex-col" data-whiteboard-editor>
            <header class="flex flex-wrap items-center gap-2 border-b border-longan-800 px-3 py-2"><button class=BUTTON on:click=move|_|host.route(None)>"Boards"</button><button class=BUTTON on:click=move|_|ui.act(BoardAction::Rename)>{move||{host.changed.track();host.seat(&ui.id.get_value()).map(|s|s.borrow().input.editor.document().title.clone()).unwrap_or_default()}}</button><span role="status" class="min-w-0 flex-1 text-label-sm text-rambutan-500">{move||{host.changed.track();host.seat(&ui.id.get_value()).map_or("Opening…",|s|{let s=s.borrow();if s.error.is_some(){"Save needs attention"}else if s.writing{"Saving…"}else if s.saved==Some(s.input.editor.document().revision){"Saved on this device"}else{"Unsaved changes"}})}}</span><button class=BUTTON on:click=move|_|ui.act(BoardAction::Share)>{move||{host.changed.track();if host.seat(&ui.id.get_value()).is_some_and(|s|s.borrow().input.editor.document().agent_read){"Shared with agents"}else{"Private to you"}}}</button><button class=BUTTON on:click=move|_|ui.commands.set(true)>"Commands ⌘K"</button></header>
            <div class="flex shrink-0 gap-1 overflow-x-auto border-b border-longan-800 p-2" role="toolbar" aria-label="Drawing tools">{BoardTool::ALL.into_iter().map(move|tool|view!{<button class=BUTTON aria-label=format!("{} ({})",tool.label(),tool.key()) aria-pressed=move||{host.changed.track();host.seat(&ui.id.get_value()).is_some_and(|s|s.borrow().input.tool==tool)} on:click=move|_|{if tool==BoardTool::Image{if let Some(input)=ui.image.get(){input.click();}}else{ui.edit(|input|{input.set_tool(tool);Ok(())});}
    if let Some(canvas)=ui.canvas.get(){let _=canvas.focus();}}>{tool.label()}</button>}).collect_view()}<button class=BUTTON on:click=move|_|ui.act(BoardAction::Fit)>"Fit"</button><button class=BUTTON on:click=move|_|ui.act(BoardAction::Undo)>"Undo"</button><button class=BUTTON on:click=move|_|ui.act(BoardAction::EditText)>"Edit text"</button><button class=BUTTON on:click=move|_|ui.act(BoardAction::Link)>"Link"</button></div>
            {move||{host.changed.track();host.seat(&ui.id.get_value()).and_then(|s|s.borrow().error.clone()).map(|error|view!{<div role="alert" class="flex flex-wrap items-center gap-2 border-b border-lychee-500/30 p-2 text-body-sm text-lychee-300"><span>{error}</span><button class=BUTTON on:click=move|_|ui.act(BoardAction::Save)>"Retry save"</button><button class=BUTTON on:click=move|_|ui.act(BoardAction::SaveCopy)>"Save a copy"</button><button class=BUTTON on:click=move|_|ui.act(BoardAction::ExportJson)>"Export recovery copy"</button></div>})}}
            <div class="relative min-h-0 flex-1 overflow-hidden"><canvas node_ref=ui.canvas tabindex="0" aria-label="Whiteboard canvas. R rectangle, A arrow, T text, F frame, C connect next, Enter edit, Tab next node, arrows move, Command K actions." class="block h-full w-full touch-none outline-none" on:keydown=keydown on:contextmenu=move|event|event.prevent_default() on:pointerdown=move|event:web_sys::PointerEvent|{event.prevent_default();if let Some(canvas)=ui.canvas.get(){let _=canvas.focus();let _=canvas.set_pointer_capture(event.pointer_id());}
    if let Some(point)=position(&event){ui.edit(|input|input.pointer_down(point,event.shift_key(),event.button()==1||event.button()==2));}} on:pointermove=move|event:web_sys::PointerEvent|{if let Some(point)=position(&event){ui.edit(|input|input.pointer_move(point));}} on:pointerup=move|event:web_sys::PointerEvent|{if let Some(point)=position(&event){ui.edit(|input|{input.pointer_up(point,now())?;Ok(())});}
    if let Some(canvas)=ui.canvas.get(){let _=canvas.release_pointer_capture(event.pointer_id());}} on:pointercancel=move|_|ui.edit(|input|{input.cancel();Ok(())}) on:wheel=move|event:web_sys::WheelEvent|{event.prevent_default();if let Some(canvas)=ui.canvas.get(){let rect=canvas.get_bounding_client_rect();ui.edit(|input|{if event.ctrl_key()||event.meta_key(){let zoom=(input.editor.camera.zoom*(-event.delta_y()*0.002).exp()).clamp(0.05,8.0);input.editor.camera.zoom_at(Point::new(f64::from(event.client_x())-rect.x(),f64::from(event.client_y())-rect.y()),zoom)?;}else{input.editor.camera.x+=event.delta_x()/input.editor.camera.zoom;input.editor.camera.y+=event.delta_y()/input.editor.camera.zoom;}Ok(())});}}></canvas></div>
            <div class="flex flex-wrap items-center gap-2 border-t border-longan-800 px-3 py-2 text-label-sm text-rambutan-500"><span>"C connects the next node · Enter edits · Tab focuses · arrows move"</span><span aria-live="polite">{move||{host.changed.track();host.seat(&ui.id.get_value()).map(|s|selection_caption(&s.borrow().input.editor))}}</span>{move||{host.changed.track();host.seat(&ui.id.get_value()).map(|s|{let s=s.borrow();format!("{} shapes · {:.0}%",s.input.editor.document().shapes.len(),s.input.editor.camera.zoom*100.0)})}}{move||{host.changed.track();host.seat(&ui.id.get_value()).map(|seat|{let seat=seat.borrow();seat.input.editor.selection.iter().filter_map(|id|seat.input.editor.document().shapes.get(id)).flat_map(|s|s.links.clone()).map(|link|{let label=link.label();let project=seat.input.editor.document().project_path.clone();view!{<button class=BUTTON on:click=move|_|open_link(&link,&project)>{label}</button>}}).collect_view()})}}</div>
            {move||{host.changed.track();host.seat(&ui.id.get_value()).map(|s|{let s=s.borrow();s.input.editor.document().sources.values().map(|source|view!{<p class="px-3 py-1 text-label-sm text-rambutan-400">{format!("{} · {}",source.path,if source.stale{"changed or unavailable"}else{"source matches"})}</p>}).collect_view()})}}
            <p class="min-w-0 truncate px-3 py-1 text-label-sm text-rambutan-500" title=project_tooltip>{project_caption}</p>
        <super::review::ProposalReviews id=ui.id.get_value()/>
            <input node_ref=ui.image type="file" class="hidden" accept="image/png,image/jpeg,image/webp,image/gif" aria-label="Local whiteboard image" on:change=move|_|read_image(ui)/>
            <input node_ref=ui.import type="file" class="hidden" accept=".json,.tldr,.excalidraw,.mmd" aria-label="Import whiteboard file" on:change=move|_|read_board(ui)/>
            <Dialog open=ui.commands label="Whiteboard commands" on_close=Callback::new(move|_|ui.commands.set(false)) max_width="max-w-lg"><div class="max-h-[calc(100dvh-6rem)] w-full max-w-lg overflow-auto rounded-grape border border-longan-700 bg-longan-900 p-3"><input node_ref=ui.command_input class=FIELD on:keydown=move|event:web_sys::KeyboardEvent|{if event.key()=="Escape"{event.stop_propagation();ui.commands.set(false);}else if event.key()=="Enter"{event.prevent_default();let query=ui.query.get_untracked().to_lowercase();if let Some(action)=BoardCommand::all().find(|a|a.label().to_lowercase().contains(&query)){ui.command(action);}}} placeholder="Find a board action" aria-label="Find a board action" prop:value=move||ui.query.get() on:input=move|e|ui.query.set(event_target_value(&e))/><button class=BUTTON on:click=move|_|ui.commands.set(false)>"Close"</button>{BoardCommand::all().map(move|action|view!{<Show when=move||action.label().to_lowercase().contains(&ui.query.get().to_lowercase())><button class="bruhs-control block min-h-control w-full rounded-grape px-3 text-left text-label hover:bg-longan-800" on:click=move|_|ui.command(action)>{action.label()}</button></Show>}).collect_view()}</div></Dialog>
            <Dialog open=Signal::derive(move||ui.dialog.get().is_some()) label="Board edit" on_close=Callback::new(move|_|ui.dialog.set(None)) max_width="max-w-xl"><div class="flex max-h-[calc(100dvh-6rem)] overflow-auto w-full max-w-xl flex-col gap-3 rounded-grape border border-longan-700 bg-longan-900 p-4"><h2 class="text-h4">{move||ui.dialog.get().map_or("Board edit",BoardAction::label)}</h2><Show when=move||ui.dialog.get()==Some(BoardAction::Link)><select class=FIELD aria-label="Link type" on:change=move|e|ui.kind.set(event_target_value(&e))><option value="thread">"Thread"</option><option value="file">"Project file"</option><option value="task">"Task"</option><option value="pr">"Pull request"</option><option value="meeting">"Meeting"</option><option value="person">"Person"</option><option value="document">"Document"</option></select><Show when=move||ui.kind.get()=="file"><label>"Start line"<input class=FIELD type="number" min="1" prop:value=move||ui.start.get() on:input=move|e|ui.start.set(event_target_value(&e))/></label><label>"End line"<input class=FIELD type="number" min="1" prop:value=move||ui.end.get() on:input=move|e|ui.end.set(event_target_value(&e))/></label></Show></Show><textarea node_ref=ui.text_input class=FIELD rows="5" aria-label="Board edit text" prop:value=move||ui.text.get() on:input=move|e|ui.text.set(event_target_value(&e)) on:keydown=move|e:web_sys::KeyboardEvent|{if e.key()=="Escape"{ui.dialog.set(None);}
    if e.key()=="Enter"&&(e.ctrl_key()||e.meta_key()){e.prevent_default();ui.apply_dialog();}}></textarea><div class="flex gap-2"><button class=BUTTON on:click=move|_|ui.apply_dialog()>{move||if ui.dialog.get()==Some(BoardAction::ImportMermaid){"Preview shapes"}else{"Apply"}}</button><button class=BUTTON on:click=move|_|ui.dialog.set(None)>"Cancel"</button></div></div></Dialog>
            {move||ui.preview.get().map(|portable|{let svg=export_svg(&portable.board,&portable.payloads,None).unwrap_or_default();view!{<Dialog open=Signal::derive(move||ui.preview.get().is_some()) label="Imported board preview" on_close=Callback::new(move|_|ui.preview.set(None)) max_width="max-w-3xl"><div class="flex max-h-[calc(100dvh-6rem)] w-full max-w-3xl flex-col gap-3 overflow-auto rounded-grape border border-longan-700 bg-longan-900 p-4"><h2 class="text-h4">"Review imported shapes"</h2>{move||ui.import_warnings.get().into_iter().map(|w|view!{<p class="text-body-sm text-rambutan-300">{w}</p>}).collect_view()}<p class="text-body-sm text-rambutan-400">{format!("{} shapes. A new private board will be saved in this project.",portable.board.shapes.len())}</p><div class="whiteboard-preview overflow-hidden border border-longan-700" inner_html=svg.clone()></div><button class=BUTTON on:click=move|_|{if let Some(portable)=ui.preview.get_untracked(){let id=portable.board.id.clone();match host.insert(portable,None){Ok(_)=>{host.save(&id,false);host.route(Some(id));ui.preview.set(None);ui.dialog.set(None);},Err(e)=>ui.fail(e)}}}>"Save as new board"</button><button class=BUTTON on:click=move|_|ui.preview.set(None)>"Cancel"</button></div></Dialog>}})}
            {move||ui.error.get().map(|error|view!{<div role="alert" class="absolute bottom-10 left-3 right-3 z-40 rounded-grape border border-lychee-500 bg-longan-900 p-3 text-body-sm text-lychee-300">{error}<button class=BUTTON on:click=move|_|ui.error.set(None)>"Dismiss"</button></div>})}
        </div>}
}
fn read_image(ui: Ui) {
    let alive = ui.alive.get_value();
    let Some(file) = ui
        .image
        .get()
        .and_then(|input| input.files())
        .and_then(|files| files.get(0))
    else {
        return;
    };
    leptos::task::spawn_local(async move {
        let result = async {
            if file.size()
                > bruhs_models::num::ToFloatLossy::to_f64_lossy(
                    bruhs_models::logic::attachments::MAX_IMAGE_BYTES,
                )
            {
                return Err("This image exceeds the attachment image budget.".into());
            }
            let buffer = wasm_bindgen_futures::JsFuture::from(file.array_buffer())
                .await
                .map_err(js_error)?;
            if !alive.load(std::sync::atomic::Ordering::Acquire) {
                return Ok(());
            }
            let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
            let (asset, payload) = AssetPayload::from_bytes(&bytes).map_err(|e| e.to_string())?;
            ui.host
                .insert_image(&ui.id.get_value(), asset, payload)
                .map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        }
        .await;
        if let Err(e) = result {
            ui.fail(e);
        }
    });
}
fn read_board(ui: Ui) {
    let alive = ui.alive.get_value();
    let Some(file) = ui
        .import
        .get()
        .and_then(|input| input.files())
        .and_then(|files| files.get(0))
    else {
        return;
    };
    leptos::task::spawn_local(async move {
        if file.size() > bruhs_models::num::ToFloatLossy::to_f64_lossy(MAX_PORTABLE_BYTES) {
            ui.fail("This portable board exceeds the local content budget.");
            return;
        }
        match wasm_bindgen_futures::JsFuture::from(file.text()).await {
            Ok(value) => {
                if !alive.load(std::sync::atomic::Ordering::Acquire) {
                    return;
                }
                let result = value
                    .as_string()
                    .ok_or_else(|| "The file is not text.".to_string())
                    .and_then(|text| {
                        let project = ui.portable().map_err(|e| e.to_string())?.board.project_path;
                        import_board_file(&text, &file.name(), project, now())
                            .map_err(|e| e.to_string())
                    });
                match result {
                    Ok(imported) => {
                        ui.import_warnings.set(imported.warnings);
                        ui.preview.set(Some(imported.portable));
                    }
                    Err(e) => ui.fail(e),
                }
            }
            Err(e) => ui.fail(js_error(e)),
        }
    });
}
fn open_link(link: &BoardLink, project: &str) {
    match link {
        BoardLink::Thread { id } => {
            expect_context::<SetActiveThreadId>()
                .0
                .set(Some(id.clone()));
            expect_context::<SetActiveView>().0.set(AppView::Chat);
        }
        BoardLink::File {
            path,
            start_line,
            end_line,
        } => crate::components::file_attachments::show_open_file(
            expect_context::<OpenFileSignal>().0,
            expect_context::<RightPanelSignal>().0,
            path.clone(),
            format!("{path}:{start_line}–{end_line}"),
            Some(project.into()),
            true,
        ),
        BoardLink::Task { id } => {
            expect_context::<SetSelectedWorkItem>()
                .0
                .set(Some(id.clone()));
            expect_context::<SetActiveView>().0.set(AppView::Tasks);
        }
        BoardLink::Meeting { id } => {
            expect_context::<SetActiveView>()
                .0
                .set(AppView::MeetingDetail {
                    meeting_id: id.clone(),
                })
        }
        BoardLink::Person { id } => {
            expect_context::<SelectedPersonSignal>()
                .0
                .set(Some(id.clone()));
            expect_context::<SetActiveView>().0.set(AppView::People);
        }
        BoardLink::Document { id } => {
            expect_context::<SetActiveView>()
                .0
                .set(AppView::DocumentDetail {
                    document_id: id.clone(),
                })
        }
        BoardLink::PullRequest { url } => {
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target_and_features(
                    url,
                    "_blank",
                    "noopener,noreferrer",
                );
            }
        }
    }
}
