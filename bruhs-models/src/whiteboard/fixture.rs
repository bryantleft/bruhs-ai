use super::*;
use crate::num::ToFloatLossy;

/// Neutral scene consumed by all renderers. Bindings are not additional shape records.
pub fn benchmark_scene(count: usize) -> BoardResult<BoardDocument> {
    if count == 0 || !count.is_multiple_of(4) {
        return Err(BoardError::Invalid(
            "The neutral benchmark count must be positive and divisible by four.".into(),
        ));
    }
    let mut board = BoardDocument::empty(
        "00000000-0000-0000-0000-000000000001".into(),
        "/synthetic/whiteboard-benchmark".into(),
        format!("Neutral {count}-shape board"),
        0,
    );
    let nodes = count / 2;
    board.background = Color::rgb(249, 250, 251);
    let texts = count / 4;
    let columns = (nodes.to_f64_lossy().sqrt().ceil()).to_usize_saturating();
    let style = Style {
        stroke: Color::rgb(29, 29, 29),
        fill: Some(Color::rgb(252, 255, 254)),
        stroke_width: 2.0,
        font_size: 18.0,
        dashed: false,
    };
    for index in 0..nodes {
        let id = format!("node-{index:05}");
        let col = (index % columns).to_f64_lossy();
        let row = (index / columns).to_f64_lossy();
        let mut shape = Shape::node(
            id.clone(),
            ShapeKind::Rectangle,
            Rect::new(col * 180.0, row * 120.0, 120.0, 64.0),
            i64::try_from(index)
                .map_err(|_| BoardError::Invalid("The fixture count is too large.".into()))?,
            format!("N{index}"),
        );
        shape.style = style.clone();
        board.shapes.insert(id, shape);
    }
    for index in 0..texts {
        let id = format!("text-{index:05}");
        let target = &board.shapes[&format!("node-{index:05}")];
        let r = Rect::new(target.rect.x, target.rect.y + 76.0, 140.0, 24.0);
        let mut shape = Shape::node(
            id.clone(),
            ShapeKind::Text,
            r,
            i64::try_from(nodes + index)
                .map_err(|_| BoardError::Invalid("The fixture count is too large.".into()))?,
            format!("Label {index}"),
        );
        shape.style = style.clone();
        shape.style.fill = None;
        board.shapes.insert(id, shape);
    }
    for index in 0..texts {
        let id = format!("arrow-{index:05}");
        let mut shape = Shape::node(
            id.clone(),
            ShapeKind::Arrow {
                start: Endpoint::bound(format!("node-{index:05}"), Point::new(1.0, 0.5)),
                end: Endpoint::bound(
                    format!("node-{:05}", (index + 1) % nodes),
                    Point::new(0.0, 0.5),
                ),
            },
            Rect::default(),
            i64::try_from(nodes + texts + index)
                .map_err(|_| BoardError::Invalid("The fixture count is too large.".into()))?,
            String::new(),
        );
        shape.style = style.clone();
        shape.style.fill = None;
        board.shapes.insert(id, shape);
    }
    board.validate()?;
    Ok(board)
}
use crate::num::ToIntSaturating;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neutral_scene_has_exact_mix_and_bound_arrows_follow_rotated_targets() {
        for count in [1000, 10000] {
            let mut board = benchmark_scene(count).unwrap();
            assert_eq!(board.shapes.len(), count);
            assert_eq!(
                board
                    .shapes
                    .values()
                    .filter(|s| matches!(s.kind, ShapeKind::Rectangle))
                    .count(),
                count / 2
            );
            let ShapeKind::Arrow { start, .. } = &board.shapes["arrow-00000"].kind else {
                panic!()
            };
            let before = endpoint(&board, start);
            board.shapes.get_mut("node-00000").unwrap().rect.x += 25.0;
            let ShapeKind::Arrow { start, .. } = &board.shapes["arrow-00000"].kind else {
                panic!()
            };
            assert_eq!(
                endpoint(&board, start),
                before.offset(Point::new(25.0, 0.0))
            );
            board.shapes.get_mut("node-00000").unwrap().rotation = std::f64::consts::FRAC_PI_2;
            board.validate().unwrap();
        }
    }
    #[test]
    fn camera_zoom_preserves_pointer_anchor_and_inverse() {
        let mut camera = Camera::default();
        let p = Point::new(100.0, 200.0);
        let world = camera.world(p);
        camera.zoom_at(p, 2.0).unwrap();
        assert_eq!(camera.world(p), world);
        assert_eq!(camera.screen(world), p);
    }
}
