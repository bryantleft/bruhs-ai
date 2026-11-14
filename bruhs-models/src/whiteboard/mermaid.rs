//! Editable Mermaid flowcharts and basic sequence diagrams, without browser code.
use super::*;
use crate::num::ToFloatLossy as _;
use std::collections::{BTreeMap, VecDeque};

fn invalid(message: &str) -> BoardError {
    BoardError::Invalid(format!("Mermaid import: {message}"))
}
pub fn import_mermaid(
    text: &str,
    project: String,
    title: String,
    now_ms: i64,
) -> BoardResult<BoardDocument> {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("%%"));
    let header = lines
        .next()
        .ok_or_else(|| invalid("paste a flowchart or sequenceDiagram."))?;
    let rest = lines.collect::<Vec<_>>();
    let board = if header == "sequenceDiagram" {
        sequence(&rest, project, title, now_ms)?
    } else {
        let mut words = header.trim_end_matches(';').split_whitespace();
        if !matches!(words.next(), Some("flowchart" | "graph")) {
            return Err(invalid(
                "supported headers are flowchart, graph and sequenceDiagram. Other diagram types stay as Mermaid text.",
            ));
        }
        let direction = words.next().unwrap_or("TD");
        if !["TD", "TB", "BT", "LR", "RL"].contains(&direction) || words.next().is_some() {
            return Err(invalid(
                "use one of TD, TB, BT, LR or RL on the header line.",
            ));
        }
        flowchart(&rest, direction, project, title, now_ms)?
    };
    board.validate()?;
    Ok(board)
}
#[derive(Clone)]
struct Node {
    text: String,
    ellipse: bool,
    explicit: bool,
}
fn label(value: &str) -> BoardResult<String> {
    let value = value.trim();
    let quoted = value.starts_with('"');
    let value = if value.starts_with('"') {
        serde_json::from_str::<String>(value)
            .map_err(|_| invalid("a quoted label has invalid escaping."))?
    } else {
        value.to_string()
    };
    if !quoted && value.contains(['[', ']']) {
        return Err(invalid("nested bracket labels require quoted text."));
    }
    Ok(value.replace("<br/>", "\n").replace("<br>", "\n"))
}
fn node(
    value: &str,
    nodes: &mut BTreeMap<String, Node>,
    order: &mut Vec<String>,
) -> BoardResult<String> {
    let value = value.trim();
    let end = value
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
        .unwrap_or(value.len());
    let id = &value[..end];
    validate_id(id).map_err(|_| {
        invalid("node IDs may contain letters, numbers, underscores, dots and hyphens.")
    })?;
    let suffix = value[end..].trim();
    let parsed = if suffix.is_empty() {
        Node {
            text: id.into(),
            ellipse: false,
            explicit: false,
        }
    } else if let Some(text) = suffix.strip_prefix("((").and_then(|s| s.strip_suffix("))")) {
        Node {
            text: label(text)?,
            ellipse: true,
            explicit: true,
        }
    } else if let Some(text) = suffix.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        Node {
            text: label(text)?,
            ellipse: false,
            explicit: true,
        }
    } else if let Some(text) = suffix.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        Node {
            text: label(text)?,
            ellipse: true,
            explicit: true,
        }
    } else {
        return Err(invalid(
            "this node syntax is unsupported; use rectangular [labels] or elliptical (labels).",
        ));
    };
    if let Some(previous) = nodes.get(id) {
        if parsed.explicit
            && previous.explicit
            && (parsed.text != previous.text || parsed.ellipse != previous.ellipse)
        {
            return Err(invalid("a node has conflicting definitions."));
        }
        if parsed.explicit {
            nodes.insert(id.into(), parsed);
        }
    } else {
        nodes.insert(id.into(), parsed);
        order.push(id.into());
    }
    Ok(id.into())
}
fn split_statements(line: &str) -> BoardResult<Vec<&str>> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    let mut quoted = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted {
            match ch {
                '[' | '(' => {
                    depth = depth
                        .checked_add(1)
                        .ok_or_else(|| invalid("bracket nesting is invalid."))?
                }
                ']' | ')' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or_else(|| invalid("brackets are unbalanced."))?
                }
                ';' if depth == 0 => {
                    if !line[start..index].trim().is_empty() {
                        statements.push(line[start..index].trim());
                    }
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    if quoted || depth != 0 {
        return Err(invalid("a label has unbalanced quotes or brackets."));
    }
    if !line[start..].trim().is_empty() {
        statements.push(line[start..].trim());
    }
    Ok(statements)
}
fn arrow_at(value: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0_i32;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quoted && ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            quoted = !quoted;
        }
        if !quoted {
            match ch {
                '[' | '(' => depth += 1,
                ']' | ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 && value[index..].starts_with("-->") {
                return Some(index);
            }
        }
    }
    None
}
fn flowchart(
    lines: &[&str],
    direction: &str,
    project: String,
    title: String,
    now: i64,
) -> BoardResult<BoardDocument> {
    let mut nodes = BTreeMap::new();
    let mut order = Vec::new();
    let mut edges = Vec::new();
    for line in lines {
        for statement in split_statements(line)? {
            if [
                "subgraph ",
                "end",
                "style ",
                "class ",
                "classDef ",
                "linkStyle ",
                "click ",
                "direction ",
            ]
            .iter()
            .any(|prefix| statement.starts_with(prefix))
            {
                return Err(invalid(
                    "subgraphs, styling and executable directives are unsupported. Keep the original text or simplify before importing.",
                ));
            }
            let mut rest = statement;
            let mut previous = None;
            loop {
                if let Some(index) = arrow_at(rest) {
                    let from = node(&rest[..index], &mut nodes, &mut order)?;
                    if let Some((prior, label)) = previous.take() {
                        edges.push((prior, from.clone(), label));
                    }
                    rest = rest[index + 3..].trim();
                    let edge_label = if let Some(after) = rest.strip_prefix('|') {
                        let end = after
                            .find('|')
                            .ok_or_else(|| invalid("an edge label is missing its closing pipe."))?;
                        let text = label(&after[..end])?;
                        rest = after[end + 1..].trim();
                        text
                    } else {
                        String::new()
                    };
                    previous = Some((from, edge_label));
                } else {
                    let to = node(rest, &mut nodes, &mut order)?;
                    if let Some((from, label)) = previous.take() {
                        edges.push((from, to, label));
                    }
                    break;
                }
            }
        }
    }
    let mut indegree = nodes
        .keys()
        .map(|id| (id.clone(), 0_usize))
        .collect::<BTreeMap<_, _>>();
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (from, to, _) in &edges {
        *indegree.get_mut(to).ok_or(BoardError::Missing)? += 1;
        children.entry(from.clone()).or_default().push(to.clone());
    }
    let mut queue = order
        .iter()
        .filter(|id| indegree.get(*id) == Some(&0))
        .cloned()
        .collect::<VecDeque<_>>();
    let mut ranks: BTreeMap<String, usize> = BTreeMap::new();
    while let Some(id) = queue.pop_front() {
        let rank = ranks.get(&id).copied().unwrap_or(0);
        if let Some(next) = children.get(&id) {
            for to in next {
                ranks
                    .entry(to.clone())
                    .and_modify(|r| *r = (*r).max(rank + 1))
                    .or_insert(rank + 1);
                let degree = indegree.get_mut(to).ok_or(BoardError::Missing)?;
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(to.clone());
                }
            }
        }
    }
    let mut board = BoardDocument::empty(uuid::Uuid::new_v4().to_string(), project, title, now);
    let mut slots = BTreeMap::new();
    for (index, id) in order.into_iter().enumerate() {
        let rank = ranks.get(&id).copied().unwrap_or(0);
        let slot = slots.entry(rank).or_insert(0_usize);
        let (a, b) = (rank.to_f64_lossy() * 280.0, slot.to_f64_lossy() * 140.0);
        *slot += 1;
        let (x, y) = match direction {
            "LR" => (a, b),
            "RL" => (-a, b),
            "BT" => (b, -a),
            _ => (b, a),
        };
        let item = &nodes[&id];
        let shape = Shape::node(
            id.clone(),
            if item.ellipse {
                ShapeKind::Ellipse
            } else {
                ShapeKind::Rectangle
            },
            Rect::new(x, y, 180.0, 80.0),
            i64::try_from(index).map_err(|_| invalid("too many nodes."))?,
            item.text.clone(),
        );
        board.shapes.insert(id, shape);
    }
    let mut editor = BoardEditor::new(board)?;
    for (from, to, label) in edges {
        editor.connect(&from, &to, label, now)?;
    }
    let mut board = editor.document().clone();
    board.revision = 0;
    Ok(board)
}
fn sequence(
    lines: &[&str],
    project: String,
    title: String,
    now: i64,
) -> BoardResult<BoardDocument> {
    let mut participants: BTreeMap<String, (String, bool)> = BTreeMap::new();
    let mut order = Vec::new();
    let mut messages = Vec::new();
    for line in lines {
        if let Some((rest, actor)) = line
            .strip_prefix("participant ")
            .map(|s| (s, false))
            .or_else(|| line.strip_prefix("actor ").map(|s| (s, true)))
        {
            let (id, name) = rest.split_once(" as ").unwrap_or((rest, rest));
            validate_id(id.trim())?;
            if participants.contains_key(id.trim()) {
                return Err(invalid("a sequence participant was declared twice."));
            }
            participants.insert(id.trim().into(), (name.trim().into(), actor));
            order.push(id.trim().to_owned());
            continue;
        }
        let (body,message)=line.split_once(':').ok_or_else(||invalid("sequence diagrams support participants and one message per line; loops, notes and alternatives remain as text."))?;
        let mut found = None;
        for (token, dashed) in [("-->>", true), ("->>", false), ("-->", true), ("->", false)] {
            if let Some((from, to)) = body.split_once(token) {
                found = Some((from.trim().to_string(), to.trim().to_string(), dashed));
                break;
            }
        }
        let (from, to, dashed) =
            found.ok_or_else(|| invalid("use ->>, -->>, -> or --> sequence messages."))?;
        for id in [&from, &to] {
            validate_id(id)?;
            if !participants.contains_key(id) {
                participants.insert(id.clone(), (id.clone(), false));
                order.push(id.clone());
            }
        }
        if from == to {
            return Err(invalid(
                "self-messages need a loop path and are not yet editable; keep this diagram as text.",
            ));
        }
        messages.push((from, to, message.trim().to_string(), dashed));
    }
    let mut board = BoardDocument::empty(uuid::Uuid::new_v4().to_string(), project, title, now);
    let height = messages.len().to_f64_lossy() * 80.0 + 120.0;
    for (index, id) in order.into_iter().enumerate() {
        let (name, actor) = &participants[&id];
        let mut shape = Shape::node(
            id.clone(),
            ShapeKind::Frame,
            Rect::new(index.to_f64_lossy() * 280.0, 0.0, 180.0, height),
            i64::try_from(index).map_err(|_| invalid("too many participants."))?,
            name.clone(),
        );
        shape.semantic = Some(ShapeSemantic::SequenceParticipant {
            name: id.clone(),
            actor: *actor,
        });
        shape.style.fill = None;
        board.shapes.insert(id, shape);
    }
    for (index, (from, to, text, dashed)) in messages.into_iter().enumerate() {
        let y = (80.0 + index.to_f64_lossy() * 80.0) / height;
        let a = &board.shapes[&from];
        let b = &board.shapes[&to];
        let rect = Rect::new(
            a.rect.center().x.min(b.rect.center().x),
            y * height - 24.0,
            (a.rect.center().x - b.rect.center().x).abs(),
            24.0,
        );
        let id = format!("message-{index}");
        let mut shape = Shape::node(
            id.clone(),
            ShapeKind::Arrow {
                start: Endpoint::bound(from.clone(), Point::new(0.5, y)),
                end: Endpoint::bound(to.clone(), Point::new(0.5, y)),
            },
            rect,
            i64::try_from(board.shapes.len()).map_err(|_| invalid("too many messages."))?,
            text,
        );
        shape.semantic = Some(ShapeSemantic::SequenceMessage {
            from,
            to,
            index: u32::try_from(index).map_err(|_| invalid("too many messages."))?,
            dashed,
        });
        shape.style.dashed = dashed;
        shape.style.fill = None;
        board.shapes.insert(id, shape);
    }
    Ok(board)
}
fn quoted(text: &str) -> BoardResult<String> {
    serde_json::to_string(text).map_err(|e| BoardError::Invalid(e.to_string()))
}
pub fn export_mermaid_selection(
    board: &BoardDocument,
    selection: Option<&std::collections::BTreeSet<ShapeId>>,
) -> BoardResult<String> {
    if let Some(selection) = selection {
        let mut editor = BoardEditor::new(board.clone())?;
        editor.selection = selection.clone();
        export_mermaid(&editor.copy_selection()?)
    } else {
        export_mermaid(board)
    }
}
pub fn export_mermaid(board: &BoardDocument) -> BoardResult<String> {
    board.validate()?;
    if board
        .shapes
        .values()
        .any(|s| matches!(s.semantic, Some(ShapeSemantic::SequenceParticipant { .. })))
    {
        let mut lines = vec!["sequenceDiagram".to_string()];
        let mut names = BTreeMap::new();
        let mut participants = board
            .shapes
            .values()
            .filter(|s| matches!(s.semantic, Some(ShapeSemantic::SequenceParticipant { .. })))
            .collect::<Vec<_>>();
        participants.sort_by_key(|shape| shape.z);
        for shape in participants {
            if let Some(ShapeSemantic::SequenceParticipant { name, actor }) = &shape.semantic {
                validate_id(name)?;
                if shape.text.contains(['\n', '\r']) {
                    return Err(invalid(
                        "participant labels must fit on one line for sequence export.",
                    ));
                }
                names.insert(shape.id.clone(), name.clone());
                lines.push(format!(
                    "    {} {} as {}",
                    if *actor { "actor" } else { "participant" },
                    name,
                    shape.text
                ));
            }
        }
        let mut messages = board
            .shapes
            .values()
            .filter_map(|s| {
                if let Some(ShapeSemantic::SequenceMessage { index, .. }) = &s.semantic {
                    Some((*index, s))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        messages.sort_by_key(|(index, _)| *index);
        for (_, shape) in messages {
            if let Some(ShapeSemantic::SequenceMessage {
                from, to, dashed, ..
            }) = &shape.semantic
            {
                let a = names
                    .get(from)
                    .ok_or_else(|| invalid("a sequence message participant is missing."))?;
                let b = names
                    .get(to)
                    .ok_or_else(|| invalid("a sequence message participant is missing."))?;
                if shape.text.contains(['\n', '\r']) {
                    return Err(invalid("sequence message labels must fit on one line."));
                }
                lines.push(format!(
                    "    {a}{}{b}: {}",
                    if *dashed { "-->>" } else { "->>" },
                    shape.text
                ));
            }
        }
        if board
            .shapes
            .values()
            .any(|s| s.semantic.is_none() && !matches!(s.kind, ShapeKind::Frame | ShapeKind::Group))
        {
            return Err(invalid(
                "this sequence board has additional untyped shapes. Use portable JSON for a lossless export.",
            ));
        }
        return Ok(lines.join("\n"));
    }
    let mut lines = vec!["flowchart LR".to_string()];
    let mut ids = BTreeMap::new();
    for (index, shape) in board
        .shapes
        .values()
        .filter(|s| {
            !matches!(
                s.kind,
                ShapeKind::Arrow { .. } | ShapeKind::Frame | ShapeKind::Group
            )
        })
        .enumerate()
    {
        if !matches!(
            shape.kind,
            ShapeKind::Rectangle | ShapeKind::Ellipse | ShapeKind::Text | ShapeKind::Note
        ) {
            return Err(invalid(
                "Mermaid flowchart export supports nodes and bound arrows. Export portable JSON to retain frames, groups, paths and images.",
            ));
        }
        let id = format!("n{index}");
        ids.insert(shape.id.clone(), id.clone());
        let text = quoted(&shape.text)?;
        lines.push(if matches!(shape.kind, ShapeKind::Ellipse) {
            format!("    {id}({text})")
        } else {
            format!("    {id}[{text}]")
        });
    }
    for shape in board.shapes.values() {
        if let ShapeKind::Arrow { start, end } = &shape.kind {
            let from = start
                .binding
                .as_ref()
                .and_then(|b| ids.get(&b.shape_id))
                .ok_or_else(|| invalid("free arrows cannot round-trip as Mermaid edges."))?;
            let to = end
                .binding
                .as_ref()
                .and_then(|b| ids.get(&b.shape_id))
                .ok_or_else(|| invalid("free arrows cannot round-trip as Mermaid edges."))?;
            lines.push(if shape.text.is_empty(){format!("    {from} --> {to}")}else{if shape.text.contains('|'){return Err(invalid("an edge label containing a pipe cannot be exported without changing its text."))}format!("    {from} -->|{}| {to}",quoted(&shape.text)?)});
        }
    }
    Ok(lines.join("\n"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flowchart_labels_edges_and_ellipses_round_trip() {
        let board = import_mermaid(
            "flowchart LR\n A[\"API\"] -->|\"calls\"| B(\"Database\"); B --> C[\"Store\"]",
            "/repo".into(),
            "Design".into(),
            0,
        )
        .unwrap();
        assert_eq!(board.shapes.len(), 5);
        let output = export_mermaid(&board).unwrap();
        let copy = import_mermaid(&output, "/repo".into(), "Copy".into(), 0).unwrap();
        assert_eq!(copy.shapes.len(), 5);
        assert_eq!(
            copy.shapes
                .values()
                .filter(|s| matches!(s.kind, ShapeKind::Ellipse))
                .count(),
            1
        );
        assert!(copy.shapes.values().any(|s| s.text == "calls"));
        assert!(import_mermaid(
            "flowchart LR\n click A javascript:alert(1)",
            "/repo".into(),
            "Bad".into(),
            0
        )
        .is_err());
    }
    #[test]
    fn sequence_is_editable_and_preserves_participants_message_order_and_dash() {
        let board = import_mermaid(
            "sequenceDiagram\nparticipant A as App\nactor U as User\nU->>A: Ask\nA-->>U: Answer",
            "/repo".into(),
            "Sequence".into(),
            0,
        )
        .unwrap();
        assert_eq!(board.shapes.len(), 4);
        let text = export_mermaid(&board).unwrap();
        assert!(text.contains("A-->>U: Answer"));
        let copy = import_mermaid(&text, "/repo".into(), "Copy".into(), 0).unwrap();
        assert_eq!(copy.shapes.len(), 4);
    }
}
