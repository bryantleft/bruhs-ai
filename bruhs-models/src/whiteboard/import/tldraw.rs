use super::*;
use crate::num::{ToFloatLossy as _, ToIntSaturating as _};
#[derive(Clone, Copy)]
struct Transform {
    origin: Point,
    angle: f64,
}
impl Transform {
    fn apply(self, p: Point) -> Point {
        p.rotate(Point::default(), self.angle).offset(self.origin)
    }
}
pub(super) fn convert(
    value: Value,
    project: String,
    name: &str,
    now: i64,
) -> BoardResult<BoardImport> {
    if value["tldrawFileFormatVersion"] != 1 || value["schema"]["schemaVersion"] != 2 {
        return Err(invalid("Use a tldraw file format 1 / schema 2 export."));
    }
    let schema = value["schema"]["sequences"]
        .as_object()
        .ok_or_else(|| invalid("The complete tldraw schema sequence map is required."))?;
    for (key, max) in [
        ("com.tldraw.store", 5),
        ("com.tldraw.asset", 1),
        ("com.tldraw.page", 1),
        ("com.tldraw.shape", 4),
        ("com.tldraw.shape.geo", 12),
        ("com.tldraw.shape.arrow", 8),
        ("com.tldraw.shape.draw", 5),
        ("com.tldraw.shape.text", 4),
        ("com.tldraw.shape.note", 13),
        ("com.tldraw.shape.frame", 1),
        ("com.tldraw.shape.group", 0),
        ("com.tldraw.shape.image", 5),
        ("com.tldraw.shape.line", 5),
        ("com.tldraw.shape.highlight", 4),
        ("com.tldraw.asset.image", 6),
        ("com.tldraw.binding.arrow", 1),
    ] {
        if let Some(value) = schema.get(key) {
            if value.as_u64().is_none_or(|n| n > max) {
                return Err(invalid(
                    "A consumed tldraw schema is newer than 5.6.0 or has an invalid version. Re-export the source board.",
                ));
            }
        }
    }
    let records = value["records"]
        .as_array()
        .ok_or_else(|| invalid("The .tldr records array is missing."))?;
    let assets = records
        .iter()
        .filter(|r| r["typeName"] == "asset")
        .map(|r| (text(r, "id"), r))
        .collect::<BTreeMap<_, _>>();
    let source = records
        .iter()
        .filter(|r| r["typeName"] == "shape")
        .map(|r| (text(r, "id").to_string(), r))
        .collect::<BTreeMap<_, _>>();
    if source.len() != records.iter().filter(|r| r["typeName"] == "shape").count() {
        return Err(invalid("Duplicate tldraw shape identities."));
    }
    let mut transforms = BTreeMap::<String, Transform>::new();
    let mut pages = BTreeMap::new();
    let mut pending = source.keys().cloned().collect::<BTreeSet<_>>();
    while !pending.is_empty() {
        let mut progress = false;
        let ids = pending.iter().cloned().collect::<Vec<_>>();
        for id in ids {
            let record = source[&id];
            let parent = text(record, "parentId");
            if source.contains_key(parent) && !transforms.contains_key(parent) {
                continue;
            }
            let local = Point::new(number(record, "x", 0.0)?, number(record, "y", 0.0)?);
            let angle = number(record, "rotation", 0.0)?;
            let transform = if let Some(parent) = transforms.get(parent) {
                Transform {
                    origin: parent.apply(local),
                    angle: parent.angle + angle,
                }
            } else {
                Transform {
                    origin: local,
                    angle,
                }
            };
            let page = pages
                .get(parent)
                .cloned()
                .unwrap_or_else(|| parent.to_string());
            pages.insert(id.clone(), page);
            transforms.insert(id.clone(), transform);
            pending.remove(&id);
            progress = true;
        }
        if !progress {
            return Err(invalid("The .tldr parent graph contains a cycle."));
        }
    }
    let mut children = BTreeMap::<String, Vec<&Value>>::new();
    for record in source.values() {
        children
            .entry(text(record, "parentId").to_string())
            .or_default()
            .push(record);
    }
    for siblings in children.values_mut() {
        siblings.sort_by(|a, b| {
            text(a, "index")
                .cmp(text(b, "index"))
                .then(text(a, "id").cmp(text(b, "id")))
        });
    }
    let mut ordered = Vec::new();
    let roots = children
        .keys()
        .filter(|id| !source.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    for root in roots {
        let mut stack = children[&root].iter().rev().copied().collect::<Vec<_>>();
        while let Some(record) = stack.pop() {
            ordered.push(record);
            if let Some(items) = children.get(text(record, "id")) {
                stack.extend(items.iter().rev().copied());
            }
        }
    }
    let mut board =
        BoardDocument::empty(uuid::Uuid::new_v4().to_string(), project, name.into(), now);
    let mut payloads = BTreeMap::new();
    let mut warnings=vec!["Imported fonts, rich text, pressure-sensitive strokes and hatch fills use local typography and editable board geometry.".into()];
    for (index, record) in ordered.into_iter().enumerate() {
        let id = text(record, "id").to_string();
        validate_id(&id)?;
        let transform = transforms[&id];
        let props = &record["props"];
        let scale = number(props, "scale", 1.0)?;
        let size = text(props, "size");
        let font =
            16.0 * match size {
                "s" => 1.125,
                "l" => 2.25,
                "xl" => 2.75,
                _ => 1.5,
            } * scale;
        let mut label = rich_text(&props["richText"]);
        if label.is_empty() {
            label = text(props, "text").into();
        }
        let w = number(
            props,
            "w",
            if record["type"] == "note" {
                200.0
            } else {
                180.0
            },
        )? * if matches!(text(record, "type"), "note" | "text") {
            scale
        } else {
            1.0
        };
        let h = number(
            props,
            "h",
            if record["type"] == "note" {
                200.0
            } else if record["type"] == "text" {
                font * 1.35 * label.lines().count().max(1).to_f64_lossy()
            } else {
                80.0
            },
        )? + number(props, "growY", 0.0)?;
        let h = if record["type"] == "note" {
            h * scale
        } else {
            h
        };
        let center = transform.apply(Point::new(w / 2.0, h / 2.0));
        let mut rect = Rect::new(
            center.x - w / 2.0,
            center.y - h / 2.0,
            w.max(1.0),
            h.max(1.0),
        );
        let mut rotation = transform.angle;
        let kind = match text(record, "type") {
            "geo" => match text(props, "geo") {
                "rectangle" => ShapeKind::Rectangle,
                "ellipse" | "oval" => ShapeKind::Ellipse,
                other => {
                    warnings.push(format!(
                        "tldraw '{other}' geometry becomes an editable rectangle."
                    ));
                    ShapeKind::Rectangle
                }
            },
            "text" => ShapeKind::Text,
            "note" => ShapeKind::Note,
            "frame" => {
                label = text(props, "name").into();
                ShapeKind::Frame
            }
            "group" => ShapeKind::Group,
            "arrow" => {
                if number(props, "bend", 0.0)? != 0.0 || props["kind"] == "elbow" {
                    warnings.push("Bent and elbow arrows become straight bound arrows.".into());
                }
                if !matches!(text(props, "arrowheadStart"), "" | "none")
                    || !matches!(text(props, "arrowheadEnd"), "" | "arrow" | "triangle")
                {
                    warnings.push("Arrowheads use the board's standard end arrowhead.".into());
                }
                let start = transform.apply(point(&props["start"])?);
                let end = transform.apply(point(&props["end"])?);
                rotation = 0.0;
                rect = Rect::new(
                    start.x.min(end.x),
                    start.y.min(end.y),
                    (end.x - start.x).abs().max(1.0),
                    (end.y - start.y).abs().max(1.0),
                );
                ShapeKind::Arrow {
                    start: Endpoint::free(start),
                    end: Endpoint::free(end),
                }
            }
            "line" | "draw" | "highlight" => {
                let mut points = Vec::new();
                if text(record, "type") == "line" {
                    let values = props["points"]
                        .as_object()
                        .ok_or_else(|| invalid("A line has no points."))?;
                    let mut values = values.values().collect::<Vec<_>>();
                    values.sort_by(|a, b| text(a, "index").cmp(text(b, "index")));
                    for value in values {
                        points.push(transform.apply(point(value)?));
                    }
                } else {
                    let segments = props["segments"]
                        .as_array()
                        .ok_or_else(|| invalid("A draw shape has no segments."))?;
                    for segment in segments {
                        for p in segment_points(segment)? {
                            points.push(transform.apply(Point::new(
                                p.x * number(props, "scaleX", 1.0)? * scale,
                                p.y * number(props, "scaleY", 1.0)? * scale,
                            )));
                        }
                    }
                }
                if points.len() < 2 {
                    return Err(invalid("A drawn shape needs at least two points."));
                }
                rotation = 0.0;
                rect = points
                    .iter()
                    .map(|p| Rect::new(p.x, p.y, 0.0, 0.0))
                    .reduce(Rect::union)
                    .ok_or(BoardError::Missing)?;
                rect.w = rect.w.max(1.0);
                rect.h = rect.h.max(1.0);
                ShapeKind::Freehand { points }
            }
            "image" => {
                if !props["crop"].is_null() || props["flipX"] == true || props["flipY"] == true {
                    return Err(invalid(
                        "Cropped or mirrored images require flattened local rasters in the source export.",
                    ));
                }
                let asset = assets
                    .get(text(props, "assetId"))
                    .ok_or_else(|| invalid("An image asset is missing."))?;
                let (asset, payload) = asset_data(text(&asset["props"], "src"))?;
                let asset_id = asset.id.clone();
                payloads.insert(asset.sha256.clone(), payload);
                board.assets.insert(asset.id.clone(), asset);
                ShapeKind::Image { asset_id }
            }
            other => {
                return Err(invalid(&format!(
                    "tldraw '{other}' shapes are unsupported. Replace embeds, bookmarks and videos with local raster images in the source copy."
                )));
            }
        };
        let mut shape = Shape::node(
            id.clone(),
            kind,
            rect,
            i64::try_from(index).map_err(|_| invalid("Too many shapes."))?,
            label,
        );
        shape.rotation = rotation;
        shape.parent = source
            .contains_key(text(record, "parentId"))
            .then(|| text(record, "parentId").to_string());
        let stroke = palette(text(props, "color"));
        shape.style.stroke = stroke;
        shape.style.stroke_width = match size {
            "s" => 1.0,
            "l" => 2.5,
            "xl" => 5.0,
            _ => 1.75,
        } * 2.0;
        shape.style.font_size = font.clamp(1.0, 512.0);
        shape.style.dashed = matches!(text(props, "dash"), "dashed" | "dotted");
        shape.style.fill = if matches!(
            shape.kind,
            ShapeKind::Text
                | ShapeKind::Arrow { .. }
                | ShapeKind::Frame
                | ShapeKind::Group
                | ShapeKind::Freehand { .. }
        ) || matches!(text(props, "fill"), "none" | "")
        {
            None
        } else if props["fill"] == "semi" {
            Some(Color::rgb(252, 255, 254))
        } else {
            Some(stroke)
        };
        if matches!(shape.kind, ShapeKind::Note) {
            shape.style.fill = Some(Color::rgb(250, 226, 150));
        }
        let opacity = (number(record, "opacity", 1.0)?.clamp(0.0, 1.0) * 255.0)
            .round()
            .to_u8_saturating();
        shape.style.stroke.a = opacity;
        if let Some(fill) = &mut shape.style.fill {
            fill.a = opacity;
        }
        board.shapes.insert(id, shape);
    }
    finish_groups(&mut board)?;
    let page_ids = pages.values().cloned().collect::<BTreeSet<_>>();
    if page_ids.len() > 1 {
        let mut cursor = 0.0;
        for page in page_ids {
            let ids = pages
                .iter()
                .filter(|(_, p)| **p == page)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            let area = ids
                .iter()
                .filter_map(|id| board.shapes.get(id))
                .map(|s| bounds(&board, s))
                .reduce(Rect::union)
                .unwrap_or(Rect::new(0.0, 0.0, 640.0, 480.0));
            let delta = Point::new(cursor - area.x, -area.y);
            let frame_id = uuid::Uuid::new_v4().to_string();
            for id in ids {
                let shape = board.shapes.get_mut(&id).ok_or(BoardError::Missing)?;
                shape.rect.x += delta.x;
                shape.rect.y += delta.y;
                match &mut shape.kind {
                    ShapeKind::Arrow { start, end } => {
                        start.point = start.point.offset(delta);
                        end.point = end.point.offset(delta);
                    }
                    ShapeKind::Freehand { points } => {
                        for p in points {
                            *p = p.offset(delta);
                        }
                    }
                    _ => {}
                }
                if shape.parent.is_none() {
                    shape.parent = Some(frame_id.clone());
                }
            }
            let title = records
                .iter()
                .find(|r| r["id"] == page)
                .map_or(page.as_str(), |r| text(r, "name"));
            let frame = Shape::node(
                frame_id.clone(),
                ShapeKind::Frame,
                Rect::new(cursor - 20.0, -50.0, area.w + 40.0, area.h + 70.0),
                -2,
                title.into(),
            );
            board.shapes.insert(frame_id, frame);
            cursor += area.w + 200.0;
        }
        warnings
            .push("Multiple pages become separate editable frames arranged side by side.".into());
    }
    let bindings = records
        .iter()
        .filter(|r| r["typeName"] == "binding" && r["type"] == "arrow")
        .collect::<Vec<_>>();
    for binding_record in &bindings {
        let from = text(binding_record, "fromId");
        let to = text(binding_record, "toId");
        let props = &binding_record["props"];
        let arrow = board
            .shapes
            .get(from)
            .ok_or_else(|| invalid("A binding has no arrow."))?;
        let ShapeKind::Arrow { start, end } = &arrow.kind else {
            return Err(invalid("A binding source is not an arrow."));
        };
        if !matches!(text(props, "terminal"), "start" | "end") {
            return Err(invalid("An arrow binding has an invalid terminal."));
        }
        let other = bindings
            .iter()
            .find(|b| text(b, "fromId") == from && b["props"]["terminal"] != props["terminal"])
            .and_then(|b| {
                board
                    .shapes
                    .get(text(b, "toId"))
                    .map(|target| target.rect.center())
            })
            .unwrap_or_else(|| {
                if props["terminal"] == "start" {
                    endpoint(&board, end)
                } else {
                    endpoint(&board, start)
                }
            });
        let target = board
            .shapes
            .get(to)
            .ok_or_else(|| invalid("An arrow target is missing."))?;
        let anchor = point(&props["normalizedAnchor"])?;
        let anchor = if props["isExact"] == true {
            anchor
        } else {
            warnings.push("Non-exact tldraw bindings use the target outline; source-tool decorative gaps may differ.".into());
            outline_anchor(target, other)
        };
        let actual = Point::new(
            target.rect.x + anchor.x * target.rect.w,
            target.rect.y + anchor.y * target.rect.h,
        )
        .rotate(target.rect.center(), target.rotation);
        let terminal = binding(&board, to, Some(anchor), actual)?;
        if let ShapeKind::Arrow { start, end } =
            &mut board.shapes.get_mut(from).ok_or(BoardError::Missing)?.kind
        {
            if props["terminal"] == "start" {
                *start = terminal;
            } else {
                *end = terminal;
            }
        }
    }
    Ok(BoardImport {
        portable: PortableBoard { board, payloads },
        warnings,
    })
}
fn rich_text(value: &Value) -> String {
    fn walk(value: &Value, out: &mut String) {
        if let Some(text) = value["text"].as_str() {
            out.push_str(text);
        }
        if value["type"] == "hardBreak" {
            out.push('\n');
        }
        if let Some(children) = value["content"].as_array() {
            for child in children {
                walk(child, out);
            }
        }
        if value["type"] == "paragraph" && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    let mut out = String::new();
    walk(value, &mut out);
    out.trim_end_matches('\n').into()
}
fn palette(name: &str) -> Color {
    match name {
        "red" | "light-red" => Color::rgb(224, 49, 49),
        "blue" | "light-blue" => Color::rgb(32, 114, 226),
        "green" | "light-green" => Color::rgb(9, 146, 104),
        "orange" => Color::rgb(247, 103, 7),
        "yellow" => Color::rgb(245, 159, 0),
        "violet" | "light-violet" => Color::rgb(132, 87, 220),
        "grey" => Color::rgb(144, 144, 144),
        "white" => Color::rgb(252, 255, 254),
        _ => Color::rgb(29, 29, 29),
    }
}
fn segment_points(segment: &Value) -> BoardResult<Vec<Point>> {
    if let Some(points) = segment["points"].as_array() {
        return points.iter().map(point).collect();
    }
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let bytes = STANDARD
        .decode(text(segment, "path"))
        .map_err(|_| invalid("Invalid packed draw path."))?;
    let dimensions = segment["dim"].as_u64().unwrap_or(3);
    if dimensions != 2 && dimensions != 3 {
        return Err(invalid("Unsupported packed path dimensions."));
    }
    let dim = usize::try_from(dimensions).map_err(|_| invalid("Invalid path dimension."))?;
    let first = dim * 4;
    let step = dim * 2;
    if bytes.len() < first || (bytes.len() - first) % step != 0 {
        return Err(invalid("The packed draw path is truncated."));
    }
    let f32_at = |at: usize| -> BoardResult<f64> {
        Ok(f64::from(f32::from_le_bytes(
            bytes
                .get(at..at + 4)
                .ok_or_else(|| invalid("Truncated point."))?
                .try_into()
                .map_err(|_| invalid("Invalid point bytes."))?,
        )))
    };
    let mut at = Point::new(f32_at(0)?, f32_at(4)?);
    let mut points = vec![at];
    for chunk in bytes[first..].chunks_exact(step) {
        let half = |offset| {
            let bits = u16::from_le_bytes([chunk[offset], chunk[offset + 1]]);
            half_float(bits)
        };
        at = at.offset(Point::new(half(0), half(2)));
        if !at.finite() {
            return Err(invalid("The draw path contains a non-finite coordinate."));
        }
        points.push(at);
    }
    Ok(points)
}
fn half_float(bits: u16) -> f64 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 31);
    let fraction = f64::from(bits & 1023);
    if exponent == 0 {
        sign * 2f64.powi(-14) * (fraction / 1024.0)
    } else if exponent == 31 {
        f64::NAN
    } else {
        sign * 2f64.powi(exponent - 15) * (1.0 + fraction / 1024.0)
    }
}

fn outline_anchor(target: &Shape, toward: Point) -> Point {
    let local = toward.rotate(target.rect.center(), -target.rotation);
    let mut dx = (local.x - target.rect.center().x) / (target.rect.w / 2.0);
    let dy = (local.y - target.rect.center().y) / (target.rect.h / 2.0);
    if dx == 0.0 && dy == 0.0 {
        dx = 1.0;
    }
    let scale = if matches!(target.kind, ShapeKind::Ellipse) {
        1.0 / dx.hypot(dy)
    } else {
        1.0 / dx.abs().max(dy.abs())
    };
    Point::new(
        (0.5 + dx * scale / 2.0).clamp(0.0, 1.0),
        (0.5 + dy * scale / 2.0).clamp(0.0, 1.0),
    )
}
