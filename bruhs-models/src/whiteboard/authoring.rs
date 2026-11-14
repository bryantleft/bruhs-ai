//! Editing commands used by both surfaces, including keyboard-only diagramming.
use super::*;
use crate::num::ToFloatLossy as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardTool {
    Select,
    Hand,
    Rectangle,
    Ellipse,
    Arrow,
    Text,
    Note,
    Frame,
    Freehand,
    Image,
}
impl BoardTool {
    pub const ALL: [Self; 10] = [
        Self::Select,
        Self::Hand,
        Self::Rectangle,
        Self::Ellipse,
        Self::Arrow,
        Self::Text,
        Self::Note,
        Self::Frame,
        Self::Freehand,
        Self::Image,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select",
            Self::Hand => "Hand",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Arrow => "Arrow",
            Self::Text => "Text",
            Self::Note => "Note",
            Self::Frame => "Frame",
            Self::Freehand => "Draw",
            Self::Image => "Image",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::Select => "v",
            Self::Hand => "h",
            Self::Rectangle => "r",
            Self::Ellipse => "e",
            Self::Arrow => "a",
            Self::Text => "t",
            Self::Note => "n",
            Self::Frame => "f",
            Self::Freehand => "p",
            Self::Image => "i",
        }
    }
    pub fn kind(self) -> Option<ShapeKind> {
        match self {
            Self::Rectangle => Some(ShapeKind::Rectangle),
            Self::Ellipse => Some(ShapeKind::Ellipse),
            Self::Text => Some(ShapeKind::Text),
            Self::Note => Some(ShapeKind::Note),
            Self::Frame => Some(ShapeKind::Frame),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub enum Alignment {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
    DistributeHorizontal,
    DistributeVertical,
}

impl BoardEditor {
    pub fn edit(&mut self, operations: Vec<BoardOperation>, now_ms: i64) -> BoardResult<()> {
        if operations.is_empty() {
            return Ok(());
        }
        self.transaction(
            BoardTransaction {
                base_revision: self.document().revision,
                operations,
            },
            now_ms,
        )
    }
    pub fn next_z(&self) -> BoardResult<i64> {
        self.ordered()
            .map(|shape| shape.z)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| BoardError::Invalid("The shape stacking order is exhausted.".into()))
    }
    pub fn add_node(
        &mut self,
        kind: ShapeKind,
        at: Point,
        text: String,
        now_ms: i64,
    ) -> BoardResult<ShapeId> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut shape = Shape::node(
            id.clone(),
            kind,
            Rect::new(at.x, at.y, 180.0, 80.0),
            self.next_z()?,
            text,
        );
        if matches!(shape.kind, ShapeKind::Text) {
            shape.rect.h = 28.0;
            shape.style.fill = None;
        }
        if matches!(shape.kind, ShapeKind::Note) {
            shape.style.fill = Some(Color::rgb(250, 226, 150));
        }
        if matches!(shape.kind, ShapeKind::Frame) {
            shape.rect.w = 480.0;
            shape.rect.h = 320.0;
            shape.style.fill = None;
        }
        self.edit(vec![BoardOperation::shape(shape)], now_ms)?;
        self.selection = BTreeSet::from([id.clone()]);
        Ok(id)
    }
    pub fn connect(
        &mut self,
        from: &str,
        to: &str,
        text: String,
        now_ms: i64,
    ) -> BoardResult<ShapeId> {
        if ![from, to]
            .iter()
            .all(|id| self.document().shapes.get(*id).is_some_and(Shape::bindable))
        {
            return Err(BoardError::Invalid(
                "Select two diagram nodes to connect.".into(),
            ));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let a = &self.document().shapes[from];
        let b = &self.document().shapes[to];
        let horizontal = (b.rect.center().x - a.rect.center().x).abs()
            >= (b.rect.center().y - a.rect.center().y).abs();
        let (start, end) = if horizontal {
            if b.rect.center().x >= a.rect.center().x {
                (Point::new(1.0, 0.5), Point::new(0.0, 0.5))
            } else {
                (Point::new(0.0, 0.5), Point::new(1.0, 0.5))
            }
        } else if b.rect.center().y >= a.rect.center().y {
            (Point::new(0.5, 1.0), Point::new(0.5, 0.0))
        } else {
            (Point::new(0.5, 0.0), Point::new(0.5, 1.0))
        };
        let mut shape = Shape::node(
            id.clone(),
            ShapeKind::Arrow {
                start: Endpoint::bound(from.into(), start),
                end: Endpoint::bound(to.into(), end),
            },
            Rect::default(),
            self.next_z()?,
            text,
        );
        shape.style.fill = None;
        self.edit(vec![BoardOperation::shape(shape)], now_ms)?;
        Ok(id)
    }
    pub fn connect_next(&mut self, text: String, now_ms: i64) -> BoardResult<ShapeId> {
        let from = self
            .selection
            .iter()
            .find(|id| self.document().shapes.get(*id).is_some_and(Shape::bindable))
            .cloned();
        let at = from.as_ref().map_or(
            Point::new(self.camera.x + 80.0, self.camera.y + 80.0),
            |id| {
                let r = self.document().shapes[id].rect;
                Point::new(r.right() + 80.0, r.y)
            },
        );
        // The new node and edge are one undoable transaction.
        let id = uuid::Uuid::new_v4().to_string();
        let z = self.next_z()?;
        let mut ops = vec![BoardOperation::shape(Shape::node(
            id.clone(),
            ShapeKind::Rectangle,
            Rect::new(at.x, at.y, 180.0, 80.0),
            z,
            text,
        ))];
        if let Some(from) = from {
            let mut arrow = Shape::node(
                uuid::Uuid::new_v4().to_string(),
                ShapeKind::Arrow {
                    start: Endpoint::bound(from, Point::new(1.0, 0.5)),
                    end: Endpoint::bound(id.clone(), Point::new(0.0, 0.5)),
                },
                Rect::default(),
                z.checked_add(1)
                    .ok_or_else(|| BoardError::Invalid("Stacking order exhausted.".into()))?,
                String::new(),
            );
            arrow.style.fill = None;
            ops.push(BoardOperation::shape(arrow));
        }
        self.edit(ops, now_ms)?;
        self.selection = BTreeSet::from([id.clone()]);
        Ok(id)
    }
    pub fn set_text(&mut self, text: String, now_ms: i64) -> BoardResult<()> {
        let Some(id) = self.selection.first() else {
            return Err(BoardError::Invalid(
                "Select a shape to edit its text.".into(),
            ));
        };
        let mut shape = self
            .document()
            .shapes
            .get(id)
            .ok_or(BoardError::Missing)?
            .clone();
        shape.text = text;
        if matches!(shape.kind, ShapeKind::Text) {
            shape.rect.h =
                shape.text.lines().count().max(1).to_f64_lossy() * shape.style.font_size * 1.35;
        }
        self.edit(vec![BoardOperation::shape(shape)], now_ms)
    }
    pub fn link(&mut self, link: BoardLink, now_ms: i64) -> BoardResult<()> {
        link.validate()?;
        let ops = self
            .selection
            .iter()
            .filter_map(|id| self.document().shapes.get(id))
            .map(|shape| {
                let mut shape = shape.clone();
                if !shape.links.contains(&link) {
                    shape.links.push(link.clone());
                }
                BoardOperation::shape(shape)
            })
            .collect();
        self.edit(ops, now_ms)
    }
    pub fn delete_selection(&mut self, now_ms: i64) -> BoardResult<()> {
        let ids = self.expanded_selection();
        if ids.is_empty() {
            return Ok(());
        }
        let mut ops = Vec::new();
        for shape in self.ordered().filter(|s| !ids.contains(&s.id)) {
            let mut next = shape.clone();
            let mut changed = false;
            if let ShapeKind::Arrow { start, end } = &mut next.kind {
                for end in [start, end] {
                    if end
                        .binding
                        .as_ref()
                        .is_some_and(|b| ids.contains(&b.shape_id))
                    {
                        *end = Endpoint::free(endpoint(self.document(), end));
                        changed = true;
                    }
                }
            }
            if changed {
                next.semantic = None;
                ops.push(BoardOperation::shape(next));
            }
        }
        let remaining = self
            .document()
            .shapes
            .values()
            .filter(|shape| !ids.contains(&shape.id))
            .filter_map(|shape| {
                if let ShapeKind::Image { asset_id } = &shape.kind {
                    Some(asset_id)
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        ops.extend(
            self.document()
                .assets
                .keys()
                .filter(|id| !remaining.contains(id))
                .cloned()
                .map(|id| BoardOperation::RemoveAsset { id }),
        );
        ops.extend(ids.into_iter().map(|id| BoardOperation::Delete { id }));
        self.edit(ops, now_ms)
    }
    pub fn nudge(&mut self, delta: Point, now_ms: i64) -> BoardResult<()> {
        self.start_move();
        self.preview_move(delta)?;
        self.commit_move(now_ms)?;
        Ok(())
    }
    pub fn focus_next(&mut self, reverse: bool) -> Option<ShapeId> {
        let ids = self
            .ordered()
            .filter(|s| s.bindable())
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        if ids.is_empty() {
            return None;
        }
        let current = self
            .selection
            .first()
            .and_then(|id| ids.iter().position(|item| item == id));
        let index = if reverse {
            current.map_or(ids.len() - 1, |i| (i + ids.len() - 1) % ids.len())
        } else {
            current.map_or(0, |i| (i + 1) % ids.len())
        };
        let id = ids[index].clone();
        self.selection = BTreeSet::from([id.clone()]);
        Some(id)
    }
    pub fn group(&mut self, now_ms: i64) -> BoardResult<()> {
        let bounds = self
            .selected_bounds()
            .ok_or_else(|| BoardError::Invalid("Select shapes to group.".into()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let group = Shape::node(
            id.clone(),
            ShapeKind::Group,
            bounds,
            self.next_z()?,
            String::new(),
        );
        let mut ops = self
            .selection
            .iter()
            .filter_map(|id| self.document().shapes.get(id))
            .map(|s| {
                let mut s = s.clone();
                s.parent = Some(id.clone());
                BoardOperation::shape(s)
            })
            .collect::<Vec<_>>();
        ops.push(BoardOperation::shape(group));
        self.edit(ops, now_ms)?;
        self.selection = BTreeSet::from([id]);
        Ok(())
    }
    pub fn ungroup(&mut self, now_ms: i64) -> BoardResult<()> {
        let groups = self
            .selection
            .iter()
            .filter(|id| {
                self.document()
                    .shapes
                    .get(*id)
                    .is_some_and(|s| matches!(s.kind, ShapeKind::Group))
            })
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut selected = BTreeSet::new();
        let mut ops = Vec::new();
        for shape in self.ordered() {
            if shape.parent.as_ref().is_some_and(|id| groups.contains(id)) {
                let mut shape = shape.clone();
                shape.parent = shape
                    .parent
                    .as_ref()
                    .and_then(|id| self.document().shapes[id].parent.clone());
                selected.insert(shape.id.clone());
                ops.push(BoardOperation::shape(shape));
            }
        }
        ops.extend(groups.into_iter().map(|id| BoardOperation::Delete { id }));
        self.edit(ops, now_ms)?;
        self.selection = selected;
        Ok(())
    }
    pub fn align(&mut self, alignment: Alignment, now_ms: i64) -> BoardResult<()> {
        let roots = self
            .selection
            .iter()
            .filter(|id| {
                let mut parent = self
                    .document()
                    .shapes
                    .get(*id)
                    .and_then(|s| s.parent.as_ref());
                while let Some(id) = parent {
                    if self.selection.contains(id) {
                        return false;
                    }
                    parent = self
                        .document()
                        .shapes
                        .get(id)
                        .and_then(|s| s.parent.as_ref());
                }
                true
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut shapes = roots
            .iter()
            .filter_map(|id| self.document().shapes.get(id))
            .map(|shape| {
                let area = if matches!(shape.kind, ShapeKind::Group) {
                    self.expand_ids([shape.id.clone()])
                        .iter()
                        .filter_map(|id| self.document().shapes.get(id))
                        .filter(|s| !matches!(s.kind, ShapeKind::Group))
                        .map(|s| bounds(self.document(), s))
                        .reduce(Rect::union)
                        .unwrap_or(shape.rect)
                } else {
                    bounds(self.document(), shape)
                };
                (shape.clone(), area)
            })
            .collect::<Vec<_>>();
        if shapes.len() < 2 {
            return Err(BoardError::Invalid(
                "Select at least two separate shapes or containers to align.".into(),
            ));
        }
        if shapes
            .iter()
            .any(|(s, _)| matches!(s.kind, ShapeKind::Arrow { .. }))
        {
            return Err(BoardError::Invalid(
                "Align nodes, paths or containers. Bound arrows follow their targets.".into(),
            ));
        }
        let all = shapes
            .iter()
            .map(|(_, area)| *area)
            .reduce(Rect::union)
            .ok_or(BoardError::Missing)?;
        let horizontal = matches!(alignment, Alignment::DistributeHorizontal);
        if matches!(
            alignment,
            Alignment::DistributeHorizontal | Alignment::DistributeVertical
        ) {
            shapes.sort_by(|(_, a), (_, b)| {
                if horizontal {
                    a.x.total_cmp(&b.x)
                } else {
                    a.y.total_cmp(&b.y)
                }
            });
        }
        let occupied = shapes
            .iter()
            .map(|(_, r)| if horizontal { r.w } else { r.h })
            .sum::<f64>();
        let gaps = crate::num::ToFloatLossy::to_f64_lossy(shapes.len() - 1);
        let gap = if horizontal {
            (all.w - occupied) / gaps
        } else {
            (all.h - occupied) / gaps
        };
        let mut cursor = if horizontal { all.x } else { all.y };
        let selected = self.expanded_selection();
        let mut ops = Vec::new();
        for (shape, r) in shapes {
            let delta = match alignment {
                Alignment::Left => Point::new(all.x - r.x, 0.0),
                Alignment::Center => Point::new(all.center().x - r.center().x, 0.0),
                Alignment::Right => Point::new(all.right() - r.right(), 0.0),
                Alignment::Top => Point::new(0.0, all.y - r.y),
                Alignment::Middle => Point::new(0.0, all.center().y - r.center().y),
                Alignment::Bottom => Point::new(0.0, all.bottom() - r.bottom()),
                Alignment::DistributeHorizontal => {
                    let d = Point::new(cursor - r.x, 0.0);
                    cursor += r.w + gap;
                    d
                }
                Alignment::DistributeVertical => {
                    let d = Point::new(0.0, cursor - r.y);
                    cursor += r.h + gap;
                    d
                }
            };
            for id in self.expand_ids([shape.id]) {
                if let Some(shape) = self.document().shapes.get(&id) {
                    ops.push(BoardOperation::shape(translated_shape(
                        self.document(),
                        shape,
                        delta,
                        &selected,
                    )));
                }
            }
        }
        self.edit(ops, now_ms)
    }
    pub fn copy_selection(&self) -> BoardResult<BoardDocument> {
        let ids = self.expanded_selection();
        let mut copy = self.document().clone();
        copy.id = uuid::Uuid::new_v4().to_string();
        copy.agent_read = false;
        copy.revision = 0;
        copy.shapes.retain(|id, _| ids.contains(id));
        let assets = copy
            .shapes
            .values()
            .filter_map(|shape| {
                if let ShapeKind::Image { asset_id } = &shape.kind {
                    Some(asset_id.clone())
                } else {
                    None
                }
            })
            .collect::<BTreeSet<_>>();
        let sources = copy
            .shapes
            .values()
            .filter_map(|shape| shape.source_id.clone())
            .collect::<BTreeSet<_>>();
        copy.assets.retain(|id, _| assets.contains(id));
        copy.sources.retain(|id, _| sources.contains(id));
        for shape in copy.shapes.values_mut() {
            if matches!(&shape.semantic,Some(ShapeSemantic::SequenceMessage{from,to,..}) if !ids.contains(from)||!ids.contains(to))
            {
                shape.semantic = None;
            }
            if shape.parent.as_ref().is_some_and(|id| !ids.contains(id)) {
                shape.parent = None;
            }
            if let ShapeKind::Arrow { start, end } = &mut shape.kind {
                for end in [start, end] {
                    if end
                        .binding
                        .as_ref()
                        .is_some_and(|b| !ids.contains(&b.shape_id))
                    {
                        *end = Endpoint::free(endpoint(self.document(), end));
                    }
                }
            }
        }
        copy.validate()?;
        Ok(copy)
    }
    pub fn paste(&mut self, copy: &BoardDocument, delta: Point, now_ms: i64) -> BoardResult<()> {
        copy.validate()?;
        let ids = copy
            .shapes
            .keys()
            .map(|id| (id.clone(), uuid::Uuid::new_v4().to_string()))
            .collect::<BTreeMap<_, _>>();
        let all = copy.shapes.keys().cloned().collect::<BTreeSet<_>>();
        let mut ops = Vec::new();
        let z = self.next_z()?;
        for source in copy.sources.values() {
            if self
                .document()
                .sources
                .get(&source.id)
                .is_some_and(|s| s != source)
            {
                return Err(BoardError::Invalid(
                    "Clipboard source identity conflicts with this board.".into(),
                ));
            }
            ops.push(BoardOperation::Source {
                source: source.clone(),
            });
        }
        for asset in copy.assets.values() {
            if self
                .document()
                .assets
                .get(&asset.id)
                .is_some_and(|a| a != asset)
            {
                return Err(BoardError::Invalid(
                    "Clipboard asset identity conflicts with this board.".into(),
                ));
            }
            ops.push(BoardOperation::Asset {
                asset: asset.clone(),
            });
        }
        let mut ordered = copy.shapes.values().collect::<Vec<_>>();
        ordered.sort_by(|a, b| a.z.cmp(&b.z).then(a.id.cmp(&b.id)));
        let sequence_base = self
            .document()
            .shapes
            .values()
            .filter_map(|shape| {
                if let Some(ShapeSemantic::SequenceMessage { index, .. }) = &shape.semantic {
                    Some(*index)
                } else {
                    None
                }
            })
            .max()
            .map_or(Ok(0), |index| {
                index
                    .checked_add(1)
                    .ok_or_else(|| BoardError::Invalid("Sequence ordering is exhausted.".into()))
            })?;
        for (index, shape) in ordered.into_iter().enumerate() {
            let mut shape = translated_shape(copy, shape, delta, &all);
            shape.id = ids[&shape.id].clone();
            shape.parent = shape.parent.as_ref().and_then(|id| ids.get(id).cloned());
            shape.z = z
                .checked_add(
                    i64::try_from(index)
                        .map_err(|_| BoardError::Invalid("Too many pasted shapes.".into()))?,
                )
                .ok_or_else(|| BoardError::Invalid("Stacking order exhausted.".into()))?;
            if let ShapeKind::Arrow { start, end } = &mut shape.kind {
                for end in [start, end] {
                    if let Some(binding) = &mut end.binding {
                        binding.shape_id = ids[&binding.shape_id].clone();
                    }
                }
            }
            match &mut shape.semantic {
                Some(ShapeSemantic::SequenceParticipant { name, .. }) => {
                    *name = format!("p_{}", shape.id.replace('-', "_"))
                }
                Some(ShapeSemantic::SequenceMessage {
                    from, to, index, ..
                }) => {
                    *from = ids.get(from).cloned().ok_or(BoardError::Missing)?;
                    *to = ids.get(to).cloned().ok_or(BoardError::Missing)?;
                    *index = sequence_base.checked_add(*index).ok_or_else(|| {
                        BoardError::Invalid("Sequence ordering is exhausted.".into())
                    })?;
                }
                None => {}
            }
            ops.push(BoardOperation::shape(shape));
        }
        self.edit(ops, now_ms)?;
        self.selection = ids.into_values().collect();
        Ok(())
    }
}

pub fn snap(point: Point, grid: f64) -> BoardResult<Point> {
    if !point.finite() || !grid.is_finite() || grid <= 0.0 {
        return Err(BoardError::Invalid(
            "Snapping needs finite coordinates and a positive grid size.".into(),
        ));
    }
    Ok(Point::new(
        (point.x / grid).round() * grid,
        (point.y / grid).round() * grid,
    ))
}

pub fn selection_caption(editor: &BoardEditor) -> String {
    let Some(shape) = editor
        .selection
        .first()
        .and_then(|id| editor.document().shapes.get(id))
    else {
        return "No shape selected".into();
    };
    let kind = match shape.kind {
        ShapeKind::Rectangle => "Rectangle",
        ShapeKind::Ellipse => "Ellipse",
        ShapeKind::Arrow { .. } => "Arrow",
        ShapeKind::Text => "Text",
        ShapeKind::Note => "Note",
        ShapeKind::Frame => "Frame",
        ShapeKind::Group => "Group",
        ShapeKind::Freehand { .. } => "Drawing",
        ShapeKind::Image { .. } => "Image",
    };
    let label = if shape.text.trim().is_empty() {
        kind.into()
    } else {
        shape.text.chars().take(80).collect::<String>()
    };
    format!("{} selected · {label}", editor.selection.len())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_connect_delete_and_undo_keep_bindings_valid() {
        let mut editor = BoardEditor::new(BoardDocument::empty(
            uuid::Uuid::new_v4().to_string(),
            "/repo".into(),
            "Design".into(),
            0,
        ))
        .unwrap();
        let a = editor.connect_next("A".into(), 1).unwrap();
        let b = editor.connect_next("B".into(), 2).unwrap();
        assert_eq!(editor.document().shapes.len(), 3);
        editor.selection = BTreeSet::from([a.clone()]);
        editor.delete_selection(3).unwrap();
        editor.document().validate().unwrap();
        assert!(editor
            .document()
            .shapes
            .values()
            .any(|s| matches!(&s.kind,ShapeKind::Arrow{start,..}if start.binding.is_none())));
        editor.undo(4).unwrap();
        assert!(editor.document().shapes.contains_key(&a));
        assert!(editor.document().shapes.contains_key(&b));
        editor.document().validate().unwrap();
    }
    #[test]
    fn paste_remaps_internal_bindings_and_detaches_external_ones() {
        let mut editor = BoardEditor::new(benchmark_scene(1000).unwrap()).unwrap();
        editor.selection = BTreeSet::from(["node-00000".into(), "arrow-00000".into()]);
        let copy = editor.copy_selection().unwrap();
        editor.paste(&copy, Point::new(24.0, 24.0), 1).unwrap();
        assert_eq!(editor.document().shapes.len(), 1002);
        editor.document().validate().unwrap();
        assert_eq!(editor.selection.len(), 2);
    }
}
