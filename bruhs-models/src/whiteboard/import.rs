//! Legacy files are converted locally to editable records, with explicit loss notes.
mod excalidraw;
mod tldraw;
use super::*;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Debug, Clone)]
pub struct BoardImport {
    pub portable: PortableBoard,
    pub warnings: Vec<String>,
}
pub fn import_board_file(
    text: &str,
    name: &str,
    project: String,
    now: i64,
) -> BoardResult<BoardImport> {
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_PORTABLE_BYTES {
        return Err(invalid("The file exceeds the portable board budget."));
    }
    if name.ends_with(".mmd") || name.ends_with(".mermaid") {
        return Ok(BoardImport {
            portable: PortableBoard {
                board: import_mermaid(text, project, name.into(), now)?,
                payloads: BTreeMap::new(),
            },
            warnings: Vec::new(),
        });
    }
    let value: Value =
        serde_json::from_str(text).map_err(|e| invalid(&format!("Invalid board JSON: {e}")))?;
    let mut result = if value.get("board").is_some() {
        BoardImport {
            portable: serde_json::from_value::<PortableBoard>(value)
                .map_err(|e| invalid(&e.to_string()))?
                .import_copy(project, now)?,
            warnings: Vec::new(),
        }
    } else if value["type"] == "excalidraw" {
        excalidraw::convert(value, project, name, now)?
    } else if value.get("tldrawFileFormatVersion").is_some() {
        tldraw::convert(value, project, name, now)?
    } else {
        return Err(invalid(
            "Choose a Bruhs portable board, Excalidraw JSON v2, tldraw .tldr v1 or supported Mermaid file.",
        ));
    };
    result.portable.validate()?;
    let warnings = result.warnings.into_iter().collect::<BTreeSet<_>>();
    result.warnings = warnings.into_iter().collect();
    Ok(result)
}
fn invalid(message: &str) -> BoardError {
    BoardError::Invalid(format!("Board import: {message}"))
}
fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or_default()
}
fn number(value: &Value, key: &str, fallback: f64) -> BoardResult<f64> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(fallback),
        Some(v) => v
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| invalid(&format!("{key} must be a finite number."))),
    }
}
fn point(value: &Value) -> BoardResult<Point> {
    if let Some(array) = value.as_array() {
        Ok(Point::new(
            array
                .first()
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .ok_or_else(|| invalid("Invalid point x."))?,
            array
                .get(1)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .ok_or_else(|| invalid("Invalid point y."))?,
        ))
    } else {
        Ok(Point::new(
            number(value, "x", 0.0)?,
            number(value, "y", 0.0)?,
        ))
    }
}
fn color(value: &str) -> BoardResult<Option<Color>> {
    if value.is_empty() || value == "transparent" || value == "none" {
        return Ok(None);
    }
    let value = value
        .strip_prefix('#')
        .ok_or_else(|| invalid("Legacy colors must be hexadecimal or transparent."))?;
    if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid(
            "Legacy colors must contain ASCII hexadecimal digits.",
        ));
    }
    let normalized = match value.len() {
        3 => value.chars().flat_map(|c| [c, c]).collect(),
        6 => value.to_string(),
        8 => value.to_string(),
        _ => return Err(invalid("Invalid legacy color.")),
    };
    let byte = |at| {
        u8::from_str_radix(&normalized[at..at + 2], 16).map_err(|_| invalid("Invalid color byte."))
    };
    Ok(Some(Color {
        r: byte(0)?,
        g: byte(2)?,
        b: byte(4)?,
        a: if normalized.len() == 8 { byte(6)? } else { 255 },
    }))
}
fn asset_data(url: &str) -> BoardResult<(Asset, AssetPayload)> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let (data, payload) = url
        .split_once(',')
        .ok_or_else(|| invalid("An image needs embedded base64 raster bytes."))?;
    if !data.starts_with("data:image/") || !data.ends_with(";base64") {
        return Err(invalid(
            "Remote images, SVG, video and non-base64 assets are not fetched. Embed a local PNG, JPEG, GIF or WebP before exporting the source board.",
        ));
    }
    if u64::try_from(payload.len()).unwrap_or(u64::MAX) > bruhs_models_image_budget() {
        return Err(invalid("The image exceeds the encoded attachment budget."));
    }
    let bytes = STANDARD
        .decode(payload)
        .map_err(|_| invalid("Invalid image base64."))?;
    AssetPayload::from_bytes(&bytes)
}
fn bruhs_models_image_budget() -> u64 {
    crate::logic::attachments::MAX_IMAGE_BYTES.div_ceil(3) * 4
}
fn finish_groups(board: &mut BoardDocument) -> BoardResult<()> {
    let mut pending = board
        .shapes
        .values()
        .filter(|s| matches!(s.kind, ShapeKind::Group))
        .map(|s| s.id.clone())
        .collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .filter(|id| {
                !board
                    .shapes
                    .values()
                    .any(|s| s.parent.as_ref() == Some(id) && pending.contains(&s.id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if ready.is_empty() {
            return Err(invalid("Legacy groups contain a parent cycle."));
        }
        for id in ready {
            if let Some(area) = board
                .shapes
                .values()
                .filter(|s| s.parent.as_ref() == Some(&id))
                .map(|s| bounds(board, s))
                .reduce(Rect::union)
            {
                board.shapes.get_mut(&id).ok_or(BoardError::Missing)?.rect = area;
            }
            pending.remove(&id);
        }
    }
    Ok(())
}
fn binding(
    board: &BoardDocument,
    target: &str,
    anchor: Option<Point>,
    actual: Point,
) -> BoardResult<Endpoint> {
    let shape = board
        .shapes
        .get(target)
        .filter(|s| s.bindable())
        .ok_or_else(|| invalid("An arrow binding refers to an unsupported or missing shape."))?;
    let local = actual.rotate(shape.rect.center(), -shape.rotation);
    let anchor = anchor.unwrap_or(Point::new(
        ((local.x - shape.rect.x) / shape.rect.w).clamp(0.0, 1.0),
        ((local.y - shape.rect.y) / shape.rect.h).clamp(0.0, 1.0),
    ));
    let mut endpoint = Endpoint {
        point: actual,
        binding: Some(Binding {
            shape_id: target.into(),
            anchor,
            offset: Point::default(),
        }),
    };
    let base = super::endpoint(board, &endpoint);
    if let Some(binding) = &mut endpoint.binding {
        binding.offset = Point::new(actual.x - base.x, actual.y - base.y)
            .rotate(Point::default(), -shape.rotation);
    }
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_sdk_export_keeps_shape_identity_geometry_text_and_bindings() {
        let fixture = include_str!("../../../bench/whiteboard/fixtures/tldraw-5.6.0.tldr");
        let expected: Value = serde_json::from_str(include_str!(
            "../../../bench/whiteboard/fixtures/tldraw-5.6.0.expected.json"
        ))
        .unwrap();
        let imported = import_board_file(fixture, "sdk.tldr", "/project".into(), 1).unwrap();
        let board = &imported.portable.board;
        assert_eq!(board.shapes.len(), 4);
        assert!(!board.agent_read);
        for arrow in expected["arrows"].as_array().unwrap() {
            let shape = &board.shapes[&format!("shape:{}", arrow["id"].as_str().unwrap())];
            let ShapeKind::Arrow { start, end } = &shape.kind else {
                panic!("editable arrow required")
            };
            for (actual, expected) in [
                (endpoint(board, start), &arrow["start"]),
                (endpoint(board, end), &arrow["end"]),
            ] {
                let expected = point(expected).unwrap();
                assert!(
                    actual.distance(expected) < 0.25,
                    "{actual:?} != {expected:?}"
                );
            }
        }
        assert_eq!(board.shapes["shape:node-00000"].text, "N0");
        assert_eq!(board.shapes["shape:node-00000"].rect.w, 120.0);
    }
    #[test]
    fn excalidraw_groups_frames_bound_text_drawings_and_local_images_remain_editable() {
        let imported = import_board_file(
            include_str!("../../../bench/whiteboard/fixtures/excalidraw-v2.excalidraw"),
            "legacy.excalidraw",
            "/project".into(),
            1,
        )
        .unwrap();
        let board = &imported.portable.board;
        assert_eq!(board.shapes["a"].text, "Browser");
        assert_eq!(board.shapes["a"].parent.as_deref(), Some("g"));
        assert_eq!(board.shapes["g"].parent.as_deref(), Some("frame"));
        assert_eq!(board.assets.len(), 1);
        assert_eq!(imported.portable.payloads.len(), 1);
        let ShapeKind::Arrow { start, end } = &board.shapes["edge"].kind else {
            panic!()
        };
        assert_eq!(endpoint(board, start), Point::new(120.0, 70.0));
        assert_eq!(endpoint(board, end), Point::new(240.0, 70.0));
        let mut editor = BoardEditor::new(board.clone()).unwrap();
        editor.selection.insert("a".into());
        editor.nudge(Point::new(10.0, 0.0), 2).unwrap();
        let ShapeKind::Arrow { start, .. } = &editor.document().shapes["edge"].kind else {
            panic!()
        };
        assert_eq!(endpoint(editor.document(), start), Point::new(130.0, 70.0));
        assert!(
            export_svg(editor.document(), &imported.portable.payloads, None)
                .unwrap()
                .contains("data:image/png;base64,")
        );
    }
    #[test]
    fn remote_images_and_future_schemas_fail_without_fetching_and_cross_project_imports_reset_links(
    ) {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../bench/whiteboard/fixtures/excalidraw-v2.excalidraw"
        ))
        .unwrap();
        value["files"]["raster"]["dataURL"] =
            Value::String("https://example.invalid/private.png".into());
        assert!(import_board_file(
            &value.to_string(),
            "remote.excalidraw",
            "/project".into(),
            1
        )
        .is_err());
        let mut board = benchmark_scene(4).unwrap();
        board
            .shapes
            .get_mut("node-00000")
            .unwrap()
            .links
            .push(BoardLink::Thread {
                id: "thread".into(),
            });
        let copy = PortableBoard {
            board,
            payloads: BTreeMap::new(),
        }
        .import_copy("/different".into(), 2)
        .unwrap();
        assert!(copy.board.shapes.values().all(|s| s.links.is_empty()));
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../bench/whiteboard/fixtures/tldraw-5.6.0.tldr"
        ))
        .unwrap();
        value["schema"]["sequences"]["com.tldraw.shape.geo"] = Value::from(999);
        assert!(
            import_board_file(&value.to_string(), "future.tldr", "/project".into(), 1).is_err()
        );
    }
}
