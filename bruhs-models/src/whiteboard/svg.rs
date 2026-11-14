//! Inert SVG export: text is escaped and images are validated local raster data.
use super::*;
use crate::num::ToFloatLossy as _;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn export_svg(
    board: &BoardDocument,
    payloads: &BTreeMap<String, AssetPayload>,
    selection: Option<&BTreeSet<ShapeId>>,
) -> BoardResult<String> {
    board.validate()?;
    let mut shapes = board
        .shapes
        .values()
        .filter(|shape| selection.is_none_or(|ids| ids.contains(&shape.id)))
        .collect::<Vec<_>>();
    shapes.sort_by_key(|s| s.z);
    let area = shapes
        .iter()
        .map(|shape| bounds(board, shape))
        .reduce(Rect::union)
        .unwrap_or(Rect::new(0.0, 0.0, 640.0, 480.0))
        .expanded(20.0);
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"{} {} {} {}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>",
        area.w,
        area.h,
        area.x,
        area.y,
        area.w,
        area.h,
        area.x,
        area.y,
        area.w,
        area.h,
        board.background.css()
    );
    for shape in shapes {
        if matches!(shape.kind, ShapeKind::Group) {
            continue;
        }
        let r = shape.rect;
        let c = r.center();
        let stroke = shape.style.stroke.css();
        let fill = shape.style.fill.map_or_else(|| "none".into(), |c| c.css());
        let dash = if shape.style.dashed {
            " stroke-dasharray=\"6 4\""
        } else {
            ""
        };
        write!(out,"<g stroke=\"{stroke}\" stroke-width=\"{}\" fill=\"{fill}\" stroke-linecap=\"round\" stroke-linejoin=\"round\"{dash}",shape.style.stroke_width).map_err(|e|BoardError::Invalid(e.to_string()))?;
        if shape.rotation != 0.0 && !matches!(shape.kind, ShapeKind::Arrow { .. }) {
            write!(
                out,
                " transform=\"rotate({} {} {})\"",
                shape.rotation.to_degrees(),
                c.x,
                c.y
            )
            .map_err(|e| BoardError::Invalid(e.to_string()))?;
        }
        out.push('>');
        match &shape.kind {
            ShapeKind::Rectangle | ShapeKind::Note | ShapeKind::Frame => {
                write!(
                    out,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{}/>",
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    if matches!(shape.kind, ShapeKind::Frame) {
                        " fill=\"none\""
                    } else {
                        ""
                    }
                )
                .map_err(|e| BoardError::Invalid(e.to_string()))?;
            }
            ShapeKind::Ellipse => {
                write!(
                    out,
                    "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"/>",
                    c.x,
                    c.y,
                    r.w / 2.0,
                    r.h / 2.0
                )
                .map_err(|e| BoardError::Invalid(e.to_string()))?;
            }
            ShapeKind::Arrow { start, end } => {
                let a = endpoint(board, start);
                let b = endpoint(board, end);
                write!(
                    out,
                    "<path fill=\"none\" d=\"M {} {} L {} {}\"/>",
                    a.x, a.y, b.x, b.y
                )
                .map_err(|e| BoardError::Invalid(e.to_string()))?;
                let length = a.distance(b);
                if length > 0.0 {
                    let head = shape.style.stroke_width * 5.0;
                    let dx = (b.x - a.x) / length;
                    let dy = (b.y - a.y) / length;
                    write!(
                        out,
                        "<path stroke=\"none\" fill=\"{stroke}\" d=\"M {} {} L {} {} L {} {} Z\"/>",
                        b.x,
                        b.y,
                        b.x - dx * head - dy * head / 2.0,
                        b.y - dy * head + dx * head / 2.0,
                        b.x - dx * head + dy * head / 2.0,
                        b.y - dy * head - dx * head / 2.0
                    )
                    .map_err(|e| BoardError::Invalid(e.to_string()))?;
                }
            }
            ShapeKind::Freehand { points } => {
                out.push_str("<polyline fill=\"none\" points=\"");
                for point in points {
                    write!(out, "{},{} ", point.x, point.y)
                        .map_err(|e| BoardError::Invalid(e.to_string()))?;
                }
                out.push_str("\"/>");
            }
            ShapeKind::Image { asset_id } => {
                let asset = &board.assets[asset_id];
                let payload = payloads.get(&asset.sha256).ok_or_else(|| {
                    BoardError::Invalid("Restore the missing image before exporting.".into())
                })?;
                let url = payload.data_url(asset)?;
                write!(
                    out,
                    "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" href=\"{}\"/>",
                    r.x, r.y, r.w, r.h, url
                )
                .map_err(|e| BoardError::Invalid(e.to_string()))?;
            }
            _ => {}
        }
        if !shape.text.is_empty() {
            let centered = !matches!(shape.kind, ShapeKind::Text | ShapeKind::Frame);
            let anchor = text_anchor(board, shape);
            write!(out,"<text stroke=\"none\" fill=\"{stroke}\" font-family=\"MonaspaceArgonVariable,Monaspace Argon,monospace\" font-size=\"{}\" dominant-baseline=\"central\" text-anchor=\"{}\">",shape.style.font_size,if centered{"middle"}else{"start"}).map_err(|e|BoardError::Invalid(e.to_string()))?;
            for (index, line) in shape.text.lines().enumerate() {
                write!(
                    out,
                    "<tspan x=\"{}\" y=\"{}\">{}</tspan>",
                    anchor.x,
                    anchor.y + index.to_f64_lossy() * shape.style.font_size * 1.35,
                    xml(line)
                )
                .map_err(|e| BoardError::Invalid(e.to_string()))?;
            }
            out.push_str("</text>");
        }
        out.push_str("</g>");
    }
    out.push_str("</svg>");
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn svg_escapes_markup_and_contains_no_external_resource() {
        let mut board = benchmark_scene(1000).unwrap();
        board.shapes.get_mut("node-00000").unwrap().text = "<script>&\"".into();
        let svg = export_svg(&board, &BTreeMap::new(), None).unwrap();
        assert!(svg.contains("&lt;script&gt;&amp;&quot;"));
        assert!(!svg.contains("<script>"));
        assert!(!svg.contains("href=\"http"));
    }
}
