//! Native canvas and input, shared by the full editor and isolated frame probe.
pub(crate) mod canvas;
pub(crate) mod catalog;
use bruhs_models::whiteboard as board;
use gpui::{prelude::*, *};
use gpui_component::zoom::px;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone)]
pub(crate) enum BoardCanvasEvent {
    Changed,
    Action(board::BoardAction),
    Commands,
    BeginText(String),
    Image,
}
pub(crate) struct WhiteboardView {
    pub input: Rc<RefCell<board::BoardInput>>,
    painter: Rc<RefCell<canvas::BoardPainter>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    focus: Option<FocusHandle>,
    dragging: bool,
    fit_pending: Rc<Cell<bool>>,
    pub frozen: bool,
    pub error: Option<String>,
}
impl EventEmitter<BoardCanvasEvent> for WhiteboardView {}
impl WhiteboardView {
    pub fn new(editor: board::BoardEditor) -> Self {
        Self {
            input: Rc::new(RefCell::new(board::BoardInput::new(editor))),
            painter: Rc::new(RefCell::new(canvas::BoardPainter::default())),
            bounds: Rc::new(Cell::new(Bounds::default())),
            focus: None,
            dragging: false,
            fit_pending: Rc::new(Cell::new(false)),
            frozen: false,
            error: None,
        }
    }
    pub fn focus(&mut self, window: &mut Window, cx: &mut App) {
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        window.focus(&focus, cx);
    }
    pub fn install_images(&mut self, images: canvas::BoardImages, cx: &mut Context<'_, Self>) {
        self.painter.borrow_mut().images = images;
        cx.notify();
    }
    pub fn images_current(&self, document: &board::BoardDocument) -> bool {
        self.painter.borrow().images.current(document)
    }
    pub fn images(&self) -> canvas::BoardImages {
        self.painter.borrow().images.clone()
    }
    pub fn center(&self) -> board::Point {
        let area = self.bounds.get();
        self.input.borrow().editor.camera.world(board::Point::new(
            f64::from(f32::from(area.size.width)) / 2.0,
            f64::from(f32::from(area.size.height)) / 2.0,
        ))
    }
    fn point(&self, position: Point<Pixels>) -> board::Point {
        let origin = self.bounds.get().origin;
        board::Point::new(
            f64::from(f32::from(position.x - origin.x)),
            f64::from(f32::from(position.y - origin.y)),
        )
    }
    pub fn edit(
        &mut self,
        edit: impl FnOnce(&mut board::BoardInput) -> board::BoardResult<()>,
        cx: &mut Context<'_, Self>,
    ) {
        if self.frozen {
            return;
        }
        let mut input = self.input.borrow_mut();
        let before = input.editor.document().revision;
        match edit(&mut input) {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error.to_string()),
        }
        let changed = input.editor.document().revision != before;
        drop(input);
        if changed {
            cx.emit(BoardCanvasEvent::Changed);
        }
        cx.notify();
    }
    pub fn action(&mut self, action: board::BoardAction, cx: &mut Context<'_, Self>) {
        let mut handled = false;
        self.edit(
            |input| {
                handled = action.apply(input, chrono::Utc::now().timestamp_millis())?;
                Ok(())
            },
            cx,
        );
        if !handled {
            cx.emit(BoardCanvasEvent::Action(action));
        }
    }
    pub fn set_tool(&mut self, tool: board::BoardTool, cx: &mut Context<'_, Self>) {
        self.edit(
            |input| {
                input.set_tool(tool);
                Ok(())
            },
            cx,
        );
    }
    pub fn fit(&mut self, cx: &mut Context<'_, Self>) {
        let area = self.bounds.get();
        if area.size.width > px(0.0) && area.size.height > px(0.0) {
            if let Err(e) = self.input.borrow_mut().fit(
                f64::from(f32::from(area.size.width)),
                f64::from(f32::from(area.size.height)),
            ) {
                self.error = Some(e.to_string());
            }
        } else {
            self.fit_pending.set(true);
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.frozen {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if modifiers.platform || modifiers.control {
            let action = match key {
                "k" => {
                    window.prevent_default();
                    cx.stop_propagation();
                    cx.emit(BoardCanvasEvent::Commands);
                    return;
                }
                "z" => Some(if modifiers.shift {
                    board::BoardAction::Redo
                } else {
                    board::BoardAction::Undo
                }),
                "a" => Some(board::BoardAction::SelectAll),
                "c" => Some(board::BoardAction::Copy),
                "v" => Some(board::BoardAction::Paste),
                "s" => Some(board::BoardAction::Save),
                "g" => Some(if modifiers.shift {
                    board::BoardAction::Ungroup
                } else {
                    board::BoardAction::Group
                }),
                _ => None,
            };
            if let Some(action) = action {
                window.prevent_default();
                cx.stop_propagation();
                self.action(action, cx);
            }
            return;
        }
        match key {
            "escape" => {
                self.dragging = false;
                self.edit(
                    |input| {
                        input.cancel();
                        Ok(())
                    },
                    cx,
                );
            }
            "delete" | "backspace" => self.action(board::BoardAction::Delete, cx),
            "enter" => {
                let kind = self.input.borrow().tool.kind();
                if let Some(kind) = kind {
                    let area = self.bounds.get();
                    self.edit(
                        |input| {
                            let at = input.editor.camera.world(board::Point::new(
                                f64::from(f32::from(area.size.width)) / 2.0 - 90.0,
                                f64::from(f32::from(area.size.height)) / 2.0 - 40.0,
                            ));
                            input.editor.add_node(
                                kind,
                                at,
                                String::new(),
                                chrono::Utc::now().timestamp_millis(),
                            )?;
                            input.set_tool(board::BoardTool::Select);
                            Ok(())
                        },
                        cx,
                    );
                }
                self.action(board::BoardAction::EditText, cx);
            }
            "tab" => self.edit(
                |input| {
                    input.editor.focus_next(modifiers.shift);
                    Ok(())
                },
                cx,
            ),
            "left" | "right" | "up" | "down" => {
                let amount = if modifiers.shift { 10.0 } else { 1.0 };
                let delta = match key {
                    "left" => board::Point::new(-amount, 0.0),
                    "right" => board::Point::new(amount, 0.0),
                    "up" => board::Point::new(0.0, -amount),
                    _ => board::Point::new(0.0, amount),
                };
                self.edit(
                    |input| {
                        input
                            .editor
                            .nudge(delta, chrono::Utc::now().timestamp_millis())
                    },
                    cx,
                );
            }
            "c" => self.action(board::BoardAction::ConnectNext, cx),
            _ => {
                if let Some(tool) = board::BoardTool::ALL
                    .into_iter()
                    .find(|tool| tool.key() == key)
                {
                    if tool == board::BoardTool::Image {
                        cx.emit(BoardCanvasEvent::Image);
                    } else {
                        self.set_tool(tool, cx);
                    }
                } else if key.chars().count() == 1 && !modifiers.alt {
                    cx.emit(BoardCanvasEvent::BeginText(key.to_string()));
                } else {
                    return;
                }
            }
        }
        window.prevent_default();
        cx.stop_propagation();
    }
}
impl Render for WhiteboardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let focus = self.focus.get_or_insert_with(|| cx.focus_handle()).clone();
        let input = self.input.clone();
        let painter = self.painter.clone();
        let bounds = self.bounds.clone();
        let fit_pending = self.fit_pending.clone();
        let owner = cx.entity().downgrade();
        let surface = canvas(
            |bounds, _, _| bounds,
            move |_, area, window, cx| {
                bounds.set(area);
                if fit_pending.replace(false) {
                    if let Err(error) = input.borrow_mut().fit(
                        f64::from(f32::from(area.size.width)),
                        f64::from(f32::from(area.size.height)),
                    ) {
                        tracing::warn!(%error,"Whiteboard fit failed");
                    }
                }
                painter
                    .borrow_mut()
                    .draw(&input.borrow().editor, area, window, cx);
                let view = owner.clone();
                window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = view.update(cx, |view, cx| {
                        if view.dragging {
                            let at = view.point(event.position);
                            view.edit(|input| input.pointer_move(at), cx);
                            cx.stop_propagation();
                        }
                    });
                });
                let view = owner.clone();
                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Bubble {
                        return;
                    }
                    let _ = view.update(cx, |view, cx| {
                        if view.dragging {
                            view.dragging = false;
                            let at = view.point(event.position);
                            view.edit(
                                |input| {
                                    input.pointer_up(at, chrono::Utc::now().timestamp_millis())?;
                                    Ok(())
                                },
                                cx,
                            );
                            cx.stop_propagation();
                        }
                    });
                });
            },
        )
        .size_full();
        div().id("native-whiteboard-canvas").debug_selector(||"native-whiteboard-canvas".into()).size_full().min_h_0().min_w_0().track_focus(&focus).key_context("Whiteboard").role(Role::Group).aria_label("Whiteboard canvas. R rectangle, A arrow, T text, F frame, C connect next, Enter edit, Tab next node, arrows move, Command K actions.").capture_key_down(cx.listener(Self::key))
            .on_mouse_down(MouseButton::Left,cx.listener(|view,event:&MouseDownEvent,window,cx|{view.focus(window,cx);if view.frozen{return}view.dragging=true;let at=view.point(event.position);view.edit(|input|input.pointer_down(at,event.modifiers.shift,false),cx);cx.stop_propagation();}))
            .on_mouse_down(MouseButton::Middle,cx.listener(|view,event:&MouseDownEvent,window,cx|{view.focus(window,cx);view.dragging=true;let at=view.point(event.position);view.edit(|input|input.pointer_down(at,false,true),cx);cx.stop_propagation();}))
            .on_scroll(cx.listener(|view,event:&ScrollEvent,_,cx|{let delta=event.delta.pixel_delta(px(1.0));let at=view.point(event.position);view.edit(|input|{if event.modifiers.control||event.modifiers.platform{let zoom=(input.editor.camera.zoom*(-f64::from(f32::from(delta.y))*0.002).exp()).clamp(0.05,8.0);input.editor.camera.zoom_at(at,zoom)?;}else{input.editor.camera.x-=f64::from(f32::from(delta.x))/input.editor.camera.zoom;input.editor.camera.y-=f64::from(f32::from(delta.y))/input.editor.camera.zoom;}Ok(())},cx);cx.stop_propagation();})).child(surface)
    }
}
