//! Stage 2: Pure geometry layout engine (`Diagram` → positioned [`Scene`]).
//!
//! Computes Sugiyama-style layered graph coordinates, sequence lifelines, pie
//! geometry, and gantt bars, emitting renderer-agnostic [`Primitive`]s.

use std::collections::{HashMap, HashSet, VecDeque};
use std::f64::consts::PI;

use bruhs_models::num::ToFloatLossy as _;

use super::ir::{
    Diagram, Direction, EdgeStyle, GanttChart, Graph, GraphEdge, GraphNode, NodeShape, NoteSide,
    PieChart, Primitive, Scene, SeqItem, SequenceDiagram, Stroke, TextAnchor, Theme,
};

/// Lay out any [`Diagram`] into a positioned [`Scene`].
pub fn layout(diagram: &Diagram, theme: &Theme) -> Scene {
    match diagram {
        Diagram::Graph(graph) => layout_graph(graph, theme),
        Diagram::Sequence(seq) => layout_sequence(seq, theme),
        Diagram::Pie(pie) => layout_pie(pie, theme),
        Diagram::Gantt(gantt) => layout_gantt(gantt, theme),
    }
}

#[derive(Clone, Copy)]
struct RectBox {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl RectBox {
    fn cx(self) -> f32 {
        self.x + self.w * 0.5
    }
    fn cy(self) -> f32 {
        self.y + self.h * 0.5
    }
}

fn text_width(text: &str, font_size: f32) -> f32 {
    let units: f32 = text
        .chars()
        .map(|c| if c as u32 > 0x2E7F { 1.05 } else { 0.62 })
        .sum();
    units * font_size
}

fn solid_stroke(color: &str, width: f32) -> Stroke {
    Stroke {
        color: color.to_string(),
        width,
        dash: None,
    }
}

fn dashed_stroke(color: &str, width: f32, dash: &'static str) -> Stroke {
    Stroke {
        color: color.to_string(),
        width,
        dash: Some(dash),
    }
}

// ---------------------------------------------------------------------------
// Layered Graph Layout
// ---------------------------------------------------------------------------

fn layout_graph(graph: &Graph, theme: &Theme) -> Scene {
    let n = graph.nodes.len();
    let fs = theme.font_size;
    let line_h = fs * 1.4;

    let sizes: Vec<(f32, f32)> = graph
        .nodes
        .iter()
        .map(|node| measure_node(node, fs, line_h))
        .collect();

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in &graph.edges {
        if edge.from != edge.to {
            adj[edge.from].push(edge.to);
        }
    }
    let back_edges = find_back_edges(n, &adj);
    let mut dag_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indeg = vec![0usize; n];
    for edge in &graph.edges {
        if edge.from != edge.to && !back_edges.contains(&(edge.from, edge.to)) {
            dag_adj[edge.from].push(edge.to);
            indeg[edge.to] += 1;
        }
    }

    let mut rank = vec![0usize; n];
    let mut queue: VecDeque<usize> = (0..n).filter(|&u| indeg[u] == 0).collect();
    while let Some(u) = queue.pop_front() {
        for &v in &dag_adj[u] {
            if rank[v] < rank[u] + 1 {
                rank[v] = rank[u] + 1;
            }
            indeg[v] = indeg[v].saturating_sub(1);
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }

    let max_rank = rank.iter().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_rank + 1];
    for (idx, &r) in rank.iter().enumerate() {
        layers[r].push(idx);
    }

    for _ in 0..2 {
        for l in 1..layers.len() {
            let prev_pos: HashMap<usize, usize> = layers[l - 1]
                .iter()
                .enumerate()
                .map(|(p, &id)| (id, p))
                .collect();
            layers[l].sort_by(|&a, &b| {
                let ba = barycenter_in(a, &graph.edges, &prev_pos);
                let bb = barycenter_in(b, &graph.edges, &prev_pos);
                ba.total_cmp(&bb).then(a.cmp(&b))
            });
        }
    }

    if matches!(graph.dir, Direction::Bt | Direction::Rl) {
        layers.reverse();
    }

    let horizontal = matches!(graph.dir, Direction::Lr | Direction::Rl);
    let main_gap = 64.0_f32;
    let cross_gap = 36.0_f32;
    let margin = 28.0_f32;
    let sg_pad = if graph.subgraphs.is_empty() {
        0.0
    } else {
        20.0
    };

    let layer_cross_spans: Vec<f32> = layers
        .iter()
        .map(|layer| {
            let sum: f32 = layer
                .iter()
                .map(|&id| if horizontal { sizes[id].1 } else { sizes[id].0 })
                .sum();
            sum + cross_gap * layer.len().saturating_sub(1).to_f32_lossy()
        })
        .collect();
    let max_cross = layer_cross_spans.iter().copied().fold(0.0_f32, f32::max);

    let mut boxes = vec![
        RectBox {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        n
    ];

    let mut main_cursor = margin + sg_pad;
    for (li, layer) in layers.iter().enumerate() {
        let layer_depth = layer
            .iter()
            .map(|&id| if horizontal { sizes[id].0 } else { sizes[id].1 })
            .fold(0.0_f32, f32::max);
        let mut cross_cursor = margin + sg_pad + (max_cross - layer_cross_spans[li]) * 0.5;

        for &id in layer {
            let (w, h) = sizes[id];
            let (x, y) = if horizontal {
                (main_cursor + (layer_depth - w) * 0.5, cross_cursor)
            } else {
                (cross_cursor, main_cursor + (layer_depth - h) * 0.5)
            };
            boxes[id] = RectBox { x, y, w, h };
            cross_cursor += if horizontal { h } else { w } + cross_gap;
        }
        main_cursor += layer_depth + main_gap;
    }

    let mut max_x = boxes.iter().map(|b| b.x + b.w).fold(0.0_f32, f32::max) + margin + sg_pad;
    let mut max_y = boxes.iter().map(|b| b.y + b.h).fold(0.0_f32, f32::max) + margin + sg_pad;

    let mut primitives = Vec::new();

    // 1. Subgraph clusters (behind edges & nodes)
    for sg in &graph.subgraphs {
        if sg.nodes.is_empty() {
            continue;
        }
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut r_max_x = f32::MIN;
        let mut r_max_y = f32::MIN;
        for &id in &sg.nodes {
            let b = boxes[id];
            min_x = min_x.min(b.x);
            min_y = min_y.min(b.y);
            r_max_x = r_max_x.max(b.x + b.w);
            r_max_y = r_max_y.max(b.y + b.h);
        }
        let x = (min_x - 16.0).max(8.0);
        let y = (min_y - 30.0).max(8.0);
        let w = (r_max_x - min_x) + 32.0;
        let h = (r_max_y - min_y) + 46.0;
        max_x = max_x.max(x + w + 16.0);
        max_y = max_y.max(y + h + 16.0);

        primitives.push(Primitive::Rect {
            x,
            y,
            w,
            h,
            rx: 6.0,
            fill: Some(theme.cluster_background.clone()),
            stroke: Some(dashed_stroke(&theme.cluster_border, 1.2, "4 3")),
        });
        primitives.push(Primitive::Text {
            x: x + 10.0,
            y: y + 18.0,
            font_size: fs * 0.92,
            bold: true,
            anchor: TextAnchor::Start,
            color: theme.text_color.clone(),
            content: sg.title.clone(),
        });
    }

    // 2. Edges
    for edge in &graph.edges {
        push_edge_primitives(
            &mut primitives,
            boxes[edge.from],
            boxes[edge.to],
            edge,
            horizontal,
            theme,
        );
    }

    // 3. Nodes
    for (idx, node) in graph.nodes.iter().enumerate() {
        push_node_primitives(&mut primitives, node, boxes[idx], fs, line_h, theme);
    }

    Scene {
        width: max_x.max(120.0),
        height: max_y.max(80.0),
        background: theme.background.clone(),
        primitives,
    }
}

fn find_back_edges(n: usize, adj: &[Vec<usize>]) -> HashSet<(usize, usize)> {
    let mut state = vec![0u8; n];
    let mut back = HashSet::new();
    fn dfs(u: usize, adj: &[Vec<usize>], state: &mut [u8], back: &mut HashSet<(usize, usize)>) {
        state[u] = 1;
        for &v in &adj[u] {
            if state[v] == 1 {
                back.insert((u, v));
            } else if state[v] == 0 {
                dfs(v, adj, state, back);
            }
        }
        state[u] = 2;
    }
    for u in 0..n {
        if state[u] == 0 {
            dfs(u, adj, &mut state, &mut back);
        }
    }
    back
}

fn barycenter_in(node: usize, edges: &[GraphEdge], prev_pos: &HashMap<usize, usize>) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0usize;
    for e in edges {
        if e.to == node {
            if let Some(&p) = prev_pos.get(&e.from) {
                sum += p.to_f32_lossy();
                count += 1;
            }
        }
    }
    if count == 0 {
        f32::MAX
    } else {
        sum / count.to_f32_lossy()
    }
}

fn measure_node(node: &GraphNode, fs: f32, line_h: f32) -> (f32, f32) {
    if matches!(node.shape, NodeShape::StartDot | NodeShape::EndBullseye) {
        return (24.0, 24.0);
    }
    let mut max_w = node
        .label
        .lines()
        .map(|l| text_width(l, fs))
        .fold(0.0_f32, f32::max);
    let mut total_lines = node.label.lines().count().max(1);
    for comp in &node.compartments {
        for line in comp {
            max_w = max_w.max(text_width(line, fs * 0.92));
        }
        total_lines += comp.len();
    }
    let comp_gaps = node.compartments.len().to_f32_lossy() * 10.0;
    let pad_x = match node.shape {
        NodeShape::Diamond => 36.0,
        NodeShape::Hexagon | NodeShape::Parallelogram => 34.0,
        NodeShape::Circle | NodeShape::DoubleCircle => 26.0,
        _ => 28.0,
    };
    let pad_y = match node.shape {
        NodeShape::Diamond | NodeShape::Cylinder => 28.0,
        _ => 18.0,
    };
    let w = (max_w + pad_x).max(64.0);
    let h = (total_lines.to_f32_lossy() * line_h + pad_y + comp_gaps).max(36.0);
    if matches!(node.shape, NodeShape::Circle | NodeShape::DoubleCircle) {
        let d = w.max(h);
        (d, d)
    } else {
        (w, h)
    }
}

fn push_edge_primitives(
    out: &mut Vec<Primitive>,
    from: RectBox,
    to: RectBox,
    edge: &GraphEdge,
    horizontal: bool,
    theme: &Theme,
) {
    let width = match edge.style {
        EdgeStyle::Thick => 2.5,
        _ => 1.5,
    };
    let stroke = match edge.style {
        EdgeStyle::Dotted => dashed_stroke(&theme.line_color, width, "5 4"),
        _ => solid_stroke(&theme.line_color, width),
    };

    if (from.cx() - to.cx()).abs() < 0.5 && (from.cy() - to.cy()).abs() < 0.5 {
        let sx = from.x + from.w;
        let sy = from.cy() - 6.0;
        let ex = from.x + from.w;
        let ey = from.cy() + 6.0;
        out.push(Primitive::Path {
            d: format!(
                "M {sx:.1} {sy:.1} C {:.1} {:.1} {:.1} {:.1} {ex:.1} {ey:.1}",
                sx + 36.0,
                sy - 24.0,
                ex + 36.0,
                ey + 24.0
            ),
            fill: None,
            stroke: Some(stroke),
        });
        if edge.arrow_end {
            push_arrowhead(out, ex + 12.0, ey + 6.0, ex, ey, &theme.line_color);
        }
        if !edge.label.is_empty() {
            push_edge_label(out, sx + 38.0, from.cy(), &edge.label, theme);
        }
        return;
    }

    let (sx, sy, ex, ey, c1x, c1y, c2x, c2y) = if horizontal {
        if to.cx() >= from.cx() {
            let (sx, sy, ex, ey) = (from.x + from.w, from.cy(), to.x, to.cy());
            let dx = ((ex - sx).abs() * 0.45).max(24.0);
            (sx, sy, ex, ey, sx + dx, sy, ex - dx, ey)
        } else {
            let (sx, sy, ex, ey) = (from.x, from.cy(), to.x + to.w, to.cy());
            let dx = ((ex - sx).abs() * 0.45).max(24.0);
            (sx, sy, ex, ey, sx - dx, sy, ex + dx, ey)
        }
    } else if to.cy() >= from.cy() {
        let (sx, sy, ex, ey) = (from.cx(), from.y + from.h, to.cx(), to.y);
        let dy = ((ey - sy).abs() * 0.45).max(24.0);
        (sx, sy, ex, ey, sx, sy + dy, ex, ey - dy)
    } else {
        let (sx, sy, ex, ey) = (from.cx(), from.y, to.cx(), to.y + to.h);
        let dy = ((ey - sy).abs() * 0.45).max(24.0);
        (sx, sy, ex, ey, sx, sy - dy, ex, ey + dy)
    };

    out.push(Primitive::Path {
        d: format!("M {sx:.1} {sy:.1} C {c1x:.1} {c1y:.1} {c2x:.1} {c2y:.1} {ex:.1} {ey:.1}"),
        fill: None,
        stroke: Some(stroke),
    });
    if edge.arrow_end {
        push_arrowhead(out, c2x, c2y, ex, ey, &theme.line_color);
    }
    if edge.arrow_start {
        push_arrowhead(out, c1x, c1y, sx, sy, &theme.line_color);
    }
    if !edge.label.is_empty() {
        let mx = 0.125 * sx + 0.375 * c1x + 0.375 * c2x + 0.125 * ex;
        let my = 0.125 * sy + 0.375 * c1y + 0.375 * c2y + 0.125 * ey;
        push_edge_label(out, mx, my, &edge.label, theme);
    }
}

fn push_arrowhead(
    out: &mut Vec<Primitive>,
    from_x: f32,
    from_y: f32,
    tip_x: f32,
    tip_y: f32,
    color: &str,
) {
    let dx = tip_x - from_x;
    let dy = tip_y - from_y;
    let len = dx.hypot(dy);
    if len < 1e-4 {
        return;
    }
    let ux = dx / len;
    let uy = dy / len;
    let bx = tip_x - ux * 8.0;
    let by = tip_y - uy * 8.0;
    let px = -uy * 4.2;
    let py = ux * 4.2;
    out.push(Primitive::Polygon {
        points: vec![(tip_x, tip_y), (bx + px, by + py), (bx - px, by - py)],
        fill: Some(color.to_string()),
        stroke: None,
    });
}

fn push_edge_label(out: &mut Vec<Primitive>, cx: f32, cy: f32, label: &str, theme: &Theme) {
    let fs = theme.font_size * 0.88;
    let w = text_width(label, fs) + 12.0;
    let h = fs + 8.0;
    out.push(Primitive::Rect {
        x: cx - w * 0.5,
        y: cy - h * 0.5,
        w,
        h,
        rx: 4.0,
        fill: Some(theme.edge_label_background.clone()),
        stroke: Some(solid_stroke(&theme.primary_border_color, 0.8)),
    });
    out.push(Primitive::Text {
        x: cx,
        y: cy + fs * 0.35,
        font_size: fs,
        bold: false,
        anchor: TextAnchor::Middle,
        color: theme.text_color.clone(),
        content: label.to_string(),
    });
}

fn push_node_primitives(
    out: &mut Vec<Primitive>,
    node: &GraphNode,
    b: RectBox,
    fs: f32,
    line_h: f32,
    theme: &Theme,
) {
    let fill = node.fill.as_deref().unwrap_or(&theme.primary_color);
    let stroke_col = node
        .stroke
        .as_deref()
        .unwrap_or(&theme.primary_border_color);
    let text_col = node.color.as_deref().unwrap_or(&theme.primary_text_color);

    match node.shape {
        NodeShape::StartDot => {
            out.push(Primitive::Circle {
                cx: b.cx(),
                cy: b.cy(),
                r: 8.0,
                fill: Some(theme.line_color.clone()),
                stroke: None,
            });
            return;
        }
        NodeShape::EndBullseye => {
            out.push(Primitive::Circle {
                cx: b.cx(),
                cy: b.cy(),
                r: 10.0,
                fill: None,
                stroke: Some(solid_stroke(&theme.line_color, 1.8)),
            });
            out.push(Primitive::Circle {
                cx: b.cx(),
                cy: b.cy(),
                r: 5.5,
                fill: Some(theme.line_color.clone()),
                stroke: None,
            });
            return;
        }
        NodeShape::Rect => out.push(Primitive::Rect {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
            rx: 4.0,
            fill: Some(fill.to_string()),
            stroke: Some(solid_stroke(stroke_col, 1.4)),
        }),
        NodeShape::Rounded => out.push(Primitive::Rect {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
            rx: 12.0,
            fill: Some(fill.to_string()),
            stroke: Some(solid_stroke(stroke_col, 1.4)),
        }),
        NodeShape::Stadium => out.push(Primitive::Rect {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
            rx: b.h * 0.5,
            fill: Some(fill.to_string()),
            stroke: Some(solid_stroke(stroke_col, 1.4)),
        }),
        NodeShape::Diamond => out.push(Primitive::Polygon {
            points: vec![
                (b.cx(), b.y),
                (b.x + b.w, b.cy()),
                (b.cx(), b.y + b.h),
                (b.x, b.cy()),
            ],
            fill: Some(fill.to_string()),
            stroke: Some(solid_stroke(stroke_col, 1.4)),
        }),
        NodeShape::Circle => out.push(Primitive::Circle {
            cx: b.cx(),
            cy: b.cy(),
            r: b.w * 0.5,
            fill: Some(fill.to_string()),
            stroke: Some(solid_stroke(stroke_col, 1.4)),
        }),
        NodeShape::DoubleCircle => {
            out.push(Primitive::Circle {
                cx: b.cx(),
                cy: b.cy(),
                r: b.w * 0.5,
                fill: Some(fill.to_string()),
                stroke: Some(solid_stroke(stroke_col, 1.4)),
            });
            out.push(Primitive::Circle {
                cx: b.cx(),
                cy: b.cy(),
                r: (b.w * 0.5 - 4.0).max(2.0),
                fill: None,
                stroke: Some(solid_stroke(stroke_col, 1.2)),
            });
        }
        NodeShape::Hexagon => {
            let dx = 12.0_f32.min(b.w * 0.2);
            out.push(Primitive::Polygon {
                points: vec![
                    (b.x + dx, b.y),
                    (b.x + b.w - dx, b.y),
                    (b.x + b.w, b.cy()),
                    (b.x + b.w - dx, b.y + b.h),
                    (b.x + dx, b.y + b.h),
                    (b.x, b.cy()),
                ],
                fill: Some(fill.to_string()),
                stroke: Some(solid_stroke(stroke_col, 1.4)),
            });
        }
        NodeShape::Parallelogram => {
            let skew = 10.0_f32.min(b.w * 0.2);
            out.push(Primitive::Polygon {
                points: vec![
                    (b.x + skew, b.y),
                    (b.x + b.w, b.y),
                    (b.x + b.w - skew, b.y + b.h),
                    (b.x, b.y + b.h),
                ],
                fill: Some(fill.to_string()),
                stroke: Some(solid_stroke(stroke_col, 1.4)),
            });
        }
        NodeShape::Subroutine => {
            out.push(Primitive::Rect {
                x: b.x,
                y: b.y,
                w: b.w,
                h: b.h,
                rx: 3.0,
                fill: Some(fill.to_string()),
                stroke: Some(solid_stroke(stroke_col, 1.4)),
            });
            out.push(Primitive::Line {
                x1: b.x + 8.0,
                y1: b.y,
                x2: b.x + 8.0,
                y2: b.y + b.h,
                stroke: solid_stroke(stroke_col, 1.2),
            });
            out.push(Primitive::Line {
                x1: b.x + b.w - 8.0,
                y1: b.y,
                x2: b.x + b.w - 8.0,
                y2: b.y + b.h,
                stroke: solid_stroke(stroke_col, 1.2),
            });
        }
        NodeShape::Cylinder => {
            let ry = 7.0_f32;
            out.push(Primitive::Path {
                d: format!(
                    "M {:.1} {:.1} L {:.1} {:.1} A {:.1} {ry:.1} 0 0 0 {:.1} {:.1} L {:.1} {:.1} Z",
                    b.x,
                    b.y + ry,
                    b.x,
                    b.y + b.h - ry,
                    b.w * 0.5,
                    b.x + b.w,
                    b.y + b.h - ry,
                    b.x + b.w,
                    b.y + ry
                ),
                fill: Some(fill.to_string()),
                stroke: Some(solid_stroke(stroke_col, 1.4)),
            });
            out.push(Primitive::Ellipse {
                cx: b.cx(),
                cy: b.y + ry,
                rx: b.w * 0.5,
                ry,
                fill: Some(fill.to_string()),
                stroke: Some(solid_stroke(stroke_col, 1.4)),
            });
        }
    }

    if node.compartments.is_empty() {
        let lines: Vec<&str> = node.label.lines().collect();
        let total_h = lines.len().to_f32_lossy() * line_h;
        let start_y = b.cy() - total_h * 0.5 + fs * 0.85;
        for (i, line) in lines.iter().enumerate() {
            out.push(Primitive::Text {
                x: b.cx(),
                y: start_y + i.to_f32_lossy() * line_h,
                font_size: fs,
                bold: false,
                anchor: TextAnchor::Middle,
                color: text_col.to_string(),
                content: (*line).to_string(),
            });
        }
    } else {
        let mut y = b.y + 8.0;
        for line in node.label.lines() {
            y += fs;
            out.push(Primitive::Text {
                x: b.cx(),
                y,
                font_size: fs,
                bold: true,
                anchor: TextAnchor::Middle,
                color: text_col.to_string(),
                content: line.to_string(),
            });
            y += line_h - fs;
        }
        for comp in &node.compartments {
            y += 5.0;
            out.push(Primitive::Line {
                x1: b.x,
                y1: y,
                x2: b.x + b.w,
                y2: y,
                stroke: solid_stroke(stroke_col, 1.0),
            });
            y += 5.0;
            for line in comp {
                y += fs * 0.92;
                out.push(Primitive::Text {
                    x: b.x + 10.0,
                    y,
                    font_size: fs * 0.92,
                    bold: false,
                    anchor: TextAnchor::Start,
                    color: text_col.to_string(),
                    content: line.clone(),
                });
                y += line_h - fs * 0.92;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sequence Layout
// ---------------------------------------------------------------------------

fn layout_sequence(seq: &SequenceDiagram, theme: &Theme) -> Scene {
    let fs = theme.font_size;
    let margin = 28.0_f32;
    let actor_h = 36.0_f32;
    let actor_widths: Vec<f32> = seq
        .participants
        .iter()
        .map(|p| (text_width(&p.label, fs) + 28.0).max(96.0))
        .collect();
    let col_gap = 64.0_f32;

    let mut col_x = Vec::with_capacity(seq.participants.len());
    let mut cx_cursor = margin;
    for &w in &actor_widths {
        col_x.push(cx_cursor + w * 0.5);
        cx_cursor += w + col_gap;
    }

    let row_step = 44.0_f32;
    let top_y = margin + actor_h + 24.0;
    let mut cur_y = top_y;
    let mut row_ys = Vec::with_capacity(seq.items.len() + 1);
    for item in &seq.items {
        row_ys.push(cur_y);
        cur_y += match item {
            SeqItem::Message { from, to, .. } if from == to => row_step + 14.0,
            SeqItem::BlockStart { .. } | SeqItem::BlockDivider { .. } => 28.0,
            SeqItem::BlockEnd => 16.0,
            _ => row_step,
        };
    }
    row_ys.push(cur_y);

    let bottom_actor_y = cur_y + 12.0;
    let width = (cx_cursor - col_gap + margin).max(240.0);
    let height = bottom_actor_y + actor_h + margin;

    let mut primitives = Vec::new();

    for &x in &col_x {
        primitives.push(Primitive::Line {
            x1: x,
            y1: margin + actor_h,
            x2: x,
            y2: bottom_actor_y,
            stroke: dashed_stroke(&theme.sequence_actor_line, 1.2, "5 4"),
        });
    }

    for &(col, start_row, end_row) in &seq.activations {
        let x = col_x[col] - 5.0;
        let y1 = row_ys.get(start_row).copied().unwrap_or(top_y);
        let y2 = row_ys.get(end_row).copied().unwrap_or(cur_y).max(y1 + 20.0);
        primitives.push(Primitive::Rect {
            x,
            y: y1,
            w: 10.0,
            h: y2 - y1,
            rx: 2.0,
            fill: Some(theme.sequence_activation_fill.clone()),
            stroke: Some(solid_stroke(&theme.sequence_activation_border, 1.2)),
        });
    }

    let mut block_stack: Vec<(f32, String, String)> = Vec::new();
    for (idx, item) in seq.items.iter().enumerate() {
        let y = row_ys[idx];
        match item {
            SeqItem::BlockStart { kind, label } => {
                block_stack.push((y - 12.0, kind.clone(), label.clone()));
            }
            SeqItem::BlockDivider { label } => {
                let x1 = margin * 0.6;
                let x2 = width - margin * 0.6;
                primitives.push(Primitive::Line {
                    x1,
                    y1: y,
                    x2,
                    y2: y,
                    stroke: dashed_stroke(&theme.cluster_border, 1.0, "4 3"),
                });
                if !label.is_empty() {
                    primitives.push(Primitive::Text {
                        x: x1 + 8.0,
                        y: y + 14.0,
                        font_size: fs * 0.88,
                        bold: false,
                        anchor: TextAnchor::Start,
                        color: theme.text_color.clone(),
                        content: format!("[{label}]"),
                    });
                }
            }
            SeqItem::BlockEnd => {
                if let Some((start_y, kind, label)) = block_stack.pop() {
                    let x1 = margin * 0.6;
                    let w = width - margin * 1.2;
                    let h = (y - start_y + 6.0).max(28.0);
                    primitives.push(Primitive::Rect {
                        x: x1,
                        y: start_y,
                        w,
                        h,
                        rx: 4.0,
                        fill: None,
                        stroke: Some(solid_stroke(&theme.cluster_border, 1.2)),
                    });
                    let tag_w = text_width(&kind, fs * 0.85) + 16.0;
                    primitives.push(Primitive::Rect {
                        x: x1,
                        y: start_y,
                        w: tag_w,
                        h: 18.0,
                        rx: 3.0,
                        fill: Some(theme.secondary_color.clone()),
                        stroke: Some(solid_stroke(&theme.cluster_border, 1.0)),
                    });
                    primitives.push(Primitive::Text {
                        x: x1 + 8.0,
                        y: start_y + 13.0,
                        font_size: fs * 0.85,
                        bold: true,
                        anchor: TextAnchor::Start,
                        color: theme.text_color.clone(),
                        content: kind,
                    });
                    if !label.is_empty() {
                        primitives.push(Primitive::Text {
                            x: x1 + tag_w + 8.0,
                            y: start_y + 13.0,
                            font_size: fs * 0.85,
                            bold: false,
                            anchor: TextAnchor::Start,
                            color: theme.text_color.clone(),
                            content: format!("[{label}]"),
                        });
                    }
                }
            }
            SeqItem::Message {
                from,
                to,
                label,
                dotted,
                cross,
            } => {
                let x1 = col_x[*from];
                let x2 = col_x[*to];
                let stroke = if *dotted {
                    dashed_stroke(&theme.line_color, 1.5, "5 4")
                } else {
                    solid_stroke(&theme.line_color, 1.5)
                };
                if from == to {
                    primitives.push(Primitive::Path {
                        d: format!(
                            "M {x1:.1} {y:.1} L {:.1} {y:.1} L {:.1} {:.1} L {x1:.1} {:.1}",
                            x1 + 36.0,
                            x1 + 36.0,
                            y + 22.0,
                            y + 22.0
                        ),
                        fill: None,
                        stroke: Some(stroke),
                    });
                    push_arrowhead(
                        &mut primitives,
                        x1 + 36.0,
                        y + 22.0,
                        x1,
                        y + 22.0,
                        &theme.line_color,
                    );
                    primitives.push(Primitive::Text {
                        x: x1 + 42.0,
                        y: y + 14.0,
                        font_size: fs,
                        bold: false,
                        anchor: TextAnchor::Start,
                        color: theme.text_color.clone(),
                        content: label.clone(),
                    });
                } else {
                    primitives.push(Primitive::Line {
                        x1,
                        y1: y,
                        x2,
                        y2: y,
                        stroke,
                    });
                    if *cross {
                        primitives.push(Primitive::Path {
                            d: format!(
                                "M {:.1} {:.1} L {:.1} {:.1} M {:.1} {:.1} L {:.1} {:.1}",
                                x2 - 5.0,
                                y - 5.0,
                                x2 + 5.0,
                                y + 5.0,
                                x2 - 5.0,
                                y + 5.0,
                                x2 + 5.0,
                                y - 5.0
                            ),
                            fill: None,
                            stroke: Some(solid_stroke(&theme.line_color, 1.6)),
                        });
                    } else {
                        push_arrowhead(&mut primitives, x1, y, x2, y, &theme.line_color);
                    }
                    primitives.push(Primitive::Text {
                        x: (x1 + x2) * 0.5,
                        y: y - 6.0,
                        font_size: fs,
                        bold: false,
                        anchor: TextAnchor::Middle,
                        color: theme.text_color.clone(),
                        content: label.clone(),
                    });
                }
            }
            SeqItem::Note {
                text,
                start_col,
                end_col,
                side,
            } => {
                let nw = (text_width(text, fs * 0.9) + 20.0).max(90.0);
                let nh = 26.0_f32;
                let nx = match side {
                    NoteSide::Left => (col_x[*start_col] - nw - 12.0).max(8.0),
                    NoteSide::Right => col_x[*end_col] + 12.0,
                    NoteSide::Over => (col_x[*start_col] + col_x[*end_col]) * 0.5 - nw * 0.5,
                };
                primitives.push(Primitive::Rect {
                    x: nx,
                    y: y - nh * 0.5,
                    w: nw,
                    h: nh,
                    rx: 4.0,
                    fill: Some(theme.sequence_note_fill.clone()),
                    stroke: Some(solid_stroke(&theme.sequence_note_border, 1.2)),
                });
                primitives.push(Primitive::Text {
                    x: nx + nw * 0.5,
                    y: y + fs * 0.32,
                    font_size: fs * 0.9,
                    bold: false,
                    anchor: TextAnchor::Middle,
                    color: theme.primary_text_color.clone(),
                    content: text.clone(),
                });
            }
        }
    }

    for y_box in [margin, bottom_actor_y] {
        for (i, p) in seq.participants.iter().enumerate() {
            let w = actor_widths[i];
            let x = col_x[i] - w * 0.5;
            primitives.push(Primitive::Rect {
                x,
                y: y_box,
                w,
                h: actor_h,
                rx: 5.0,
                fill: Some(theme.sequence_actor_fill.clone()),
                stroke: Some(solid_stroke(&theme.sequence_actor_border, 1.4)),
            });
            primitives.push(Primitive::Text {
                x: col_x[i],
                y: y_box + actor_h * 0.5 + fs * 0.35,
                font_size: fs,
                bold: false,
                anchor: TextAnchor::Middle,
                color: theme.primary_text_color.clone(),
                content: p.label.clone(),
            });
        }
    }

    Scene {
        width,
        height,
        background: theme.background.clone(),
        primitives,
    }
}

// ---------------------------------------------------------------------------
// Pie & Gantt Layouts
// ---------------------------------------------------------------------------

fn layout_pie(pie: &PieChart, theme: &Theme) -> Scene {
    let palette = [
        "#e07a3f", "#5b9bd5", "#70b37e", "#c984c7", "#d9b44a", "#e06c75", "#56b6c2", "#a58de8",
    ];
    let total: f64 = pie.slices.iter().map(|(_, v)| *v).sum();
    let r = 84.0_f64;
    let cx = 120.0_f64;
    let cy = if pie.title.is_empty() { 114.0 } else { 134.0 };
    let legend_x = cx + r + 36.0;
    let fs = theme.font_size;
    let max_label_w = pie
        .slices
        .iter()
        .map(|(l, v)| {
            let pct = (*v / total) * 100.0;
            text_width(&format!("{l} ({pct:.0}%)"), fs)
        })
        .fold(80.0_f32, f32::max);
    let width = legend_x.to_f32_lossy() + max_label_w + 40.0;
    let height = (cy + r + 32.0)
        .to_f32_lossy()
        .max(54.0 + pie.slices.len().to_f32_lossy() * 24.0);

    let mut primitives = Vec::new();
    if !pie.title.is_empty() {
        primitives.push(Primitive::Text {
            x: width * 0.5,
            y: 26.0,
            font_size: fs * 1.15,
            bold: true,
            anchor: TextAnchor::Middle,
            color: theme.text_color.clone(),
            content: pie.title.clone(),
        });
    }

    let mut angle = -PI * 0.5;
    for (idx, (label, val)) in pie.slices.iter().enumerate() {
        let color = palette[idx % palette.len()];
        let frac = *val / total;
        let sweep = frac * 2.0 * PI;
        if pie.slices.len() == 1 {
            primitives.push(Primitive::Circle {
                cx: cx.to_f32_lossy(),
                cy: cy.to_f32_lossy(),
                r: r.to_f32_lossy(),
                fill: Some(color.to_string()),
                stroke: Some(solid_stroke(&theme.background, 1.5)),
            });
        } else {
            let x1 = cx + r * angle.cos();
            let y1 = cy + r * angle.sin();
            let x2 = cx + r * (angle + sweep).cos();
            let y2 = cy + r * (angle + sweep).sin();
            let large = usize::from(sweep > PI);
            primitives.push(Primitive::Path {
                d: format!(
                    "M {cx:.1} {cy:.1} L {x1:.1} {y1:.1} A {r:.1} {r:.1} 0 {large} 1 {x2:.1} {y2:.1} Z"
                ),
                fill: Some(color.to_string()),
                stroke: Some(solid_stroke(&theme.background, 1.5)),
            });
        }
        angle += sweep;

        let ly = (cy - r + 14.0).to_f32_lossy() + idx.to_f32_lossy() * 24.0;
        primitives.push(Primitive::Rect {
            x: legend_x.to_f32_lossy(),
            y: ly - 10.0,
            w: 12.0,
            h: 12.0,
            rx: 2.0,
            fill: Some(color.to_string()),
            stroke: None,
        });
        let pct = frac * 100.0;
        primitives.push(Primitive::Text {
            x: legend_x.to_f32_lossy() + 18.0,
            y: ly,
            font_size: fs,
            bold: false,
            anchor: TextAnchor::Start,
            color: theme.text_color.clone(),
            content: format!("{label} ({pct:.0}%)"),
        });
    }

    Scene {
        width,
        height,
        background: theme.background.clone(),
        primitives,
    }
}

fn layout_gantt(gantt: &GanttChart, theme: &Theme) -> Scene {
    let fs = theme.font_size;
    let label_w = gantt
        .tasks
        .iter()
        .map(|l| text_width(l, fs))
        .fold(120.0_f32, f32::max)
        + 24.0;
    let chart_w = 280.0_f32;
    let width = label_w + chart_w + 32.0;
    let top = if gantt.title.is_empty() { 20.0 } else { 44.0 };
    let row_h = 28.0_f32;
    let height = top + gantt.tasks.len().to_f32_lossy() * row_h + 20.0;

    let mut primitives = Vec::new();
    if !gantt.title.is_empty() {
        primitives.push(Primitive::Text {
            x: width * 0.5,
            y: 26.0,
            font_size: fs * 1.15,
            bold: true,
            anchor: TextAnchor::Middle,
            color: theme.text_color.clone(),
            content: gantt.title.clone(),
        });
    }
    let step = chart_w / gantt.tasks.len().max(1).to_f32_lossy();
    for (i, label) in gantt.tasks.iter().enumerate() {
        let y = top + i.to_f32_lossy() * row_h;
        primitives.push(Primitive::Text {
            x: 16.0,
            y: y + 17.0,
            font_size: fs,
            bold: false,
            anchor: TextAnchor::Start,
            color: theme.text_color.clone(),
            content: label.clone(),
        });
        let bx = label_w + i.to_f32_lossy() * step * 0.65;
        let bw = (step * 1.2).min(width - 16.0 - bx).max(36.0);
        primitives.push(Primitive::Rect {
            x: bx,
            y: y + 4.0,
            w: bw,
            h: 18.0,
            rx: 4.0,
            fill: Some(theme.primary_color.clone()),
            stroke: Some(solid_stroke(&theme.primary_border_color, 1.2)),
        });
    }

    Scene {
        width,
        height,
        background: theme.background.clone(),
        primitives,
    }
}
