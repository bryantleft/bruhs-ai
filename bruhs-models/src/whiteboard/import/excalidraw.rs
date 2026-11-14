use super::*;
use crate::num::ToIntSaturating as _;
pub(super) fn convert(
    value: Value,
    project: String,
    name: &str,
    now: i64,
) -> BoardResult<BoardImport> {
    if value["version"] != 2 {
        return Err(invalid("Only Excalidraw JSON version 2 is supported."));
    }
    let elements = value["elements"]
        .as_array()
        .ok_or_else(|| invalid("Excalidraw elements are missing."))?;
    let mut board =
        BoardDocument::empty(uuid::Uuid::new_v4().to_string(), project, name.into(), now);
    if let Some(background) = value["appState"]["viewBackgroundColor"].as_str() {
        if let Some(color) = color(background)? {
            board.background = color;
        }
    }
    let mut payloads = BTreeMap::new();
    let mut warnings=vec!["Imported fonts and hand-drawn strokes use the board's local typography and editable geometry.".into()];
    let active = elements
        .iter()
        .filter(|e| !e["isDeleted"].as_bool().unwrap_or(false))
        .collect::<Vec<_>>();
    let mut bindings = Vec::new();
    let mut bound_text = Vec::new();
    let mut group_frames = BTreeMap::new();
    for (index, element) in active.iter().enumerate() {
        let id = text(element, "id").to_string();
        validate_id(&id)?;
        if board.shapes.contains_key(&id) {
            return Err(invalid("Duplicate element identities."));
        }
        let x = number(element, "x", 0.0)?;
        let y = number(element, "y", 0.0)?;
        let w = number(element, "width", 1.0)?.max(1.0);
        let h = number(element, "height", 1.0)?.max(1.0);
        let rotation = number(element, "angle", 0.0)?;
        let mut rect = Rect::new(x, y, w, h);
        let mut label = String::new();
        let kind = match text(element, "type") {
            "rectangle" | "diamond" => {
                if text(element, "type") == "diamond" {
                    warnings
                        .push("Diamonds become editable rectangles; review their geometry.".into());
                }
                ShapeKind::Rectangle
            }
            "ellipse" => ShapeKind::Ellipse,
            "frame" => {
                label = text(element, "name").into();
                ShapeKind::Frame
            }
            "text" => {
                label = text(element, "text").into();
                if let Some(container) = element["containerId"].as_str() {
                    bound_text.push((
                        container.to_string(),
                        label,
                        number(element, "fontSize", 20.0)?,
                    ));
                    continue;
                }
                ShapeKind::Text
            }
            "arrow" => {
                let points = element["points"]
                    .as_array()
                    .filter(|p| p.len() >= 2)
                    .ok_or_else(|| invalid("An arrow needs two endpoints."))?;
                if points.len() > 2 || element["elbowed"].as_bool() == Some(true) {
                    warnings.push("Bent and elbow arrows become straight bound arrows between their original endpoints.".into());
                }
                let world = |p: &Value| {
                    point(p).map(|p| p.offset(Point::new(x, y)).rotate(rect.center(), rotation))
                };
                let start = world(&points[0])?;
                let end = world(points.last().ok_or(BoardError::Missing)?)?;
                let reverse = element["startArrowhead"]
                    .as_str()
                    .is_some_and(|head| !head.is_empty())
                    && element.get("endArrowhead") == Some(&Value::Null);
                let (start, end, start_binding, end_binding) = if reverse {
                    warnings.push("Start-only arrowheads keep their direction by reversing the editable arrow endpoints.".into());
                    (
                        end,
                        start,
                        element["endBinding"].clone(),
                        element["startBinding"].clone(),
                    )
                } else {
                    (
                        start,
                        end,
                        element["startBinding"].clone(),
                        element["endBinding"].clone(),
                    )
                };
                if !reverse
                    && (element["startArrowhead"].as_str().is_some()
                        || element.get("endArrowhead") == Some(&Value::Null)
                        || element["endArrowhead"]
                            .as_str()
                            .is_some_and(|head| head != "arrow" && head != "triangle"))
                {
                    warnings.push(
                        "Custom or absent arrowheads use the board's standard end arrowhead."
                            .into(),
                    );
                }
                bindings.push((id.clone(), start_binding, end_binding, start, end));
                rect = Rect::new(
                    start.x.min(end.x),
                    start.y.min(end.y),
                    (start.x - end.x).abs().max(1.0),
                    (start.y - end.y).abs().max(1.0),
                );
                ShapeKind::Arrow {
                    start: Endpoint::free(start),
                    end: Endpoint::free(end),
                }
            }
            "line" | "freedraw" => {
                let points = element["points"]
                    .as_array()
                    .ok_or_else(|| invalid("A drawn element has no points."))?
                    .iter()
                    .map(|p| point(p).map(|p| p.offset(Point::new(x, y))))
                    .collect::<BoardResult<Vec<_>>>()?;
                if points.len() < 2 {
                    return Err(invalid("A drawn line needs at least two points."));
                }
                ShapeKind::Freehand { points }
            }
            "image" => {
                if !element["crop"].is_null() {
                    return Err(invalid(
                        "Cropped images require a flattened local raster in the source export.",
                    ));
                }
                if element["scale"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|v| v.as_f64().is_some_and(|v| v < 0.0)))
                {
                    return Err(invalid(
                        "Mirrored images require a flattened local raster in the source export.",
                    ));
                }
                let file_id = text(element, "fileId");
                let url = value["files"][file_id]["dataURL"]
                    .as_str()
                    .ok_or_else(|| invalid("An image is missing embedded file bytes."))?;
                let (asset, payload) = asset_data(url)?;
                let asset_id = asset.id.clone();
                payloads.insert(asset.sha256.clone(), payload);
                board.assets.insert(asset.id.clone(), asset);
                ShapeKind::Image { asset_id }
            }
            other => {
                return Err(invalid(&format!(
                    "Excalidraw element '{other}' is unsupported. Export that element as a local raster or remove it from the source copy."
                )));
            }
        };
        let mut shape = Shape::node(
            id.clone(),
            kind,
            rect,
            i64::try_from(index).map_err(|_| invalid("Too many elements."))?,
            label,
        );
        if !matches!(shape.kind, ShapeKind::Arrow { .. }) {
            shape.rotation = rotation;
        }
        shape.style.stroke = color(text(element, "strokeColor"))?.unwrap_or(Color::rgb(29, 29, 29));
        shape.style.fill = if matches!(
            shape.kind,
            ShapeKind::Text
                | ShapeKind::Freehand { .. }
                | ShapeKind::Arrow { .. }
                | ShapeKind::Frame
        ) {
            None
        } else {
            color(text(element, "backgroundColor"))?
        };
        shape.style.stroke_width = number(element, "strokeWidth", 2.0)?.clamp(0.25, 128.0);
        shape.style.font_size = number(element, "fontSize", 20.0)?.clamp(1.0, 512.0);
        shape.style.dashed = matches!(text(element, "strokeStyle"), "dashed" | "dotted");
        let opacity = (number(element, "opacity", 100.0)?.clamp(0.0, 100.0) * 2.55)
            .round()
            .to_u8_saturating();
        shape.style.stroke.a = opacity;
        if let Some(fill) = &mut shape.style.fill {
            fill.a = opacity;
        }
        if !matches!(text(element, "fillStyle"), "" | "solid") {
            warnings.push("Hachure and cross-hatch fills become solid fills.".into());
        }
        let groups = element["groupIds"]
            .as_array()
            .map(|g| g.iter().filter_map(Value::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        let frame = element["frameId"].as_str().map(str::to_string);
        for (index, group) in groups.iter().enumerate() {
            validate_id(group)?;
            let parent = groups
                .get(index + 1)
                .map(|s| (*s).to_string())
                .or_else(|| frame.clone());
            if group_frames
                .insert((*group).to_string(), parent.clone())
                .is_some_and(|old| old != parent)
            {
                return Err(invalid("A group has conflicting parents."));
            }
        }
        shape.parent = groups.first().map(|g| (*g).to_string()).or(frame);
        board.shapes.insert(id, shape);
    }
    for (id, parent) in group_frames {
        if board.shapes.contains_key(&id) {
            return Err(invalid("A group identity conflicts with an element."));
        }
        let mut group = Shape::node(
            id.clone(),
            ShapeKind::Group,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            -1,
            String::new(),
        );
        group.parent = parent;
        board.shapes.insert(id, group);
    }
    finish_groups(&mut board)?;
    for (id, start_binding, end_binding, start, end) in bindings {
        let resolve = |value: Value, actual: Point| -> BoardResult<Endpoint> {
            match value["elementId"].as_str() {
                Some(target) => binding(
                    &board,
                    target,
                    value["fixedPoint"]
                        .as_array()
                        .map(|_| point(&value["fixedPoint"]))
                        .transpose()?,
                    actual,
                ),
                None => Ok(Endpoint::free(actual)),
            }
        };
        let start = resolve(start_binding, start)?;
        let end = resolve(end_binding, end)?;
        board.shapes.get_mut(&id).ok_or(BoardError::Missing)?.kind =
            ShapeKind::Arrow { start, end };
    }
    for (id, label, size) in bound_text {
        let shape = board
            .shapes
            .get_mut(&id)
            .ok_or_else(|| invalid("Bound text refers to a missing container."))?;
        shape.text = label;
        shape.style.font_size = size;
    }
    Ok(BoardImport {
        portable: PortableBoard { board, payloads },
        warnings,
    })
}
