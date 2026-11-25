//! Stage 1: Mermaid text syntax parser → format-agnostic [`Diagram`] IR.

use std::collections::HashMap;

use super::ir::{
    Diagram, Direction, EdgeStyle, GanttChart, Graph, GraphEdge, NodeShape, NoteSide, PieChart,
    SeqItem, SeqParticipant, SequenceDiagram, Subgraph,
};

type StyleProps = (Option<String>, Option<String>, Option<String>);

/// Parse Mermaid source text into a [`Diagram`] IR.
pub fn parse(source: &str) -> Result<Diagram, String> {
    let lines = preprocess_lines(source);
    let Some(header) = lines.first() else {
        return Err("empty Mermaid diagram".to_string());
    };
    let lower = header.to_ascii_lowercase();
    let rest = &lines[1..];

    if lower.starts_with("flowchart") || lower.starts_with("graph") {
        let dir = parse_direction(header);
        parse_flowchart(rest, dir).map(Diagram::Graph)
    } else if lower.starts_with("sequencediagram") {
        parse_sequence(rest).map(Diagram::Sequence)
    } else if lower.starts_with("statediagram") {
        parse_state_diagram(rest).map(Diagram::Graph)
    } else if lower.starts_with("classdiagram") {
        parse_class_diagram(rest).map(Diagram::Graph)
    } else if lower.starts_with("erdiagram") {
        parse_er_diagram(rest).map(Diagram::Graph)
    } else if lower.starts_with("pie") {
        parse_pie(header, rest).map(Diagram::Pie)
    } else if lower.starts_with("gantt") {
        parse_gantt(rest).map(Diagram::Gantt)
    } else if lower.starts_with("timeline") {
        parse_timeline(rest).map(Diagram::Graph)
    } else if lower.starts_with("mindmap") {
        parse_mindmap(source).map(Diagram::Graph)
    } else {
        Err(format!("unsupported Mermaid diagram header: {header}"))
    }
}

fn preprocess_lines(source: &str) -> Vec<String> {
    let mut raw_lines = source.lines().peekable();
    if raw_lines.peek().is_some_and(|l| l.trim() == "---") {
        raw_lines.next();
        for line in raw_lines.by_ref() {
            if line.trim() == "---" {
                break;
            }
        }
    }
    let mut out = Vec::new();
    for line in raw_lines {
        let without_comment = strip_mermaid_comment(line);
        let trimmed = without_comment.trim();
        if trimmed.is_empty() || trimmed.starts_with("%%") {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

fn strip_mermaid_comment(line: &str) -> &str {
    let mut in_quotes = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes && bytes[i] == b'%' && i + 1 < bytes.len() && bytes[i + 1] == b'%' {
            return &line[..i];
        }
        i += 1;
    }
    line
}

fn parse_direction(header: &str) -> Direction {
    let dir_token = header
        .split_whitespace()
        .nth(1)
        .unwrap_or("TD")
        .trim_end_matches(';');
    match dir_token.to_ascii_uppercase().as_str() {
        "LR" => Direction::Lr,
        "RL" => Direction::Rl,
        "BT" => Direction::Bt,
        _ => Direction::Td,
    }
}

fn parse_flowchart(lines: &[String], dir: Direction) -> Result<Graph, String> {
    let mut graph = Graph::new(dir);
    let mut sg_stack: Vec<usize> = Vec::new();
    let mut class_defs: HashMap<String, StyleProps> = HashMap::new();

    for raw_line in lines {
        for stmt in split_statements(raw_line) {
            let line = stmt.trim();
            if line.is_empty() {
                continue;
            }
            let lower = line.to_ascii_lowercase();
            if lower == "end" {
                sg_stack.pop();
                continue;
            }
            if lower.starts_with("subgraph") {
                let rest = line[8..].trim();
                let title = parse_subgraph_title(rest);
                let sg_id = graph.subgraphs.len();
                graph.subgraphs.push(Subgraph {
                    title,
                    nodes: Vec::new(),
                });
                sg_stack.push(sg_id);
                continue;
            }
            if lower.starts_with("direction ") {
                if sg_stack.is_empty() {
                    graph.dir = parse_direction(line);
                }
                continue;
            }
            if lower.starts_with("classdef ") {
                let parts: Vec<&str> = line.splitn(3, char::is_whitespace).collect();
                if parts.len() == 3 {
                    class_defs.insert(parts[1].trim().to_string(), parse_style_props(parts[2]));
                }
                continue;
            }
            if lower.starts_with("style ") {
                let parts: Vec<&str> = line.splitn(3, char::is_whitespace).collect();
                if parts.len() == 3 {
                    let (fill, stroke, color) = parse_style_props(parts[2]);
                    let id = parts[1].trim();
                    let idx = graph.upsert_node(id, None, None, sg_stack.last().copied());
                    graph.nodes[idx].fill = fill.or(graph.nodes[idx].fill.take());
                    graph.nodes[idx].stroke = stroke.or(graph.nodes[idx].stroke.take());
                    graph.nodes[idx].color = color.or(graph.nodes[idx].color.take());
                }
                continue;
            }
            if lower.starts_with("class ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let class_name = parts[2].trim();
                    if let Some((fill, stroke, color)) = class_defs.get(class_name).cloned() {
                        for id in parts[1].split(',').map(str::trim).filter(|s| !s.is_empty()) {
                            let idx = graph.upsert_node(id, None, None, sg_stack.last().copied());
                            if fill.is_some() {
                                graph.nodes[idx].fill = fill.clone();
                            }
                            if stroke.is_some() {
                                graph.nodes[idx].stroke = stroke.clone();
                            }
                            if color.is_some() {
                                graph.nodes[idx].color = color.clone();
                            }
                        }
                    }
                }
                continue;
            }
            if lower.starts_with("linkstyle ") || lower.starts_with("click ") {
                continue;
            }

            parse_flowchart_statement(line, sg_stack.last().copied(), &mut graph);
        }
    }

    if graph.nodes.is_empty() {
        return Err("flowchart contains no nodes".to_string());
    }
    Ok(graph)
}

fn split_statements(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut depth = 0usize;
    for (i, &b) in line.as_bytes().iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b'[' | b'(' | b'{' if !in_quotes => depth += 1,
            b']' | b')' | b'}' if !in_quotes => depth = depth.saturating_sub(1),
            b';' if !in_quotes && depth == 0 => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

fn parse_subgraph_title(rest: &str) -> String {
    if rest.is_empty() {
        return "Group".to_string();
    }
    if let (Some(open), Some(close)) = (rest.find('['), rest.rfind(']')) {
        if close > open {
            return unquote(&rest[open + 1..close]);
        }
    }
    unquote(rest)
}

fn parse_style_props(props: &str) -> StyleProps {
    let mut fill = None;
    let mut stroke = None;
    let mut color = None;
    for part in props.split(',') {
        if let Some((k, v)) = part.split_once(':') {
            let v = v.trim();
            if !is_safe_color(v) {
                continue;
            }
            match k.trim().to_ascii_lowercase().as_str() {
                "fill" => fill = Some(v.to_string()),
                "stroke" => stroke = Some(v.to_string()),
                "color" => color = Some(v.to_string()),
                _ => {}
            }
        }
    }
    (fill, stroke, color)
}

fn is_safe_color(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '#' | '-' | '(' | ')' | ',' | '.' | '%' | ' ')
        })
}

fn parse_flowchart_statement(line: &str, sg: Option<usize>, graph: &mut Graph) {
    let segments = split_edge_chain(line);
    if segments.is_empty() {
        return;
    }
    let mut prev_nodes = parse_node_group(segments[0].node_text, sg, graph);
    for seg in &segments[1..] {
        let cur_nodes = parse_node_group(seg.node_text, sg, graph);
        for &from in &prev_nodes {
            for &to in &cur_nodes {
                graph.edges.push(GraphEdge {
                    from,
                    to,
                    label: seg.edge_label.clone(),
                    style: seg.edge_style,
                    arrow_end: seg.arrow_end,
                    arrow_start: seg.arrow_start,
                });
            }
        }
        prev_nodes = cur_nodes;
    }
}

struct ChainSeg<'a> {
    node_text: &'a str,
    edge_label: String,
    edge_style: EdgeStyle,
    arrow_end: bool,
    arrow_start: bool,
}

fn split_edge_chain(line: &str) -> Vec<ChainSeg<'_>> {
    let mut result = Vec::new();
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut seg_start = 0;
    let mut in_quotes = false;
    let mut depth = 0usize;
    let mut pending_edge: Option<(EdgeStyle, bool, bool, String)> = None;

    while i < len {
        let b = bytes[i];
        if b == b'"' {
            in_quotes = !in_quotes;
            i += 1;
            continue;
        }
        if !in_quotes {
            if matches!(b, b'[' | b'(' | b'{') {
                depth += 1;
                i += 1;
                continue;
            }
            if matches!(b, b']' | b')' | b'}') {
                depth = depth.saturating_sub(1);
                i += 1;
                continue;
            }
            if depth == 0 {
                if let Some((end_idx, style, arrow_start, arrow_end, label)) =
                    match_edge_operator(line, i)
                {
                    let node_text = line[seg_start..i].trim();
                    let (es, a_start, a_end, e_label) = pending_edge.take().unwrap_or((
                        EdgeStyle::Solid,
                        false,
                        true,
                        String::new(),
                    ));
                    result.push(ChainSeg {
                        node_text,
                        edge_label: e_label,
                        edge_style: es,
                        arrow_end: a_end,
                        arrow_start: a_start,
                    });
                    pending_edge = Some((style, arrow_start, arrow_end, label));
                    i = end_idx;
                    seg_start = i;
                    continue;
                }
            }
        }
        i += 1;
    }

    let tail = line[seg_start..].trim();
    if !tail.is_empty() {
        let (es, a_start, a_end, e_label) =
            pending_edge.unwrap_or((EdgeStyle::Solid, false, true, String::new()));
        result.push(ChainSeg {
            node_text: tail,
            edge_label: e_label,
            edge_style: es,
            arrow_end: a_end,
            arrow_start: a_start,
        });
    }
    result
}

fn match_edge_operator(line: &str, start: usize) -> Option<(usize, EdgeStyle, bool, bool, String)> {
    let rest = &line[start..];
    if let Some(after) = rest.strip_prefix("-- ") {
        if let Some(arrow_pos) = after.find(" -->") {
            let label = after[..arrow_pos].trim().to_string();
            return Some((
                start + 3 + arrow_pos + 4,
                EdgeStyle::Solid,
                false,
                true,
                label,
            ));
        }
        if let Some(arrow_pos) = after.find(" ---") {
            let label = after[..arrow_pos].trim().to_string();
            return Some((
                start + 3 + arrow_pos + 4,
                EdgeStyle::Solid,
                false,
                false,
                label,
            ));
        }
    }
    if let Some(after) = rest.strip_prefix("-. ") {
        if let Some(arrow_pos) = after.find(" .->") {
            let label = after[..arrow_pos].trim().to_string();
            return Some((
                start + 3 + arrow_pos + 4,
                EdgeStyle::Dotted,
                false,
                true,
                label,
            ));
        }
    }
    if let Some(after) = rest.strip_prefix("== ") {
        if let Some(arrow_pos) = after.find(" ==>") {
            let label = after[..arrow_pos].trim().to_string();
            return Some((
                start + 3 + arrow_pos + 4,
                EdgeStyle::Thick,
                false,
                true,
                label,
            ));
        }
    }

    let candidates: &[(&str, EdgeStyle, bool, bool)] = &[
        ("<-->", EdgeStyle::Solid, true, true),
        ("o--o", EdgeStyle::Solid, true, true),
        ("x--x", EdgeStyle::Solid, true, true),
        ("-.->", EdgeStyle::Dotted, false, true),
        ("-..->", EdgeStyle::Dotted, false, true),
        ("-.-", EdgeStyle::Dotted, false, false),
        ("===>", EdgeStyle::Thick, false, true),
        ("==>", EdgeStyle::Thick, false, true),
        ("===", EdgeStyle::Thick, false, false),
        ("---->", EdgeStyle::Solid, false, true),
        ("--->", EdgeStyle::Solid, false, true),
        ("-->", EdgeStyle::Solid, false, true),
        ("---", EdgeStyle::Solid, false, false),
        ("--o", EdgeStyle::Solid, false, true),
        ("--x", EdgeStyle::Solid, false, true),
    ];

    for &(op, style, a_start, a_end) in candidates {
        if rest.starts_with(op) {
            let mut end = start + op.len();
            let after = line[end..].trim_start();
            let ws_len = line[end..].len() - after.len();
            let mut label = String::new();
            if let Some(pipe_rest) = after.strip_prefix('|') {
                if let Some(close) = pipe_rest.find('|') {
                    label = unquote(pipe_rest[..close].trim());
                    end += ws_len + 1 + close + 1;
                }
            }
            return Some((end, style, a_start, a_end, label));
        }
    }
    None
}

fn parse_node_group(text: &str, sg: Option<usize>, graph: &mut Graph) -> Vec<usize> {
    let mut ids = Vec::new();
    let mut start = 0;
    let mut in_quotes = false;
    let mut depth = 0usize;
    for (i, &b) in text.as_bytes().iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b'[' | b'(' | b'{' if !in_quotes => depth += 1,
            b']' | b')' | b'}' if !in_quotes => depth = depth.saturating_sub(1),
            b'&' if !in_quotes && depth == 0 => {
                if let Some(idx) = parse_single_node(text[start..i].trim(), sg, graph) {
                    ids.push(idx);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if let Some(idx) = parse_single_node(text[start..].trim(), sg, graph) {
        ids.push(idx);
    }
    ids
}

fn parse_single_node(raw: &str, sg: Option<usize>, graph: &mut Graph) -> Option<usize> {
    let raw = raw.split(":::").next().unwrap_or(raw).trim();
    if raw.is_empty() {
        return None;
    }
    let Some(pos) = raw.find(['[', '(', '{']) else {
        return Some(graph.upsert_node(raw, None, None, sg));
    };
    let id = raw[..pos].trim();
    if id.is_empty() {
        return None;
    }
    let (shape, label_raw) = parse_shape_delim(raw[pos..].trim());
    Some(graph.upsert_node(id, Some(clean_label(label_raw)), Some(shape), sg))
}

fn parse_shape_delim(s: &str) -> (NodeShape, &str) {
    let pairs: &[(&str, &str, NodeShape)] = &[
        ("(((", ")))", NodeShape::DoubleCircle),
        ("([", "])", NodeShape::Stadium),
        ("[(", ")]", NodeShape::Cylinder),
        ("((", "))", NodeShape::Circle),
        ("{{", "}}", NodeShape::Hexagon),
        ("[[", "]]", NodeShape::Subroutine),
        ("[/", "/]", NodeShape::Parallelogram),
        ("[\\", "\\]", NodeShape::Parallelogram),
        ("[", "]", NodeShape::Rect),
        ("(", ")", NodeShape::Rounded),
        ("{", "}", NodeShape::Diamond),
    ];
    for &(open, close, shape) in pairs {
        if let Some(rest) = s.strip_prefix(open) {
            if let Some(inner) = rest.strip_suffix(close) {
                return (shape, inner);
            }
        }
    }
    (NodeShape::Rect, s)
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        t[1..t.len() - 1].to_string()
    } else {
        t.to_string()
    }
}

fn clean_label(raw: &str) -> String {
    unquote(raw)
        .replace("<br/>", "\n")
        .replace("<br />", "\n")
        .replace("<br>", "\n")
        .replace("\\n", "\n")
}

fn split_as(s: &str) -> Option<(&str, &str)> {
    let lower = s.to_ascii_lowercase();
    let idx = lower.find(" as ")?;
    Some((&s[..idx], &s[idx + 4..]))
}

// ---------------------------------------------------------------------------
// Sequence Diagram Parser
// ---------------------------------------------------------------------------

fn parse_sequence(lines: &[String]) -> Result<SequenceDiagram, String> {
    let mut participants: Vec<SeqParticipant> = Vec::new();
    let mut p_index: HashMap<String, usize> = HashMap::new();
    let mut items: Vec<SeqItem> = Vec::new();
    let mut activations: Vec<(usize, usize, usize)> = Vec::new();
    let mut open_act: HashMap<usize, Vec<usize>> = HashMap::new();

    let ensure_p = |id: &str,
                    label: Option<&str>,
                    ps: &mut Vec<SeqParticipant>,
                    idx: &mut HashMap<String, usize>|
     -> usize {
        if let Some(&i) = idx.get(id) {
            if let Some(l) = label {
                ps[i].label = l.to_string();
            }
            i
        } else {
            let i = ps.len();
            ps.push(SeqParticipant {
                label: label.unwrap_or(id).to_string(),
            });
            idx.insert(id.to_string(), i);
            i
        }
    };

    for line in lines {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("participant ") || lower.starts_with("actor ") {
            let rest = line.split_once(' ').map_or("", |(_, r)| r).trim();
            if let Some((id, alias)) = split_as(rest) {
                ensure_p(
                    id.trim(),
                    Some(alias.trim()),
                    &mut participants,
                    &mut p_index,
                );
            } else if !rest.is_empty() {
                ensure_p(rest, None, &mut participants, &mut p_index);
            }
            continue;
        }
        if lower.starts_with("activate ") {
            let col = ensure_p(line[9..].trim(), None, &mut participants, &mut p_index);
            open_act.entry(col).or_default().push(items.len());
            continue;
        }
        if lower.starts_with("deactivate ") {
            if let Some(&col) = p_index.get(line[11..].trim()) {
                if let Some(start) = open_act.get_mut(&col).and_then(Vec::pop) {
                    activations.push((col, start, items.len()));
                }
            }
            continue;
        }
        if lower.starts_with("note ") {
            if let Some((target_part, text)) = line[5..].split_once(':') {
                let target_trimmed = target_part.trim();
                let target_lower = target_trimmed.to_ascii_lowercase();
                let (side, who) = if let Some(r) = target_lower.strip_prefix("left of ") {
                    (NoteSide::Left, &target_trimmed[8..8 + r.len()])
                } else if let Some(r) = target_lower.strip_prefix("right of ") {
                    (NoteSide::Right, &target_trimmed[9..9 + r.len()])
                } else if let Some(r) = target_lower.strip_prefix("over ") {
                    (NoteSide::Over, &target_trimmed[5..5 + r.len()])
                } else {
                    continue;
                };
                let cols: Vec<usize> = who
                    .split(',')
                    .map(|p| ensure_p(p.trim(), None, &mut participants, &mut p_index))
                    .collect();
                if let (Some(&first), Some(&last)) = (cols.first(), cols.last()) {
                    items.push(SeqItem::Note {
                        text: clean_label(text.trim()),
                        start_col: first.min(last),
                        end_col: first.max(last),
                        side,
                    });
                }
            }
            continue;
        }
        if lower == "end" {
            items.push(SeqItem::BlockEnd);
            continue;
        }
        if lower.starts_with("else") || lower.starts_with("and") {
            let label = line
                .split_once(' ')
                .map_or("", |(_, r)| r)
                .trim()
                .to_string();
            items.push(SeqItem::BlockDivider { label });
            continue;
        }
        for kw in ["loop", "alt", "opt", "par", "critical", "rect"] {
            if lower.starts_with(kw)
                && line
                    .as_bytes()
                    .get(kw.len())
                    .is_none_or(u8::is_ascii_whitespace)
            {
                items.push(SeqItem::BlockStart {
                    kind: kw.to_string(),
                    label: line[kw.len()..].trim().to_string(),
                });
                break;
            }
        }
        if let Some((from_id, to_id, label, dotted, cross, act_plus, act_minus)) =
            parse_seq_message(line)
        {
            let from = ensure_p(from_id, None, &mut participants, &mut p_index);
            let to = ensure_p(to_id, None, &mut participants, &mut p_index);
            let row_idx = items.len();
            items.push(SeqItem::Message {
                from,
                to,
                label: clean_label(label),
                dotted,
                cross,
            });
            if act_plus {
                open_act.entry(to).or_default().push(row_idx);
            }
            if act_minus {
                if let Some(start) = open_act.get_mut(&from).and_then(Vec::pop) {
                    activations.push((from, start, row_idx + 1));
                }
            }
        }
    }

    for (col, starts) in open_act {
        for start in starts {
            activations.push((col, start, items.len()));
        }
    }

    if participants.is_empty() {
        return Err("sequenceDiagram has no participants or messages".to_string());
    }
    Ok(SequenceDiagram {
        participants,
        items,
        activations,
    })
}

type SeqMsg<'a> = (&'a str, &'a str, &'a str, bool, bool, bool, bool);

fn parse_seq_message(line: &str) -> Option<SeqMsg<'_>> {
    let (lhs, label) = line.split_once(':')?;
    let arrows: &[(&str, bool, bool)] = &[
        ("-->>", true, false),
        ("->>", false, false),
        ("--x", true, true),
        ("-x", false, true),
        ("--)", true, false),
        ("-)", false, false),
        ("-->", true, false),
        ("->", false, false),
    ];
    for &(op, dotted, cross) in arrows {
        if let Some((from_raw, to_raw)) = lhs.split_once(op) {
            let from = from_raw.trim();
            let mut to = to_raw.trim();
            let mut act_plus = false;
            let mut act_minus = false;
            if let Some(rest) = to.strip_prefix('+') {
                act_plus = true;
                to = rest.trim();
            } else if let Some(rest) = to.strip_prefix('-') {
                act_minus = true;
                to = rest.trim();
            }
            if !from.is_empty() && !to.is_empty() {
                return Some((from, to, label.trim(), dotted, cross, act_plus, act_minus));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// State / Class / ER / Pie / Gantt / Timeline / Mindmap Parsers
// ---------------------------------------------------------------------------

fn parse_state_diagram(lines: &[String]) -> Result<Graph, String> {
    let mut graph = Graph::new(Direction::Td);
    let mut sg_stack: Vec<usize> = Vec::new();
    let mut star_counter = 0usize;

    for line in lines {
        let lower = line.to_ascii_lowercase();
        if line == "}" {
            sg_stack.pop();
            continue;
        }
        if lower.starts_with("direction ") {
            graph.dir = parse_direction(line);
            continue;
        }
        if lower.starts_with("state ") && line.ends_with('{') {
            let title = line[6..line.len() - 1].trim();
            let sg_id = graph.subgraphs.len();
            graph.subgraphs.push(Subgraph {
                title: unquote(title),
                nodes: Vec::new(),
            });
            sg_stack.push(sg_id);
            continue;
        }
        if lower.starts_with("state ") {
            if let Some((desc, id)) = split_as(line[6..].trim()) {
                graph.upsert_node(
                    id.trim(),
                    Some(clean_label(desc)),
                    Some(NodeShape::Rounded),
                    sg_stack.last().copied(),
                );
            }
            continue;
        }
        if let Some((left_part, right_part)) = line.split_once("-->") {
            let (to_part, label) = right_part
                .split_once(':')
                .map_or((right_part, ""), |(t, l)| (t, l.trim()));
            let from_raw = left_part.trim();
            let to_raw = to_part.trim();
            let sg = sg_stack.last().copied();
            let from = if from_raw == "[*]" {
                star_counter += 1;
                graph.upsert_node(
                    &format!("__start_{star_counter}"),
                    Some(String::new()),
                    Some(NodeShape::StartDot),
                    sg,
                )
            } else {
                graph.upsert_node(from_raw, None, Some(NodeShape::Rounded), sg)
            };
            let to = if to_raw == "[*]" {
                star_counter += 1;
                graph.upsert_node(
                    &format!("__end_{star_counter}"),
                    Some(String::new()),
                    Some(NodeShape::EndBullseye),
                    sg,
                )
            } else {
                graph.upsert_node(to_raw, None, Some(NodeShape::Rounded), sg)
            };
            graph.edges.push(GraphEdge {
                from,
                to,
                label: clean_label(label),
                style: EdgeStyle::Solid,
                arrow_end: true,
                arrow_start: false,
            });
        }
    }

    if graph.nodes.is_empty() {
        return Err("stateDiagram contains no states".to_string());
    }
    Ok(graph)
}

fn parse_class_diagram(lines: &[String]) -> Result<Graph, String> {
    let mut graph = Graph::new(Direction::Td);
    let mut active_class: Option<usize> = None;

    for line in lines {
        if let Some(cls) = active_class {
            if line == "}" {
                active_class = None;
                continue;
            }
            if line.starts_with("<<") && line.ends_with(">>") {
                graph.nodes[cls].label = format!("{line}\n{}", graph.nodes[cls].label);
            } else {
                if graph.nodes[cls].compartments.is_empty() {
                    graph.nodes[cls].compartments.push(Vec::new());
                }
                graph.nodes[cls].compartments[0].push(line.clone());
            }
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("direction ") {
            graph.dir = parse_direction(line);
            continue;
        }
        if lower.starts_with("class ") {
            let rest = line[6..].trim();
            let opens = rest.ends_with('{');
            let name = rest.trim_end_matches('{').trim();
            if !name.is_empty() {
                let idx =
                    graph.upsert_node(name, Some(name.to_string()), Some(NodeShape::Rect), None);
                if opens {
                    active_class = Some(idx);
                }
            }
            continue;
        }
        if let Some((from_s, to_s, label, style, a_start, a_end)) = parse_class_relation(line) {
            let from = graph.upsert_node(from_s, None, Some(NodeShape::Rect), None);
            let to = graph.upsert_node(to_s, None, Some(NodeShape::Rect), None);
            graph.edges.push(GraphEdge {
                from,
                to,
                label,
                style,
                arrow_end: a_end,
                arrow_start: a_start,
            });
            continue;
        }
        if let Some((cls_name, member)) = line.split_once(':') {
            let cls_name = cls_name.trim();
            if !cls_name.contains(' ') && !cls_name.is_empty() {
                let idx = graph.upsert_node(cls_name, None, Some(NodeShape::Rect), None);
                if graph.nodes[idx].compartments.is_empty() {
                    graph.nodes[idx].compartments.push(Vec::new());
                }
                graph.nodes[idx].compartments[0].push(member.trim().to_string());
            }
        }
    }

    if graph.nodes.is_empty() {
        return Err("classDiagram contains no classes".to_string());
    }
    Ok(graph)
}

fn parse_class_relation(line: &str) -> Option<(&str, &str, String, EdgeStyle, bool, bool)> {
    let (main, label) = line
        .split_once(':')
        .map_or((line, ""), |(m, l)| (m, l.trim()));
    let ops: &[(&str, EdgeStyle, bool, bool)] = &[
        ("<|--", EdgeStyle::Solid, true, false),
        ("--|>", EdgeStyle::Solid, false, true),
        ("<|..", EdgeStyle::Dotted, true, false),
        ("..|>", EdgeStyle::Dotted, false, true),
        ("*--", EdgeStyle::Solid, true, false),
        ("--*", EdgeStyle::Solid, false, true),
        ("o--", EdgeStyle::Solid, true, false),
        ("--o", EdgeStyle::Solid, false, true),
        ("-->", EdgeStyle::Solid, false, true),
        ("<--", EdgeStyle::Solid, true, false),
        ("..>", EdgeStyle::Dotted, false, true),
        ("<..", EdgeStyle::Dotted, true, false),
        ("--", EdgeStyle::Solid, false, false),
        ("..", EdgeStyle::Dotted, false, false),
    ];
    for &(op, style, a_start, a_end) in ops {
        if let Some((lhs, rhs)) = main.split_once(op) {
            let from = lhs.split_whitespace().next()?;
            let to = rhs.split_whitespace().last()?;
            return Some((from, to, clean_label(label), style, a_start, a_end));
        }
    }
    None
}

fn parse_er_diagram(lines: &[String]) -> Result<Graph, String> {
    let mut graph = Graph::new(Direction::Td);
    let mut active_entity: Option<usize> = None;

    for line in lines {
        if let Some(ent) = active_entity {
            if line == "}" {
                active_entity = None;
                continue;
            }
            if graph.nodes[ent].compartments.is_empty() {
                graph.nodes[ent].compartments.push(Vec::new());
            }
            graph.nodes[ent].compartments[0].push(line.clone());
            continue;
        }
        if line.ends_with('{') && !line.contains("--") && !line.contains("..") {
            let name = line.trim_end_matches('{').trim();
            if !name.is_empty() {
                let idx =
                    graph.upsert_node(name, Some(name.to_string()), Some(NodeShape::Rect), None);
                active_entity = Some(idx);
            }
            continue;
        }
        let (rel_part, label) = line
            .split_once(':')
            .map_or((line.as_str(), ""), |(r, l)| (r, l.trim()));
        let tokens: Vec<&str> = rel_part.split_whitespace().collect();
        if tokens.len() >= 3 && (tokens[1].contains("--") || tokens[1].contains("..")) {
            let from = graph.upsert_node(tokens[0], None, Some(NodeShape::Rect), None);
            let to = graph.upsert_node(tokens[2], None, Some(NodeShape::Rect), None);
            let dotted = tokens[1].contains("..");
            let edge_label = if label.is_empty() {
                tokens[1].to_string()
            } else {
                format!("{} ({})", clean_label(label), tokens[1])
            };
            graph.edges.push(GraphEdge {
                from,
                to,
                label: edge_label,
                style: if dotted {
                    EdgeStyle::Dotted
                } else {
                    EdgeStyle::Solid
                },
                arrow_end: false,
                arrow_start: false,
            });
        }
    }

    if graph.nodes.is_empty() {
        return Err("erDiagram contains no entities".to_string());
    }
    Ok(graph)
}

fn parse_pie(header: &str, lines: &[String]) -> Result<PieChart, String> {
    let mut title = header
        .split_once("title ")
        .map(|(_, t)| t.trim().to_string())
        .unwrap_or_default();
    let mut slices: Vec<(String, f64)> = Vec::new();
    for line in lines {
        if let Some(t) = line.strip_prefix("title ") {
            title = t.trim().to_string();
            continue;
        }
        if let Some((label, val_str)) = line.split_once(':') {
            if let Ok(val) = val_str.trim().parse::<f64>() {
                if val > 0.0 {
                    slices.push((unquote(label.trim()), val));
                }
            }
        }
    }
    if slices.is_empty() {
        return Err("pie chart has no positive slices".to_string());
    }
    Ok(PieChart { title, slices })
}

fn parse_gantt(lines: &[String]) -> Result<GanttChart, String> {
    let mut title = String::new();
    let mut tasks = Vec::new();
    let mut section = String::new();
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if let Some(t) = line.strip_prefix("title ") {
            title = t.trim().to_string();
        } else if let Some(s) = line.strip_prefix("section ") {
            section = s.trim().to_string();
        } else if lower.starts_with("dateformat") || lower.starts_with("axisformat") {
            continue;
        } else if let Some((name, _)) = line.split_once(':') {
            let label = if section.is_empty() {
                name.trim().to_string()
            } else {
                format!("{section} · {}", name.trim())
            };
            tasks.push(label);
        }
    }
    if tasks.is_empty() {
        return Err("gantt diagram has no tasks".to_string());
    }
    Ok(GanttChart { title, tasks })
}

fn parse_timeline(lines: &[String]) -> Result<Graph, String> {
    let mut graph = Graph::new(Direction::Lr);
    let mut prev: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.to_ascii_lowercase().starts_with("title ") {
            continue;
        }
        let node = graph.upsert_node(
            &format!("t{idx}"),
            Some(line.replace(':', "\n")),
            Some(NodeShape::Rounded),
            None,
        );
        if let Some(p) = prev {
            graph.edges.push(GraphEdge {
                from: p,
                to: node,
                label: String::new(),
                style: EdgeStyle::Solid,
                arrow_end: true,
                arrow_start: false,
            });
        }
        prev = Some(node);
    }
    if graph.nodes.is_empty() {
        return Err("timeline contains no events".to_string());
    }
    Ok(graph)
}

fn parse_mindmap(source: &str) -> Result<Graph, String> {
    let mut graph = Graph::new(Direction::Lr);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut started = false;
    for (idx, raw) in source.lines().enumerate() {
        let trimmed = strip_mermaid_comment(raw).trim();
        if trimmed.is_empty() {
            continue;
        }
        if !started {
            if trimmed.to_ascii_lowercase().starts_with("mindmap") {
                started = true;
            }
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        let label = trimmed
            .trim_start_matches(['(', '[', '{'])
            .trim_end_matches([')', ']', '}'])
            .trim();
        let node = graph.upsert_node(
            &format!("m{idx}"),
            Some(clean_label(label)),
            Some(NodeShape::Rounded),
            None,
        );
        while stack.last().is_some_and(|&(ind, _)| ind >= indent) {
            stack.pop();
        }
        if let Some(&(_, parent)) = stack.last() {
            graph.edges.push(GraphEdge {
                from: parent,
                to: node,
                label: String::new(),
                style: EdgeStyle::Solid,
                arrow_end: false,
                arrow_start: false,
            });
        }
        stack.push((indent, node));
    }
    if graph.nodes.is_empty() {
        return Err("mindmap contains no nodes".to_string());
    }
    Ok(graph)
}
