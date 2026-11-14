//! GPUI paths and cached native glyph runs; no webview or SVG scene conversion.
use bruhs_models::{num::ToFloatLossy as _, whiteboard as board};
use gpui::*;
use std::collections::BTreeMap;

#[derive(Default, Clone)]
pub(crate) struct BoardImages {
    pub assets: BTreeMap<String, std::sync::Arc<RenderImage>>,
    pub layers: BTreeMap<String, BoardLayer>,
}
#[derive(Clone)]
pub(crate) struct BoardLayer {
    pub shape: board::Shape,
    pub bounds: board::Rect,
    pub image: std::sync::Arc<RenderImage>,
}
impl BoardLayer {
    pub fn matches(&self, shape: &board::Shape) -> bool {
        self.shape.kind == shape.kind
            && self.shape.rotation == shape.rotation
            && self.shape.style == shape.style
            && self.shape.text == shape.text
            && self.shape.rect.w == shape.rect.w
            && self.shape.rect.h == shape.rect.h
    }
}
impl BoardImages {
    pub fn current(&self, document: &board::BoardDocument) -> bool {
        document
            .assets
            .values()
            .all(|a| self.assets.contains_key(&a.sha256))
            && document
                .shapes
                .values()
                .filter(|shape| {
                    shape.rotation != 0.0
                        && (!shape.text.is_empty()
                            || matches!(shape.kind, board::ShapeKind::Image { .. }))
                })
                .all(|shape| {
                    self.layers
                        .get(&shape.id)
                        .is_some_and(|layer| layer.matches(shape))
                })
    }
}
#[derive(Default)]
pub(crate) struct BoardPainter {
    text: BTreeMap<String, (String, u32, board::Color, Vec<ShapedLine>)>,
    pub images: BoardImages,
    pub draws: u64,
    pub painted: usize,
}
fn color(value: board::Color) -> Hsla {
    rgba(u32::from_be_bytes([value.r, value.g, value.b, value.a])).into()
}
fn screen(point: board::Point, camera: board::Camera, origin: Point<Pixels>) -> Point<Pixels> {
    let position = camera.screen(point);
    origin + gpui::point(px(position.x.to_f32_lossy()), px(position.y.to_f32_lossy()))
}
impl BoardPainter {
    pub fn draw(
        &mut self,
        editor: &board::BoardEditor,
        area: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let camera = editor.camera;
        let doc = editor.document();
        let viewport = camera.viewport(
            f64::from(f32::from(area.size.width)),
            f64::from(f32::from(area.size.height)),
        );
        self.painted = 0;
        window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
            window.paint_quad(fill(area, color(doc.background)));
            for shape in editor.ordered() {
                if matches!(shape.kind, board::ShapeKind::Group) {
                    continue;
                }
                let delta = editor
                    .movement
                    .as_ref()
                    .filter(|m| m.ids.contains(&shape.id))
                    .map_or(board::Point::default(), |m| m.delta);
                let bounds = board::display_bounds(editor, shape);
                if !bounds.intersects(viewport) {
                    continue;
                }
                self.painted += 1;
                if let Some(layer) = self
                    .images
                    .layers
                    .get(&shape.id)
                    .filter(|layer| layer.matches(shape))
                {
                    let shift = board::Point::new(
                        shape.rect.x - layer.shape.rect.x,
                        shape.rect.y - layer.shape.rect.y,
                    )
                    .offset(delta);
                    let r = layer.bounds.translated(shift);
                    let a = screen(board::Point::new(r.x, r.y), camera, area.origin);
                    let b = screen(
                        board::Point::new(r.right(), r.bottom()),
                        camera,
                        area.origin,
                    );
                    if let Err(error) = window.paint_image(
                        Bounds::from_corners(a, b),
                        Corners::default(),
                        layer.image.clone(),
                        0,
                        false,
                    ) {
                        tracing::warn!(%error,"Whiteboard layer could not be painted");
                    }
                    continue;
                }

                let map = |p: board::Point| {
                    screen(
                        p.rotate(shape.rect.center(), shape.rotation).offset(delta),
                        camera,
                        area.origin,
                    )
                };
                let mut points = Vec::new();
                match &shape.kind {
                    board::ShapeKind::Image { asset_id } => {
                        if let Some(image) = doc
                            .assets
                            .get(asset_id)
                            .and_then(|asset| self.images.assets.get(&asset.sha256))
                        {
                            let r = shape.rect;
                            let a = screen(
                                board::Point::new(r.x, r.y).offset(delta),
                                camera,
                                area.origin,
                            );
                            let b = screen(
                                board::Point::new(r.right(), r.bottom()).offset(delta),
                                camera,
                                area.origin,
                            );
                            if let Err(error) = window.paint_image(
                                Bounds::from_corners(a, b),
                                Corners::default(),
                                image.clone(),
                                0,
                                false,
                            ) {
                                tracing::warn!(%error, "Whiteboard image could not be painted");
                            }
                        }
                    }
                    board::ShapeKind::Arrow { start, end } => {
                        let a = screen(
                            board::display_endpoint(editor, shape, start),
                            camera,
                            area.origin,
                        );
                        let b = screen(
                            board::display_endpoint(editor, shape, end),
                            camera,
                            area.origin,
                        );
                        stroke_styled(
                            &[a, b],
                            false,
                            px((shape.style.stroke_width * camera.zoom).to_f32_lossy()),
                            color(shape.style.stroke),
                            shape.style.dashed,
                            camera.zoom,
                            window,
                        );
                        let dx = f32::from(b.x - a.x);
                        let dy = f32::from(b.y - a.y);
                        let length = dx.hypot(dy);
                        if length > 0.0 {
                            let size =
                                (shape.style.stroke_width * 5.0 * camera.zoom).to_f32_lossy();
                            let dx = dx / length;
                            let dy = dy / length;
                            fill_path(
                                &[
                                    b,
                                    b + point(
                                        px(-dx * size - dy * size / 2.0),
                                        px(-dy * size + dx * size / 2.0),
                                    ),
                                    b + point(
                                        px(-dx * size + dy * size / 2.0),
                                        px(-dy * size - dx * size / 2.0),
                                    ),
                                ],
                                color(shape.style.stroke),
                                window,
                            );
                        }
                    }
                    board::ShapeKind::Freehand { points: source } => {
                        points.extend(source.iter().map(|p| map(*p)));
                    }
                    board::ShapeKind::Ellipse => {
                        // A closed cubic ellipse avoids a resolution-dependent polygon.
                        let center = shape.rect.center();
                        let rx = shape.rect.w / 2.0;
                        let ry = shape.rect.h / 2.0;
                        let build = |mut path: PathBuilder| {
                            let k = 0.552_284_749_830_793_6;
                            path.move_to(map(board::Point {
                                x: center.x + rx,
                                y: center.y,
                            }));
                            for (end, a, b) in [
                                (
                                    (center.x, center.y + ry),
                                    (center.x + rx, center.y + ry * k),
                                    (center.x + rx * k, center.y + ry),
                                ),
                                (
                                    (center.x - rx, center.y),
                                    (center.x - rx * k, center.y + ry),
                                    (center.x - rx, center.y + ry * k),
                                ),
                                (
                                    (center.x, center.y - ry),
                                    (center.x - rx, center.y - ry * k),
                                    (center.x - rx * k, center.y - ry),
                                ),
                                (
                                    (center.x + rx, center.y),
                                    (center.x + rx * k, center.y - ry),
                                    (center.x + rx, center.y - ry * k),
                                ),
                            ] {
                                path.cubic_bezier_to(
                                    map(board::Point { x: end.0, y: end.1 }),
                                    map(board::Point { x: a.0, y: a.1 }),
                                    map(board::Point { x: b.0, y: b.1 }),
                                );
                            }
                            path.close();
                            path.build()
                        };
                        if let Some(fill) = shape.style.fill {
                            if let Ok(path) = build(PathBuilder::fill()) {
                                window.paint_path(path, color(fill));
                            }
                        }
                        if let Ok(path) = build(PathBuilder::stroke(px((shape.style.stroke_width
                            * camera.zoom)
                            .to_f32_lossy())))
                        {
                            window.paint_path(path, color(shape.style.stroke));
                        }
                    }
                    board::ShapeKind::Text | board::ShapeKind::Group => {}
                    _ => {
                        let r = shape.rect;
                        points.extend(
                            [
                                board::Point { x: r.x, y: r.y },
                                board::Point {
                                    x: r.right(),
                                    y: r.y,
                                },
                                board::Point {
                                    x: r.right(),
                                    y: r.bottom(),
                                },
                                board::Point {
                                    x: r.x,
                                    y: r.bottom(),
                                },
                            ]
                            .map(map),
                        );
                        if !matches!(shape.kind, board::ShapeKind::Frame) {
                            if let Some(fill) = shape.style.fill {
                                fill_path(&points, color(fill), window);
                            }
                        }
                    }
                }
                if !points.is_empty() {
                    stroke(
                        &points,
                        !matches!(shape.kind, board::ShapeKind::Freehand { .. }),
                        px((shape.style.stroke_width * camera.zoom).to_f32_lossy()),
                        color(shape.style.stroke),
                        window,
                    );
                }
                if !shape.text.is_empty() {
                    let font_size = (shape.style.font_size * camera.zoom).to_f32_lossy();
                    let key = font_size.to_bits();
                    let needs_layout =
                        self.text
                            .get(&shape.id)
                            .is_none_or(|(text, size, stroke, _)| {
                                text != &shape.text || *size != key || stroke != &shape.style.stroke
                            });
                    if needs_layout {
                        let runs = shape
                            .text
                            .lines()
                            .map(|line| {
                                let text: SharedString = line.to_owned().into();
                                let len = text.len();
                                window.text_system().shape_line(
                                    text,
                                    px(font_size),
                                    &[TextRun {
                                        len,
                                        font: font("Monaspace Argon"),
                                        color: color(shape.style.stroke),
                                        ..Default::default()
                                    }],
                                    None,
                                )
                            })
                            .collect();
                        self.text.insert(
                            shape.id.clone(),
                            (shape.text.clone(), key, shape.style.stroke, runs),
                        );
                    }
                    if let Some((_, _, _, lines)) = self.text.get(&shape.id) {
                        let centered =
                            !matches!(shape.kind, board::ShapeKind::Text | board::ShapeKind::Frame);
                        let anchor = screen(
                            board::display_text_anchor(editor, shape),
                            camera,
                            area.origin,
                        );
                        for (index, line) in lines.iter().enumerate() {
                            let position = anchor
                                + point(
                                    if centered { -line.width / 2.0 } else { px(0.0) },
                                    px(-font_size * 0.675
                                        + index.to_f32_lossy() * font_size * 1.35),
                                );
                            if let Err(error) = line.paint(
                                position,
                                px(font_size * 1.35),
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            ) {
                                tracing::warn!(%error,"Whiteboard text could not be painted");
                            }
                        }
                    }
                }
            }
            if let Some(selected) = editor.selected_bounds() {
                let r = editor
                    .movement
                    .as_ref()
                    .map_or(selected, |m| selected.translated(m.delta));
                let a = screen(board::Point { x: r.x, y: r.y }, camera, area.origin);
                let b = screen(
                    board::Point {
                        x: r.right(),
                        y: r.bottom(),
                    },
                    camera,
                    area.origin,
                );
                window.paint_quad(quad(
                    Bounds::from_corners(a, b),
                    px(0.0),
                    transparent_black(),
                    px(1.0),
                    rgb(0x4169e1),
                    BorderStyle::default(),
                ));
            }
            if let Some(selection) = editor.selection_box {
                let a = screen(
                    board::Point::new(selection.x, selection.y),
                    camera,
                    area.origin,
                );
                let b = screen(
                    board::Point::new(selection.right(), selection.bottom()),
                    camera,
                    area.origin,
                );
                window.paint_quad(quad(
                    Bounds::from_corners(a, b),
                    px(0.0),
                    rgba(0x4169e120),
                    px(1.0),
                    rgb(0x4169e1),
                    BorderStyle::default(),
                ));
            }
        });
        self.text.retain(|id, _| doc.shapes.contains_key(id));
        self.draws = self.draws.wrapping_add(1);
    }
}
fn stroke_styled(
    points: &[Point<Pixels>],
    closed: bool,
    width: Pixels,
    color: Hsla,
    dashed: bool,
    zoom: f64,
    window: &mut Window,
) {
    if !dashed {
        stroke(points, closed, width, color, window);
        return;
    }
    let dash = (6.0 * zoom).to_f32_lossy();
    let gap = (4.0 * zoom).to_f32_lossy();
    for segment in points.windows(2) {
        let delta = segment[1] - segment[0];
        let length = f32::from(delta.x).hypot(f32::from(delta.y));
        let mut start = 0.0;
        while start < length {
            let end = (start + dash).min(length);
            let a = segment[0]
                + point(
                    px(f32::from(delta.x) * start / length),
                    px(f32::from(delta.y) * start / length),
                );
            let b = segment[0]
                + point(
                    px(f32::from(delta.x) * end / length),
                    px(f32::from(delta.y) * end / length),
                );
            stroke(&[a, b], false, width, color, window);
            start += dash + gap;
        }
    }
}
fn stroke(points: &[Point<Pixels>], closed: bool, width: Pixels, color: Hsla, window: &mut Window) {
    let Some(first) = points.first() else { return };
    let mut path = PathBuilder::stroke(width);
    path.move_to(*first);
    for point in &points[1..] {
        path.line_to(*point);
    }
    if closed {
        path.close();
    }
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
fn fill_path(points: &[Point<Pixels>], color: Hsla, window: &mut Window) {
    let Some(first) = points.first() else { return };
    let mut path = PathBuilder::fill();
    path.move_to(*first);
    for point in &points[1..] {
        path.line_to(*point);
    }
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
