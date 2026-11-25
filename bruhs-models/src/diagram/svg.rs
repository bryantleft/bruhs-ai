//! Stage 3: SVG vector backend (`Scene` → self-contained SVG string).

use std::fmt::Write as _;

use super::ir::{Primitive, Scene, Stroke, TextAnchor};

/// Serialize a positioned [`Scene`] into a self-contained SVG string.
pub fn render(scene: &Scene, font_family: &str) -> String {
    let mut svg = String::with_capacity(512 + scene.primitives.len() * 96);
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.1}\" height=\"{:.1}\" viewBox=\"0 0 {:.1} {:.1}\" font-family=\"{font_family}\"><rect width=\"100%\" height=\"100%\" rx=\"6\" fill=\"{}\"/>",
        scene.width, scene.height, scene.width, scene.height, scene.background
    );

    for prim in &scene.primitives {
        match prim {
            Primitive::Rect {
                x,
                y,
                w,
                h,
                rx,
                fill,
                stroke,
            } => {
                let _ = write!(
                    svg,
                    "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" rx=\"{rx:.1}\""
                );
                write_paint(&mut svg, fill.as_deref(), stroke.as_ref());
                svg.push_str("/>");
            }
            Primitive::Circle {
                cx,
                cy,
                r,
                fill,
                stroke,
            } => {
                let _ = write!(svg, "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\"");
                write_paint(&mut svg, fill.as_deref(), stroke.as_ref());
                svg.push_str("/>");
            }
            Primitive::Ellipse {
                cx,
                cy,
                rx,
                ry,
                fill,
                stroke,
            } => {
                let _ = write!(
                    svg,
                    "<ellipse cx=\"{cx:.1}\" cy=\"{cy:.1}\" rx=\"{rx:.1}\" ry=\"{ry:.1}\""
                );
                write_paint(&mut svg, fill.as_deref(), stroke.as_ref());
                svg.push_str("/>");
            }
            Primitive::Polygon {
                points,
                fill,
                stroke,
            } => {
                svg.push_str("<polygon points=\"");
                for (i, (px, py)) in points.iter().enumerate() {
                    if i > 0 {
                        svg.push(' ');
                    }
                    let _ = write!(svg, "{px:.1},{py:.1}");
                }
                svg.push('"');
                write_paint(&mut svg, fill.as_deref(), stroke.as_ref());
                svg.push_str("/>");
            }
            Primitive::Line {
                x1,
                y1,
                x2,
                y2,
                stroke,
            } => {
                let _ = write!(
                    svg,
                    "<line x1=\"{x1:.1}\" y1=\"{y1:.1}\" x2=\"{x2:.1}\" y2=\"{y2:.1}\""
                );
                write_stroke(&mut svg, stroke);
                svg.push_str("/>");
            }
            Primitive::Path { d, fill, stroke } => {
                let _ = write!(svg, "<path d=\"{d}\"");
                write_paint(&mut svg, fill.as_deref(), stroke.as_ref());
                svg.push_str("/>");
            }
            Primitive::Text {
                x,
                y,
                font_size,
                bold,
                anchor,
                color,
                content,
            } => {
                let _ = write!(svg, "<text x=\"{x:.1}\" y=\"{y:.1}\"");
                if *anchor == TextAnchor::Middle {
                    svg.push_str(" text-anchor=\"middle\"");
                }
                let _ = write!(svg, " font-size=\"{font_size:.1}\"");
                if *bold {
                    svg.push_str(" font-weight=\"600\"");
                }
                let _ = write!(svg, " fill=\"{color}\">");
                push_xml(&mut svg, content);
                svg.push_str("</text>");
            }
        }
    }

    svg.push_str("</svg>");
    svg
}

fn write_paint(out: &mut String, fill: Option<&str>, stroke: Option<&Stroke>) {
    let _ = write!(out, " fill=\"{}\"", fill.unwrap_or("none"));
    if let Some(stroke) = stroke {
        write_stroke(out, stroke);
    }
}

fn write_stroke(out: &mut String, stroke: &Stroke) {
    let _ = write!(
        out,
        " stroke=\"{}\" stroke-width=\"{:.1}\"",
        stroke.color, stroke.width
    );
    if let Some(dash) = stroke.dash {
        let _ = write!(out, " stroke-dasharray=\"{dash}\"");
    }
}

fn push_xml(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
}
