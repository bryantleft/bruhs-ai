use super::*;
use std::collections::{BTreeMap, BTreeSet};
fn empty() -> BoardDocument {
    BoardDocument::empty(
        uuid::Uuid::new_v4().to_string(),
        "/repo".into(),
        "Review regression".into(),
        0,
    )
}
fn node(id: &str, x: f64, y: f64, z: i64, text: &str) -> Shape {
    Shape::node(
        id.into(),
        ShapeKind::Rectangle,
        Rect::new(x, y, 40.0, 40.0),
        z,
        text.into(),
    )
}
#[test]
fn copying_a_plain_node_does_not_include_unselected_sources_or_image_payloads() {
    let mut board = empty();
    board
        .shapes
        .insert("safe".into(), node("safe", 0.0, 0.0, 0, "Safe"));
    let source = Source {
        id: "sensitive-source".into(),
        path: "private/config.rs".into(),
        sha256: "a".repeat(64),
        mermaid: Some("A[Private system] --> B".into()),
        stale: false,
    };
    board.sources.insert(source.id.clone(), source);
    let mut secret = node("secret", 100.0, 0.0, 1, "Private");
    secret.source_id = Some("sensitive-source".into());
    board.shapes.insert(secret.id.clone(), secret);
    let asset = Asset {
        id: "secret-image".into(),
        mime: "image/png".into(),
        bytes: 24,
        width: 1,
        height: 1,
        sha256: "b".repeat(64),
    };
    board.assets.insert(asset.id.clone(), asset);
    let mut image = node("picture", 200.0, 0.0, 2, "");
    image.kind = ShapeKind::Image {
        asset_id: "secret-image".into(),
    };
    board.shapes.insert(image.id.clone(), image);
    let mut editor = BoardEditor::new(board).unwrap();
    editor.selection = BTreeSet::from(["safe".into()]);
    let copy = editor.copy_selection().unwrap();
    assert!(copy.sources.is_empty());
    assert!(copy.assets.is_empty());
    PortableBoard {
        board: copy,
        payloads: BTreeMap::new(),
    }
    .validate()
    .unwrap();
}
#[test]
fn paste_preserves_stacking_when_ids_sort_in_the_opposite_order() {
    let mut board = empty();
    board
        .shapes
        .insert("z-back".into(), node("z-back", 0.0, 0.0, 0, "Back"));
    board
        .shapes
        .insert("a-front".into(), node("a-front", 0.0, 0.0, 1, "Front"));
    let mut source = BoardEditor::new(board).unwrap();
    source.select_all();
    let copy = source.copy_selection().unwrap();
    let mut destination = BoardEditor::new(empty()).unwrap();
    destination.paste(&copy, Point::default(), 1).unwrap();
    let top = destination.hit(Point::new(20.0, 20.0), 0.0).unwrap();
    assert_eq!(destination.document().shapes[&top].text, "Front");
}
#[test]
fn pasted_sequences_remap_participants_and_order_in_empty_and_existing_boards() {
    let board=import_mermaid("sequenceDiagram\nparticipant A as App\nparticipant B as Broker\nA->>B: Ask\nB-->>A: Answer","/repo".into(),"Sequence".into(),0).unwrap();
    let mut source = BoardEditor::new(board.clone()).unwrap();
    source.select_all();
    let copy = source.copy_selection().unwrap();
    for destination in [empty(), board] {
        let before = destination.shapes.len();
        let mut editor = BoardEditor::new(destination).unwrap();
        editor.paste(&copy, Point::new(24.0, 24.0), 1).unwrap();
        editor.document().validate().unwrap();
        let exported = export_mermaid(editor.document()).unwrap();
        let restored = import_mermaid(&exported, "/repo".into(), "Restored".into(), 0).unwrap();
        assert_eq!(restored.shapes.len(), before + 4);
    }
}
#[test]
fn alignment_moves_frame_descendants_and_their_bindings() {
    let mut board = empty();
    let mut frame = node("frame", 0.0, 0.0, 0, "");
    frame.kind = ShapeKind::Frame;
    frame.rect = Rect::new(0.0, 0.0, 100.0, 100.0);
    board.shapes.insert(frame.id.clone(), frame);
    let mut child = node("child", 10.0, 10.0, 1, "Child");
    child.parent = Some("frame".into());
    board.shapes.insert(child.id.clone(), child);
    board
        .shapes
        .insert("other".into(), node("other", 200.0, 200.0, 2, "Other"));
    let mut editor = BoardEditor::new(board).unwrap();
    let arrow = editor.connect("child", "other", String::new(), 0).unwrap();
    editor.selection = BTreeSet::from(["frame".into(), "other".into()]);
    editor.align(Alignment::Right, 1).unwrap();
    assert_eq!(editor.document().shapes["frame"].rect.x, 140.0);
    assert_eq!(editor.document().shapes["child"].rect.x, 150.0);
    if let ShapeKind::Arrow { start, .. } = &editor.document().shapes[&arrow].kind {
        assert_eq!(endpoint(editor.document(), start).x, 190.0);
    }
    editor.undo(2).unwrap();
    assert_eq!(editor.document().shapes["child"].rect.x, 10.0);
}
#[test]
fn dragging_a_grouped_child_moves_the_whole_group() {
    let mut board = empty();
    board.shapes.insert("a".into(), node("a", 0.0, 0.0, 0, "A"));
    board
        .shapes
        .insert("b".into(), node("b", 100.0, 0.0, 1, "B"));
    let mut editor = BoardEditor::new(board).unwrap();
    editor.select_all();
    editor.group(1).unwrap();
    let mut input = BoardInput::new(editor);
    input.snapping = false;
    input
        .pointer_down(Point::new(20.0, 20.0), false, false)
        .unwrap();
    input.pointer_up(Point::new(40.0, 30.0), 2).unwrap();
    assert_eq!(input.editor.document().shapes["a"].rect.x, 20.0);
    assert_eq!(input.editor.document().shapes["b"].rect.x, 120.0);
}
#[test]
fn quoted_brackets_and_dashed_sequence_messages_round_trip() {
    let board = import_mermaid(
        "flowchart LR\nA[\"array[0]\"] --> B[\"next\"]",
        "/repo".into(),
        "Brackets".into(),
        0,
    )
    .unwrap();
    let text = export_mermaid(&board).unwrap();
    let restored = import_mermaid(&text, "/repo".into(), "Restored".into(), 0).unwrap();
    assert!(restored.shapes.values().any(|s| s.text == "array[0]"));
    let board = import_mermaid(
        "sequenceDiagram\nA-->>B: Answer",
        "/repo".into(),
        "Dash".into(),
        0,
    )
    .unwrap();
    assert!(export_svg(&board, &BTreeMap::new(), None)
        .unwrap()
        .contains("stroke-dasharray=\"6 4\""));
}
#[test]
fn sequence_semantics_cannot_disagree_with_actual_bindings() {
    let mut board = import_mermaid(
        "sequenceDiagram\nA->>B: Ask",
        "/repo".into(),
        "Sequence".into(),
        0,
    )
    .unwrap();
    let message = board
        .shapes
        .values_mut()
        .find(|shape| matches!(shape.kind, ShapeKind::Arrow { .. }))
        .unwrap();
    if let ShapeKind::Arrow { end, .. } = &mut message.kind {
        end.binding.as_mut().unwrap().shape_id = "A".into();
    }
    assert!(board.validate().is_err());
}
