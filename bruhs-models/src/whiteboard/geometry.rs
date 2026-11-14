use super::{BoardDocument, BoardEditor, BoardError, BoardResult, Endpoint, Shape, ShapeKind};
use crate::num::ToFloatLossy as _;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
    pub fn offset(self, delta: Point) -> Self {
        Self::new(self.x + delta.x, self.y + delta.y)
    }
    pub fn distance(self, other: Point) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }
    pub fn rotate(self, center: Point, radians: f64) -> Self {
        let (s, c) = radians.sin_cos();
        let x = self.x - center.x;
        let y = self.y - center.y;
        Self::new(center.x + x * c - y * s, center.y + x * s + y * c)
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}
impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
    pub fn center(self) -> Point {
        Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    pub fn right(self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(self) -> f64 {
        self.y + self.h
    }
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.y >= self.y && p.x <= self.right() && p.y <= self.bottom()
    }
    pub fn intersects(self, other: Rect) -> bool {
        self.x <= other.right()
            && self.right() >= other.x
            && self.y <= other.bottom()
            && self.bottom() >= other.y
    }
    pub fn expanded(self, amount: f64) -> Self {
        Self::new(
            self.x - amount,
            self.y - amount,
            self.w + amount * 2.0,
            self.h + amount * 2.0,
        )
    }
    pub fn translated(self, delta: Point) -> Self {
        Self::new(self.x + delta.x, self.y + delta.y, self.w, self.h)
    }
    pub fn union(self, other: Rect) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Self::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }
    pub fn validate(self) -> BoardResult<()> {
        if ![self.x, self.y, self.w, self.h, self.right(), self.bottom()]
            .into_iter()
            .all(f64::is_finite)
            || self.w < 0.0
            || self.h < 0.0
        {
            Err(BoardError::Invalid(
                "Shape bounds must be finite with nonnegative dimensions.".into(),
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
        }
    }
}
impl Camera {
    pub fn world(self, screen: Point) -> Point {
        Point::new(screen.x / self.zoom + self.x, screen.y / self.zoom + self.y)
    }
    pub fn screen(self, world: Point) -> Point {
        Point::new(
            (world.x - self.x) * self.zoom,
            (world.y - self.y) * self.zoom,
        )
    }
    pub fn viewport(self, width: f64, height: f64) -> Rect {
        Rect::new(self.x, self.y, width / self.zoom, height / self.zoom)
    }
    pub fn zoom_at(&mut self, screen: Point, zoom: f64) -> BoardResult<()> {
        if !zoom.is_finite() || zoom <= 0.0 {
            return Err(BoardError::Invalid(
                "Zoom must be positive and finite.".into(),
            ));
        }
        let anchor = self.world(screen);
        let x = anchor.x - screen.x / zoom;
        let y = anchor.y - screen.y / zoom;
        if !screen.finite() || !x.is_finite() || !y.is_finite() {
            return Err(BoardError::Invalid(
                "The camera transform is outside the finite rendering range.".into(),
            ));
        }
        self.zoom = zoom;
        self.x = x;
        self.y = y;
        Ok(())
    }
}
pub fn endpoint(document: &BoardDocument, endpoint: &Endpoint) -> Point {
    endpoint
        .binding
        .as_ref()
        .and_then(|binding| {
            document.shapes.get(&binding.shape_id).map(|target| {
                Point::new(
                    target.rect.x + target.rect.w * binding.anchor.x + binding.offset.x,
                    target.rect.y + target.rect.h * binding.anchor.y + binding.offset.y,
                )
                .rotate(target.rect.center(), target.rotation)
            })
        })
        .unwrap_or(endpoint.point)
}
pub fn bounds(document: &BoardDocument, shape: &Shape) -> Rect {
    match &shape.kind {
        ShapeKind::Arrow { start, end } => {
            let a = endpoint(document, start);
            let b = endpoint(document, end);
            Rect::new(
                a.x.min(b.x),
                a.y.min(b.y),
                (a.x - b.x).abs(),
                (a.y - b.y).abs(),
            )
            .expanded(shape.style.stroke_width)
        }
        ShapeKind::Freehand { points } => points
            .iter()
            .map(|p| p.rotate(shape.rect.center(), shape.rotation))
            .map(|p| Rect::new(p.x, p.y, 0.0, 0.0))
            .reduce(Rect::union)
            .unwrap_or_default()
            .expanded(shape.style.stroke_width),
        _ => {
            let r = shape.rect;
            let center = r.center();
            point_bounds(
                &[
                    Point::new(r.x, r.y),
                    Point::new(r.right(), r.y),
                    Point::new(r.right(), r.bottom()),
                    Point::new(r.x, r.bottom()),
                ]
                .map(|p| p.rotate(center, shape.rotation)),
            )
        }
    }
}
pub fn point_bounds(points: &[Point]) -> Rect {
    let Some(first) = points.first() else {
        return Rect::default();
    };
    let mut x = first.x;
    let mut y = first.y;
    let mut right = x;
    let mut bottom = y;
    for p in &points[1..] {
        x = x.min(p.x);
        y = y.min(p.y);
        right = right.max(p.x);
        bottom = bottom.max(p.y);
    }
    Rect::new(x, y, right - x, bottom - y)
}
/// Bounds and endpoints during transient moves include affected bound arrows.
pub fn display_endpoint(editor: &super::BoardEditor, shape: &Shape, end: &Endpoint) -> Point {
    let point = endpoint(editor.document(), end);
    let Some(movement) = &editor.movement else {
        return point;
    };
    if movement.ids.contains(&shape.id)
        || end
            .binding
            .as_ref()
            .is_some_and(|b| movement.ids.contains(&b.shape_id))
    {
        point.offset(movement.delta)
    } else {
        point
    }
}
pub fn display_bounds(editor: &super::BoardEditor, shape: &Shape) -> Rect {
    if let ShapeKind::Arrow { start, end } = &shape.kind {
        let a = display_endpoint(editor, shape, start);
        let b = display_endpoint(editor, shape, end);
        point_bounds(&[a, b]).expanded(shape.style.stroke_width * 2.5)
    } else {
        let bounds = bounds(editor.document(), shape);
        editor
            .movement
            .as_ref()
            .filter(|m| m.ids.contains(&shape.id))
            .map_or(bounds, |m| bounds.translated(m.delta))
    }
}
pub fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let length = dx.hypot(dy);
    if length == 0.0 {
        return p.distance(a);
    }
    let along = ((p.x - a.x) * (dx / length) + (p.y - a.y) * (dy / length)).clamp(0.0, length);
    p.distance(Point::new(
        a.x + dx / length * along,
        a.y + dy / length * along,
    ))
}
pub fn hit(document: &BoardDocument, shape: &Shape, point: Point, tolerance: f64) -> bool {
    match &shape.kind {
        ShapeKind::Group => false,
        ShapeKind::Arrow { start, end } => {
            segment_distance(point, endpoint(document, start), endpoint(document, end))
                <= tolerance + shape.style.stroke_width / 2.0
        }
        ShapeKind::Freehand { points } => points.windows(2).any(|pair| {
            segment_distance(
                point.rotate(shape.rect.center(), -shape.rotation),
                pair[0],
                pair[1],
            ) <= tolerance + shape.style.stroke_width / 2.0
        }),
        ShapeKind::Ellipse => {
            let p = point.rotate(shape.rect.center(), -shape.rotation);
            let rx = shape.rect.w / 2.0 + tolerance;
            let ry = shape.rect.h / 2.0 + tolerance;
            rx > 0.0
                && ry > 0.0
                && ((p.x - shape.rect.center().x) / rx).powi(2)
                    + ((p.y - shape.rect.center().y) / ry).powi(2)
                    <= 1.0
        }
        ShapeKind::Frame => {
            let p = point.rotate(shape.rect.center(), -shape.rotation);
            shape.rect.expanded(tolerance).contains(p)
                && !shape.rect.expanded(-tolerance).contains(p)
        }
        _ => shape
            .rect
            .expanded(tolerance)
            .contains(point.rotate(shape.rect.center(), -shape.rotation)),
    }
}

pub fn text_anchor(document: &BoardDocument, shape: &Shape) -> Point {
    let block = (shape.text.lines().count().saturating_sub(1)).to_f64_lossy()
        * shape.style.font_size
        * 1.35
        / 2.0;
    match &shape.kind {
        ShapeKind::Arrow { start, end } => {
            let a = endpoint(document, start);
            let b = endpoint(document, end);
            Point::new(
                (a.x + b.x) / 2.0,
                (a.y + b.y) / 2.0 - shape.style.font_size - block,
            )
        }
        ShapeKind::Text | ShapeKind::Frame => {
            Point::new(shape.rect.x, shape.rect.y + shape.style.font_size / 2.0)
        }
        _ => Point::new(shape.rect.center().x, shape.rect.center().y - block),
    }
}
pub fn display_text_anchor(editor: &BoardEditor, shape: &Shape) -> Point {
    if let ShapeKind::Arrow { start, end } = &shape.kind {
        let a = display_endpoint(editor, shape, start);
        let b = display_endpoint(editor, shape, end);
        let block = (shape.text.lines().count().saturating_sub(1)).to_f64_lossy()
            * shape.style.font_size
            * 1.35
            / 2.0;
        Point::new(
            (a.x + b.x) / 2.0,
            (a.y + b.y) / 2.0 - shape.style.font_size - block,
        )
    } else {
        let delta = editor
            .movement
            .as_ref()
            .filter(|m| m.ids.contains(&shape.id))
            .map_or(Point::default(), |m| m.delta);
        text_anchor(editor.document(), shape)
            .rotate(shape.rect.center(), shape.rotation)
            .offset(delta)
    }
}
