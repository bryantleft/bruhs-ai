//! Zero-dependency, 3-stage diagram engine (`parse` → `layout` → `render`).
//!
//! - [`ir`]: Format-agnostic [`ir::Diagram`] IR, [`Theme`], and positioned [`ir::Scene`] geometry.
//! - [`mermaid`]: Mermaid text syntax parser → [`ir::Diagram`].
//! - [`layout`]: Sugiyama layered graph, sequence, pie, and gantt layout → [`ir::Scene`].
//! - [`svg`]: Vector SVG backend (`Scene` → SVG string).

pub mod ir;
pub mod layout;
pub mod mermaid;
pub mod svg;

pub use ir::Theme;

/// Parse Mermaid text, lay it out using `theme`, and emit a self-contained SVG.
pub fn render_mermaid_svg(source: &str, theme: &Theme) -> Result<String, String> {
    let diagram = mermaid::parse(source)?;
    let scene = layout::layout(&diagram, theme);
    Ok(svg::render(&scene, &theme.font_family))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_flowchart_lr_with_shapes_subgraphs_and_labels() {
        let src = r#"
flowchart LR
    subgraph Pipeline [Build Pipeline]
        A[Start] --> B{Check <ok>?}
        B -->|yes| C([Deploy & Verify])
        B -.->|no| D[(Logs)]
    end
"#;
        let diagram = mermaid::parse(src).unwrap();
        let scene = layout::layout(&diagram, &Theme::bruhs_dark());
        assert!(scene.width > 120.0 && scene.height > 80.0);
        let svg = svg::render(&scene, &Theme::bruhs_dark().font_family);
        assert!(svg.starts_with("<svg "));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Start"));
        assert!(svg.contains("Check &lt;ok&gt;?"));
        assert!(svg.contains("Deploy &amp; Verify"));
        assert!(svg.contains("Build Pipeline"));
    }

    #[test]
    fn renders_sequence_diagram_with_blocks_and_notes() {
        let src = r#"
sequenceDiagram
    participant U as User
    participant S as Server
    U->>+S: POST /api/chat
    Note over U,S: Streaming SSE
    alt success
        S-->>-U: 200 OK
    else error
        S--xU: 500
    end
"#;
        let svg = render_mermaid_svg(src, &Theme::bruhs_dark()).unwrap();
        assert!(svg.contains("User"));
        assert!(svg.contains("Server"));
        assert!(svg.contains("Streaming SSE"));
        assert!(svg.contains("alt"));
    }

    #[test]
    fn renders_state_class_er_and_pie_diagrams() {
        let state_svg = render_mermaid_svg(
            "stateDiagram-v2\n[*] --> Idle\nIdle --> Running : start\nRunning --> [*]",
            &Theme::bruhs_dark(),
        )
        .unwrap();
        assert!(state_svg.contains("Idle") && state_svg.contains("Running"));

        let class_svg = render_mermaid_svg(
            "classDiagram\nclass Animal {\n+String name\n+speak()\n}\nAnimal <|-- Dog",
            &Theme::bruhs_dark(),
        )
        .unwrap();
        assert!(class_svg.contains("Animal") && class_svg.contains("Dog"));

        let er_svg = render_mermaid_svg(
            "erDiagram\nUSER ||--o{ ORDER : places",
            &Theme::bruhs_dark(),
        )
        .unwrap();
        assert!(er_svg.contains("USER") && er_svg.contains("ORDER"));

        let pie_svg = render_mermaid_svg(
            "pie title Usage\n\"Rust\" : 75\n\"TS\" : 25",
            &Theme::bruhs_dark(),
        )
        .unwrap();
        assert!(pie_svg.contains("Rust (75%)"));
    }

    #[test]
    fn rejects_empty_and_unknown_diagrams() {
        assert!(render_mermaid_svg("   ", &Theme::bruhs_dark()).is_err());
        assert!(render_mermaid_svg("notADiagram\nA-->B", &Theme::bruhs_dark()).is_err());
    }
}
