//! Format-agnostic Diagram IR, Theme, and positioned vector Scene primitives.
//!
//! This layer has zero knowledge of Mermaid syntax and zero knowledge of SVG
//! serialization, so any parser (Mermaid, DOT, D2, structured JSON) and any
//! backend (SVG, Whiteboard import, direct GPUI painting) can share it.

use std::collections::HashMap;

/// Semantic palette and typography used during layout and rendering.
#[derive(Clone, Debug)]
pub struct Theme {
    pub background: String,
    pub primary_color: String,
    pub primary_text_color: String,
    pub primary_border_color: String,
    pub line_color: String,
    pub secondary_color: String,
    pub edge_label_background: String,
    pub cluster_background: String,
    pub cluster_border: String,
    pub sequence_actor_fill: String,
    pub sequence_actor_border: String,
    pub sequence_actor_line: String,
    pub sequence_note_fill: String,
    pub sequence_note_border: String,
    pub sequence_activation_fill: String,
    pub sequence_activation_border: String,
    pub text_color: String,
    pub font_family: String,
    pub font_size: f32,
}

impl Theme {
    /// Default dark theme matching the Bruhs `longan-950` canvas palette.
    #[must_use]
    pub fn bruhs_dark() -> Self {
        Self {
            background: "#141210".to_string(),
            primary_color: "#2f2922".to_string(),
            primary_text_color: "#dfcfc3".to_string(),
            primary_border_color: "#594a36".to_string(),
            line_color: "#c3afa1".to_string(),
            secondary_color: "#1f1a16".to_string(),
            edge_label_background: "#141210".to_string(),
            cluster_background: "#1f1a16".to_string(),
            cluster_border: "#594a36".to_string(),
            sequence_actor_fill: "#2f2922".to_string(),
            sequence_actor_border: "#594a36".to_string(),
            sequence_actor_line: "#c3afa1".to_string(),
            sequence_note_fill: "#2f2922".to_string(),
            sequence_note_border: "#594a36".to_string(),
            sequence_activation_fill: "#1f1a16".to_string(),
            sequence_activation_border: "#594a36".to_string(),
            text_color: "#dfcfc3".to_string(),
            font_family: "MonaspaceNeonVariable, Monaspace Neon, ui-monospace, monospace"
                .to_string(),
            font_size: 12.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Stage 1 Output: Diagram Intermediate Representation (IR)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Diagram {
    Graph(Graph),
    Sequence(SequenceDiagram),
    Pie(PieChart),
    Gantt(GanttChart),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Td,
    Bt,
    Lr,
    Rl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeShape {
    Rect,
    Rounded,
    Stadium,
    Diamond,
    Circle,
    DoubleCircle,
    Cylinder,
    Hexagon,
    Subroutine,
    Parallelogram,
    StartDot,
    EndBullseye,
}

#[derive(Clone, Debug)]
pub struct GraphNode {
    pub label: String,
    pub shape: NodeShape,
    pub subgraph: Option<usize>,
    pub fill: Option<String>,
    pub stroke: Option<String>,
    pub color: Option<String>,
    /// Optional stacked compartments (class fields/methods, ER attributes).
    pub compartments: Vec<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeStyle {
    Solid,
    Dotted,
    Thick,
}

#[derive(Clone, Debug)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub style: EdgeStyle,
    pub arrow_end: bool,
    pub arrow_start: bool,
}

#[derive(Clone, Debug)]
pub struct Subgraph {
    pub title: String,
    pub nodes: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct Graph {
    pub dir: Direction,
    pub nodes: Vec<GraphNode>,
    index: HashMap<String, usize>,
    pub edges: Vec<GraphEdge>,
    pub subgraphs: Vec<Subgraph>,
}

impl Graph {
    pub fn new(dir: Direction) -> Self {
        Self {
            dir,
            nodes: Vec::new(),
            index: HashMap::new(),
            edges: Vec::new(),
            subgraphs: Vec::new(),
        }
    }

    pub fn upsert_node(
        &mut self,
        id: &str,
        label: Option<String>,
        shape: Option<NodeShape>,
        subgraph: Option<usize>,
    ) -> usize {
        if let Some(&idx) = self.index.get(id) {
            let node = &mut self.nodes[idx];
            if let Some(l) = label {
                node.label = l;
            }
            if let Some(s) = shape {
                node.shape = s;
            }
            if node.subgraph.is_none() {
                if let Some(sg) = subgraph {
                    node.subgraph = Some(sg);
                    if !self.subgraphs[sg].nodes.contains(&idx) {
                        self.subgraphs[sg].nodes.push(idx);
                    }
                }
            }
            idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(GraphNode {
                label: label.unwrap_or_else(|| id.to_string()),
                shape: shape.unwrap_or(NodeShape::Rect),
                subgraph,
                fill: None,
                stroke: None,
                color: None,
                compartments: Vec::new(),
            });
            self.index.insert(id.to_string(), idx);
            if let Some(sg) = subgraph {
                self.subgraphs[sg].nodes.push(idx);
            }
            idx
        }
    }
}

#[derive(Clone, Debug)]
pub struct SeqParticipant {
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteSide {
    Left,
    Right,
    Over,
}

#[derive(Clone, Debug)]
pub enum SeqItem {
    Message {
        from: usize,
        to: usize,
        label: String,
        dotted: bool,
        cross: bool,
    },
    Note {
        text: String,
        start_col: usize,
        end_col: usize,
        side: NoteSide,
    },
    BlockStart {
        kind: String,
        label: String,
    },
    BlockDivider {
        label: String,
    },
    BlockEnd,
}

#[derive(Clone, Debug)]
pub struct SequenceDiagram {
    pub participants: Vec<SeqParticipant>,
    pub items: Vec<SeqItem>,
    /// `(participant_col, start_row, end_row)`
    pub activations: Vec<(usize, usize, usize)>,
}

#[derive(Clone, Debug)]
pub struct PieChart {
    pub title: String,
    pub slices: Vec<(String, f64)>,
}

#[derive(Clone, Debug)]
pub struct GanttChart {
    pub title: String,
    pub tasks: Vec<String>,
}

// ---------------------------------------------------------------------------
// Stage 2 Output: Positioned Vector Scene (Renderer-Agnostic)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAnchor {
    Start,
    Middle,
}

#[derive(Clone, Debug)]
pub struct Stroke {
    pub color: String,
    pub width: f32,
    pub dash: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub enum Primitive {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rx: f32,
        fill: Option<String>,
        stroke: Option<Stroke>,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: Option<String>,
        stroke: Option<Stroke>,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        fill: Option<String>,
        stroke: Option<Stroke>,
    },
    Polygon {
        points: Vec<(f32, f32)>,
        fill: Option<String>,
        stroke: Option<Stroke>,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        stroke: Stroke,
    },
    Path {
        d: String,
        fill: Option<String>,
        stroke: Option<Stroke>,
    },
    Text {
        x: f32,
        y: f32,
        font_size: f32,
        bold: bool,
        anchor: TextAnchor,
        color: String,
        content: String,
    },
}

/// A fully laid-out diagram canvas ready for any vector backend.
#[derive(Clone, Debug)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub background: String,
    pub primitives: Vec<Primitive>,
}
