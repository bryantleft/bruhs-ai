use super::*;
use serde_json::{json, Value};
#[test]
fn proposals_seal_the_exact_base_and_every_operation_is_reviewable() {
    let mut board = benchmark_scene(4).unwrap();
    board.agent_read = true;
    let proposal = BoardProposal::new(
        &board,
        BoardRequester {
            thread_id: "thread".into(),
            turn_id: "turn".into(),
        },
        "Rename".into(),
        vec![BoardOperation::Title {
            title: "Changed".into(),
        }],
    )
    .unwrap();
    assert_eq!(proposal.operations.len(), 1);
    let mut same_revision = board.clone();
    same_revision.title = "Externally changed without revision".into();
    assert_eq!(
        proposal.preview(&same_revision, 2),
        Err(BoardError::Conflict)
    );
    assert_eq!(proposal.preview(&board, 2).unwrap().title, "Changed");
}
#[test]
fn tool_replies_budget_the_outer_frame_and_color_input_cannot_panic() {
    let mut board = benchmark_scene(4).unwrap();
    board.title = "\"".repeat(80_000);
    let page = board_page(&board, None, Some(1)).unwrap();
    let reply = board_tool_reply(page);
    assert!(!reply.ok && reply.text.contains("frame budget"));
    assert!(serde_json::to_vec(&reply).unwrap().len() < crate::thread_link::BROKER_FRAME_BYTES);
    assert!(import_board_file(r##"{"type":"excalidraw","version":2,"elements":[],"appState":{"viewBackgroundColor":"#☃aaa"}}"##,"bad.excalidraw","/project".into(),1).is_err());
}
#[test]
fn tldraw_nonexact_bindings_use_outlines_and_all_consumed_schema_versions_are_checked() {
    let original: Value = serde_json::from_str(include_str!(
        "../../../bench/whiteboard/fixtures/tldraw-5.6.0.tldr"
    ))
    .unwrap();
    let mut value = original.clone();
    for binding in value["records"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .filter(|r| r["typeName"] == "binding")
    {
        binding["props"]["normalizedAnchor"] = json!({"x":0.5,"y":0.5});
        binding["props"]["isExact"] = Value::Bool(false);
    }
    let imported =
        import_board_file(&value.to_string(), "normal.tldr", "/project".into(), 1).unwrap();
    let board = &imported.portable.board;
    for shape in board.shapes.values() {
        if let ShapeKind::Arrow { start, end } = &shape.kind {
            for endpoint in [start, end] {
                let binding = endpoint.binding.as_ref().unwrap();
                assert!(
                    binding.anchor.x == 0.0
                        || binding.anchor.x == 1.0
                        || binding.anchor.y == 0.0
                        || binding.anchor.y == 1.0
                );
            }
        }
    }
    for key in [
        "com.tldraw.binding.arrow",
        "com.tldraw.shape.image",
        "com.tldraw.shape.line",
        "com.tldraw.shape.highlight",
        "com.tldraw.asset.image",
    ] {
        for version in [json!(999), json!("invalid")] {
            let mut value = original.clone();
            value["schema"]["sequences"][key] = version;
            assert!(
                import_board_file(&value.to_string(), "future.tldr", "/project".into(), 1).is_err(),
                "{key}"
            );
        }
    }
}
#[test]
fn nested_tldraw_stacking_and_note_scale_survive_conversion() {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../bench/whiteboard/fixtures/tldraw-5.6.0.tldr"
    ))
    .unwrap();
    let page = value["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["typeName"] == "page")
        .unwrap()["id"]
        .clone();
    value["records"] = json!([
        {"typeName":"shape","id":"shape:root","type":"geo","parentId":page,"index":"a1V","x":0,"y":0,"props":{"geo":"rectangle","w":100,"h":100}},
        {"typeName":"shape","id":"shape:group","type":"group","parentId":page,"index":"a2","x":10,"y":20,"props":{}},
        {"typeName":"shape","id":"shape:child","type":"note","parentId":"shape:group","index":"a1","x":0,"y":0,"props":{"scale":2,"growY":0}}
    ]);
    let imported =
        import_board_file(&value.to_string(), "groups.tldr", "/project".into(), 1).unwrap();
    let board = &imported.portable.board;
    assert!(board.shapes["shape:child"].z > board.shapes["shape:root"].z);
    assert_eq!(
        board.shapes["shape:child"].rect,
        Rect::new(10.0, 20.0, 400.0, 400.0)
    );
}
