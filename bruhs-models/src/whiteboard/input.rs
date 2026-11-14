//! Platform-neutral pointer gestures. Durable edits happen only at gesture end.
use super::*;
use std::collections::BTreeSet;

enum Gesture {
    Pan {
        screen: Point,
        camera: Camera,
    },
    Move {
        world: Point,
    },
    Draw {
        world: Point,
    },
    Box {
        world: Point,
        previous: BTreeSet<ShapeId>,
    },
}
pub struct BoardInput {
    pub editor: BoardEditor,
    pub tool: BoardTool,
    pub snapping: bool,
    gesture: Option<Gesture>,
}
impl BoardInput {
    pub fn new(editor: BoardEditor) -> Self {
        Self {
            editor,
            tool: BoardTool::Select,
            snapping: true,
            gesture: None,
        }
    }
    pub fn cancel(&mut self) {
        self.gesture = None;
        self.editor.cancel_preview();
    }
    pub fn set_tool(&mut self, tool: BoardTool) {
        self.cancel();
        self.tool = tool;
    }
    pub fn pointer_down(&mut self, screen: Point, additive: bool, pan: bool) -> BoardResult<()> {
        if !screen.finite() {
            return Err(BoardError::Invalid(
                "The pointer position is invalid.".into(),
            ));
        }
        self.cancel();
        let world = self.editor.camera.world(screen);
        if pan || self.tool == BoardTool::Hand {
            self.gesture = Some(Gesture::Pan {
                screen,
                camera: self.editor.camera,
            });
            return Ok(());
        }
        if self.tool == BoardTool::Select {
            if let Some(id) = self.editor.hit(world, 6.0 / self.editor.camera.zoom) {
                if additive && self.editor.selection.contains(&id) {
                    self.editor.selection.remove(&id);
                    return Ok(());
                }
                if additive {
                    self.editor.selection.insert(id);
                } else if !self.editor.selection.contains(&id) {
                    self.editor.selection = BTreeSet::from([id]);
                }
                self.editor.start_move();
                self.gesture = Some(Gesture::Move { world });
            } else {
                let previous = if additive {
                    self.editor.selection.clone()
                } else {
                    BTreeSet::new()
                };
                self.editor.selection = previous.clone();
                self.editor.selection_box = Some(Rect::new(world.x, world.y, 0.0, 0.0));
                self.gesture = Some(Gesture::Box { world, previous });
            }
            return Ok(());
        }
        if self.tool == BoardTool::Image {
            return Err(BoardError::Invalid(
                "Choose a local image with the Image action.".into(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let kind = if self.tool == BoardTool::Arrow {
            ShapeKind::Arrow {
                start: bind_at(&self.editor, world),
                end: Endpoint::free(world),
            }
        } else if self.tool == BoardTool::Freehand {
            ShapeKind::Freehand {
                points: vec![world, world],
            }
        } else {
            self.tool
                .kind()
                .ok_or_else(|| BoardError::Invalid("Choose a drawing tool.".into()))?
        };
        let mut shape = Shape::node(
            id,
            kind,
            Rect::new(world.x, world.y, 0.0, 0.0),
            self.editor.next_z()?,
            String::new(),
        );
        if matches!(
            shape.kind,
            ShapeKind::Text
                | ShapeKind::Arrow { .. }
                | ShapeKind::Freehand { .. }
                | ShapeKind::Frame
        ) {
            shape.style.fill = None;
        }
        if matches!(shape.kind, ShapeKind::Note) {
            shape.style.fill = Some(Color::rgb(250, 226, 150));
        }
        self.editor.draft = Some(shape);
        self.gesture = Some(Gesture::Draw { world });
        Ok(())
    }
    pub fn pointer_move(&mut self, screen: Point) -> BoardResult<()> {
        if !screen.finite() {
            return Err(BoardError::Invalid(
                "The pointer position is invalid.".into(),
            ));
        }
        let Some(gesture) = &self.gesture else {
            return Ok(());
        };
        let at = self.editor.camera.world(screen);
        match gesture {
            Gesture::Pan {
                screen: origin,
                camera,
            } => {
                self.editor.camera.x = camera.x - (screen.x - origin.x) / camera.zoom;
                self.editor.camera.y = camera.y - (screen.y - origin.y) / camera.zoom;
            }
            Gesture::Move { world } => {
                let mut delta = Point::new(at.x - world.x, at.y - world.y);
                if self.snapping {
                    delta = snap(delta, 8.0)?;
                }
                self.editor.preview_move(delta)?;
            }
            Gesture::Box { world, previous } => {
                let rect = between(*world, at);
                self.editor.selection = self
                    .editor
                    .document()
                    .shapes
                    .values()
                    .filter(|shape| {
                        !matches!(shape.kind, ShapeKind::Group)
                            && bounds(self.editor.document(), shape).intersects(rect)
                    })
                    .map(|s| s.id.clone())
                    .chain(previous.iter().cloned())
                    .collect();
                self.editor.selection_box = Some(rect);
            }
            Gesture::Draw { world } => {
                let mut end = at;
                if self.snapping && self.tool != BoardTool::Freehand {
                    end = snap(at, 8.0)?;
                }
                let endpoint = if self.tool == BoardTool::Arrow {
                    Some(bind_at(&self.editor, at))
                } else {
                    None
                };
                if let Some(draft) = &mut self.editor.draft {
                    draft.rect = between(*world, end);
                    match &mut draft.kind {
                        ShapeKind::Arrow { end, .. } => {
                            if let Some(value) = endpoint {
                                *end = value;
                            }
                        }
                        ShapeKind::Freehand { points } => {
                            if points
                                .last()
                                .is_none_or(|p| p.distance(at) >= 0.5 / self.editor.camera.zoom)
                            {
                                points.push(at);
                            }
                            draft.rect = point_bounds(points);
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
    pub fn pointer_up(&mut self, screen: Point, now_ms: i64) -> BoardResult<bool> {
        self.pointer_move(screen)?;
        let Some(gesture) = self.gesture.take() else {
            return Ok(false);
        };
        self.editor.selection_box = None;
        match gesture {
            Gesture::Move { .. } => self.editor.commit_move(now_ms),
            Gesture::Draw { .. } => {
                let mut shape = self.editor.draft.take().ok_or(BoardError::Missing)?;
                if !matches!(
                    shape.kind,
                    ShapeKind::Arrow { .. } | ShapeKind::Freehand { .. }
                ) && (shape.rect.w < 1.0 || shape.rect.h < 1.0)
                {
                    shape.rect.w = 180.0;
                    shape.rect.h = if matches!(shape.kind, ShapeKind::Text) {
                        28.0
                    } else {
                        80.0
                    };
                }
                if matches!(shape.kind, ShapeKind::Text) {
                    shape.text = "Text".into();
                }
                let id = shape.id.clone();
                self.editor
                    .edit(vec![BoardOperation::shape(shape)], now_ms)?;
                self.editor.selection = BTreeSet::from([id]);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub fn fit(&mut self, width: f64, height: f64) -> BoardResult<()> {
        if ![width, height]
            .into_iter()
            .all(|v| v.is_finite() && v > 0.0)
        {
            return Err(BoardError::Invalid(
                "The board viewport is unavailable.".into(),
            ));
        }
        let Some(bounds) = self.editor.bounds() else {
            self.editor.camera = Camera::default();
            return Ok(());
        };
        let margin = 40.0;
        let zoom = ((width - margin * 2.0) / bounds.w.max(1.0))
            .min((height - margin * 2.0) / bounds.h.max(1.0))
            .clamp(0.05, 8.0);
        self.editor.camera = Camera {
            x: bounds.center().x - width / zoom / 2.0,
            y: bounds.center().y - height / zoom / 2.0,
            zoom,
        };
        Ok(())
    }
}
fn between(a: Point, b: Point) -> Rect {
    Rect::new(
        a.x.min(b.x),
        a.y.min(b.y),
        (b.x - a.x).abs(),
        (b.y - a.y).abs(),
    )
}
pub fn bind_at(editor: &BoardEditor, point: Point) -> Endpoint {
    if let Some(shape) = editor.ordered().rev().find(|shape| {
        shape.bindable() && hit(editor.document(), shape, point, 8.0 / editor.camera.zoom)
    }) {
        let local = point.rotate(shape.rect.center(), -shape.rotation);
        let anchor = Point::new(
            ((local.x - shape.rect.x) / shape.rect.w.max(1.0)).clamp(0.0, 1.0),
            ((local.y - shape.rect.y) / shape.rect.h.max(1.0)).clamp(0.0, 1.0),
        );
        Endpoint::bound(shape.id.clone(), anchor)
    } else {
        Endpoint::free(point)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dragging_previews_and_commits_once_and_cancellation_leaves_no_shapes() {
        let doc = BoardDocument::empty(
            uuid::Uuid::new_v4().to_string(),
            "/repo".into(),
            "Design".into(),
            0,
        );
        let mut input = BoardInput::new(BoardEditor::new(doc).unwrap());
        input.set_tool(BoardTool::Rectangle);
        input
            .pointer_down(Point::new(10.0, 10.0), false, false)
            .unwrap();
        input.pointer_move(Point::new(200.0, 100.0)).unwrap();
        assert_eq!(input.editor.document().revision, 0);
        assert!(input.editor.draft.is_some());
        input.cancel();
        assert!(input.editor.document().shapes.is_empty());
        input
            .pointer_down(Point::new(10.0, 10.0), false, false)
            .unwrap();
        input.pointer_up(Point::new(200.0, 100.0), 1).unwrap();
        assert_eq!(input.editor.document().revision, 1);
        assert_eq!(input.editor.document().shapes.len(), 1);
        input.editor.undo(2).unwrap();
        assert!(input.editor.document().shapes.is_empty());
    }
}
