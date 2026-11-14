//! Durable edits are atomic transactions; pointer previews never clone the document.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardTransaction {
    pub base_revision: u64,
    pub operations: Vec<BoardOperation>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum BoardOperation {
    Upsert { shape: Box<Shape> },
    Delete { id: ShapeId },
    Title { title: String },
    AgentRead { allowed: bool },
    Source { source: Source },
    RemoveSource { id: String },
    Asset { asset: Asset },
    RemoveAsset { id: AssetId },
    Background { color: Color },
}
impl BoardOperation {
    pub fn shape(shape: Shape) -> Self {
        Self::Upsert {
            shape: Box::new(shape),
        }
    }
}
#[derive(Clone)]
struct History {
    undo: Vec<BoardOperation>,
    redo: Vec<BoardOperation>,
}
#[derive(Debug, Clone)]
pub struct MovePreview {
    pub ids: BTreeSet<ShapeId>,
    pub delta: Point,
}
pub struct BoardEditor {
    document: BoardDocument,
    order: Vec<ShapeId>,
    children: std::collections::BTreeMap<ShapeId, Vec<ShapeId>>,
    pub camera: Camera,
    pub selection: BTreeSet<ShapeId>,
    pub movement: Option<MovePreview>,
    pub draft: Option<Shape>,
    pub selection_box: Option<Rect>,
    history: Vec<History>,
    redo: Vec<History>,
}
impl BoardEditor {
    pub fn new(document: BoardDocument) -> BoardResult<Self> {
        document.validate()?;
        let mut this = Self {
            document,
            order: Vec::new(),
            children: std::collections::BTreeMap::new(),
            camera: Camera::default(),
            selection: BTreeSet::new(),
            movement: None,
            draft: None,
            selection_box: None,
            history: Vec::new(),
            redo: Vec::new(),
        };
        this.reindex();
        Ok(this)
    }
    pub fn document(&self) -> &BoardDocument {
        &self.document
    }
    pub fn ordered(&self) -> impl DoubleEndedIterator<Item = &Shape> {
        self.order
            .iter()
            .filter_map(|id| self.document.shapes.get(id))
            .chain(self.draft.iter())
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn transaction(&mut self, transaction: BoardTransaction, now_ms: i64) -> BoardResult<()> {
        let inverse = self.apply(transaction.base_revision, &transaction.operations, now_ms)?;
        self.history.push(History {
            undo: inverse,
            redo: transaction.operations,
        });
        self.redo.clear();
        Ok(())
    }
    fn apply(
        &mut self,
        revision: u64,
        operations: &[BoardOperation],
        now_ms: i64,
    ) -> BoardResult<Vec<BoardOperation>> {
        if revision != self.document.revision {
            return Err(BoardError::Conflict);
        }
        // One validation copy per committed edit, never per pointer move. Inverses retain
        // only edited records; undo does not store whole-board snapshots.
        let mut candidate = self.document.clone();
        let mut inverse = Vec::new();
        for operation in operations {
            match operation {
                BoardOperation::Upsert { shape } => {
                    let old = candidate
                        .shapes
                        .insert(shape.id.clone(), shape.as_ref().clone());
                    inverse.push(old.map_or_else(
                        || BoardOperation::Delete {
                            id: shape.id.clone(),
                        },
                        BoardOperation::shape,
                    ));
                }
                BoardOperation::Delete { id } => {
                    if let Some(old) = candidate.shapes.remove(id) {
                        inverse.push(BoardOperation::shape(old));
                    }
                }
                BoardOperation::Title { title } => inverse.push(BoardOperation::Title {
                    title: std::mem::replace(&mut candidate.title, title.clone()),
                }),
                BoardOperation::AgentRead { allowed } => inverse.push(BoardOperation::AgentRead {
                    allowed: std::mem::replace(&mut candidate.agent_read, *allowed),
                }),
                BoardOperation::Source { source } => {
                    let old = candidate.sources.insert(source.id.clone(), source.clone());
                    inverse.push(old.map_or_else(
                        || BoardOperation::RemoveSource {
                            id: source.id.clone(),
                        },
                        |source| BoardOperation::Source { source },
                    ));
                }
                BoardOperation::RemoveSource { id } => {
                    if let Some(source) = candidate.sources.remove(id) {
                        inverse.push(BoardOperation::Source { source });
                    }
                }
                BoardOperation::Asset { asset } => {
                    let old = candidate.assets.insert(asset.id.clone(), asset.clone());
                    inverse.push(old.map_or_else(
                        || BoardOperation::RemoveAsset {
                            id: asset.id.clone(),
                        },
                        |asset| BoardOperation::Asset { asset },
                    ));
                }
                BoardOperation::RemoveAsset { id } => {
                    if let Some(asset) = candidate.assets.remove(id) {
                        inverse.push(BoardOperation::Asset { asset });
                    }
                }
                BoardOperation::Background { color } => inverse.push(BoardOperation::Background {
                    color: std::mem::replace(&mut candidate.background, *color),
                }),
            }
        }
        candidate.validate()?;
        candidate.revision = candidate.revision.checked_add(1).ok_or_else(|| {
            BoardError::Invalid("The board revision is exhausted. Save a copy.".into())
        })?;
        candidate.updated_at_ms = now_ms;
        inverse.reverse();
        self.document = candidate;
        self.selection
            .retain(|id| self.document.shapes.contains_key(id));
        self.movement = None;
        self.reindex();
        Ok(inverse)
    }
    pub fn undo(&mut self, now_ms: i64) -> BoardResult<bool> {
        let Some(entry) = self.history.pop() else {
            return Ok(false);
        };
        match self.apply(self.document.revision, &entry.undo, now_ms) {
            Ok(_) => {
                self.redo.push(entry);
                Ok(true)
            }
            Err(error) => {
                self.history.push(entry);
                Err(error)
            }
        }
    }
    pub fn redo(&mut self, now_ms: i64) -> BoardResult<bool> {
        let Some(entry) = self.redo.pop() else {
            return Ok(false);
        };
        match self.apply(self.document.revision, &entry.redo, now_ms) {
            Ok(_) => {
                self.history.push(entry);
                Ok(true)
            }
            Err(error) => {
                self.redo.push(entry);
                Err(error)
            }
        }
    }
    fn reindex(&mut self) {
        let mut order = self
            .document
            .shapes
            .values()
            .map(|shape| (shape.z, shape.id.clone()))
            .collect::<Vec<_>>();
        order.sort_unstable();
        self.order = order.into_iter().map(|(_, id)| id).collect();
        self.children.clear();
        for shape in self.document.shapes.values() {
            if let Some(parent) = &shape.parent {
                self.children
                    .entry(parent.clone())
                    .or_default()
                    .push(shape.id.clone());
            }
        }
    }
    pub fn select_all(&mut self) {
        self.selection = self.document.shapes.keys().cloned().collect();
    }
    pub fn selected_bounds(&self) -> Option<Rect> {
        let expanded;
        let ids = if let Some(movement) = &self.movement {
            &movement.ids
        } else if self.selection.iter().any(|id| {
            self.document
                .shapes
                .get(id)
                .is_some_and(|s| matches!(s.kind, ShapeKind::Group))
        }) {
            expanded = self.expanded_selection();
            &expanded
        } else {
            &self.selection
        };
        ids.iter()
            .filter_map(|id| self.document.shapes.get(id))
            .filter(|shape| !matches!(shape.kind, ShapeKind::Group))
            .map(|shape| bounds(&self.document, shape))
            .reduce(Rect::union)
    }
    pub fn hit(&self, point: Point, tolerance: f64) -> Option<ShapeId> {
        let shape = self
            .ordered()
            .rev()
            .find(|shape| hit(&self.document, shape, point, tolerance))?;
        let mut id = shape.id.clone();
        let mut parent = shape.parent.as_ref();
        while let Some(next) = parent.and_then(|id| self.document.shapes.get(id)) {
            if matches!(next.kind, ShapeKind::Group) {
                id = next.id.clone();
            }
            parent = next.parent.as_ref();
        }
        Some(id)
    }
    pub fn start_move(&mut self) {
        self.movement = Some(MovePreview {
            ids: self.expanded_selection(),
            delta: Point::default(),
        });
    }
    pub fn preview_move(&mut self, delta: Point) -> BoardResult<()> {
        if !delta.finite() {
            return Err(BoardError::Invalid("The move must be finite.".into()));
        }
        let movement = self.movement.as_mut().ok_or(BoardError::Missing)?;
        movement.delta = delta;
        Ok(())
    }
    pub fn cancel_preview(&mut self) {
        self.movement = None;
        self.draft = None;
        self.selection_box = None;
    }
    pub fn commit_move(&mut self, now_ms: i64) -> BoardResult<bool> {
        let Some(preview) = self.movement.take() else {
            return Ok(false);
        };
        if preview.delta == Point::default() {
            return Ok(false);
        }
        let operations = preview
            .ids
            .iter()
            .filter_map(|id| self.document.shapes.get(id))
            .map(|shape| {
                BoardOperation::shape(translated_shape(
                    &self.document,
                    shape,
                    preview.delta,
                    &preview.ids,
                ))
            })
            .collect();
        self.transaction(
            BoardTransaction {
                base_revision: self.document.revision,
                operations,
            },
            now_ms,
        )?;
        Ok(true)
    }
    pub fn expanded_selection(&self) -> BTreeSet<ShapeId> {
        self.expand_ids(self.selection.iter().cloned())
    }
    pub fn expand_ids(&self, ids: impl IntoIterator<Item = ShapeId>) -> BTreeSet<ShapeId> {
        let mut ids = ids.into_iter().collect::<BTreeSet<_>>();
        let mut pending = ids.iter().cloned().collect::<Vec<_>>();
        while let Some(parent) = pending.pop() {
            if let Some(children) = self.children.get(&parent) {
                for id in children {
                    if ids.insert(id.clone()) {
                        pending.push(id.clone());
                    }
                }
            }
        }
        ids
    }
    pub fn bounds(&self) -> Option<Rect> {
        self.ordered()
            .filter(|shape| !matches!(shape.kind, ShapeKind::Group))
            .map(|shape| bounds(&self.document, shape))
            .reduce(Rect::union)
    }
}
pub fn translated_shape(
    document: &BoardDocument,
    shape: &Shape,
    delta: Point,
    selected: &BTreeSet<ShapeId>,
) -> Shape {
    let mut next = shape.clone();
    next.rect = next.rect.translated(delta);
    match &mut next.kind {
        ShapeKind::Arrow { start, end } => {
            for endpoint in [start, end] {
                if endpoint
                    .binding
                    .as_ref()
                    .is_none_or(|b| !selected.contains(&b.shape_id))
                {
                    if matches!(shape.semantic, Some(ShapeSemantic::SequenceMessage { .. })) {
                        if let Some(binding) = &mut endpoint.binding {
                            if let Some(target) = document.shapes.get(&binding.shape_id) {
                                binding.offset = binding
                                    .offset
                                    .offset(delta.rotate(Point::default(), -target.rotation));
                            }
                        }
                    } else {
                        *endpoint =
                            Endpoint::free(super::endpoint(document, endpoint).offset(delta));
                    }
                }
            }
        }
        ShapeKind::Freehand { points } => {
            for point in points {
                *point = point.offset(delta);
            }
        }
        _ => {}
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pointer_preview_does_not_mutate_document_and_one_commit_undoes_the_whole_selection() {
        let document = benchmark_scene(1000).unwrap();
        let before = document.clone();
        let mut editor = BoardEditor::new(document).unwrap();
        editor.select_all();
        editor.start_move();
        for step in 0..100 {
            editor
                .preview_move(Point::new(f64::from(step), 20.0))
                .unwrap();
            assert_eq!(editor.document().revision, 0);
        }
        assert_eq!(editor.document(), &before);
        editor.commit_move(1).unwrap();
        assert_eq!(editor.document().revision, 1);
        assert_ne!(&editor.document().shapes, &before.shapes);
        editor.undo(2).unwrap();
        assert_eq!(editor.document().shapes, before.shapes);
        editor.redo(3).unwrap();
        assert_eq!(editor.document().revision, 3);
    }
    #[test]
    fn invalid_or_stale_edit_is_atomic_and_undo_keeps_only_changed_records() {
        let document = benchmark_scene(1000).unwrap();
        let mut editor = BoardEditor::new(document).unwrap();
        let before = editor.document().clone();
        let mut shape = before.shapes["node-00000"].clone();
        shape.rect.x = f64::NAN;
        assert!(editor
            .transaction(
                BoardTransaction {
                    base_revision: 0,
                    operations: vec![BoardOperation::shape(shape)]
                },
                1
            )
            .is_err());
        assert_eq!(editor.document(), &before);
        editor
            .transaction(
                BoardTransaction {
                    base_revision: 0,
                    operations: vec![BoardOperation::Title {
                        title: "New title".into(),
                    }],
                },
                1,
            )
            .unwrap();
        assert_eq!(editor.history[0].undo.len(), 1);
        assert!(matches!(
            editor.transaction(
                BoardTransaction {
                    base_revision: 0,
                    operations: vec![]
                },
                2
            ),
            Err(BoardError::Conflict)
        ));
    }
}
