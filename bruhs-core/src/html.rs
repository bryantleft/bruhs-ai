//! Lean, arena-allocated HTML5 DOM parser, serializer, and single-pass
//! HTML-to-Markdown converter.
//!
//! Replaces `html5ever`, `markup5ever_rcdom`, `dom_query`, and `htmd` across
//! `bruhs-core` with one cache-friendly arena tree: zero `Rc`/`RefCell`
//! allocations, zero double-parsing in `website_reader`, and zero duplicate
//! `html5ever` versions in `Cargo.lock`.

use std::fmt::Write as _;

/// Index of a node inside a [`Document`] arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeId(pub(crate) usize);

#[derive(Clone, Debug)]
pub(crate) enum NodeKind {
    Document,
    Doctype(String),
    Comment(String),
    Text(String),
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) kind: NodeKind,
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    pub(crate) removed: bool,
}

/// Arena-backed HTML document.
#[derive(Clone, Debug)]
pub(crate) struct Document {
    nodes: Vec<Node>,
}

impl Document {
    pub(crate) const ROOT: NodeId = NodeId(0);

    /// Parse an HTML document into an arena tree.
    pub(crate) fn parse(input: &str) -> Self {
        let mut doc = Self {
            nodes: vec![Node {
                kind: NodeKind::Document,
                parent: None,
                children: Vec::new(),
                removed: false,
            }],
        };
        let mut stack = vec![Self::ROOT];
        let bytes = input.as_bytes();
        let len = bytes.len();
        let mut pos = 0;

        while pos < len {
            let Some(lt_rel) = input[pos..].find('<') else {
                doc.push_text(current_parent(&stack), &decode_entities(&input[pos..]));
                break;
            };
            let lt = pos + lt_rel;
            if lt > pos {
                doc.push_text(current_parent(&stack), &decode_entities(&input[pos..lt]));
            }

            // Comment `<!-- ... -->`
            if input[lt..].starts_with("<!--") {
                let end = input[lt + 4..]
                    .find("-->")
                    .map_or(len, |off| lt + 4 + off + 3);
                let body_end = if end >= lt + 7 && input[..end].ends_with("-->") {
                    end - 3
                } else {
                    end
                };
                doc.alloc_child(
                    current_parent(&stack),
                    NodeKind::Comment(input[lt + 4..body_end].to_string()),
                );
                pos = end;
                continue;
            }

            // DOCTYPE / bogus declaration `<!...>` or processing instruction `<?...>`
            if lt + 1 < len && matches!(bytes[lt + 1], b'!' | b'?') {
                let end = input[lt + 2..]
                    .find('>')
                    .map_or(len, |off| lt + 2 + off + 1);
                let inner_end = if end > lt + 2 && bytes[end - 1] == b'>' {
                    end - 1
                } else {
                    end
                };
                let inner = input[lt + 2..inner_end].trim();
                if inner.len() >= 7 && inner[..7].eq_ignore_ascii_case("doctype") {
                    doc.alloc_child(
                        current_parent(&stack),
                        NodeKind::Doctype(inner[7..].trim().to_string()),
                    );
                }
                pos = end;
                continue;
            }

            // Closing tag `</tag>`
            if lt + 1 < len && bytes[lt + 1] == b'/' {
                let name_start = lt + 2;
                let mut cursor = name_start;
                while cursor < len && !bytes[cursor].is_ascii_whitespace() && bytes[cursor] != b'>'
                {
                    cursor += 1;
                }
                let close_name = input[name_start..cursor].to_ascii_lowercase();
                let gt = input[cursor..]
                    .find('>')
                    .map_or(len, |off| cursor + off + 1);
                if !close_name.is_empty() {
                    if let Some(idx) = stack
                        .iter()
                        .rposition(|&id| doc.tag_name(id) == Some(close_name.as_str()))
                    {
                        if idx > 0 {
                            stack.truncate(idx);
                        }
                    }
                }
                pos = gt;
                continue;
            }

            // Opening tag `<tag ...>`
            let name_start = lt + 1;
            if name_start >= len || !bytes[name_start].is_ascii_alphabetic() {
                // Literal `<` in text
                doc.push_text(current_parent(&stack), "<");
                pos = lt + 1;
                continue;
            }
            let mut cursor = name_start;
            while cursor < len
                && (bytes[cursor].is_ascii_alphanumeric()
                    || matches!(bytes[cursor], b'-' | b'_' | b':'))
            {
                cursor += 1;
            }
            let tag = input[name_start..cursor].to_ascii_lowercase();
            let (attrs, self_closing, next_pos) = parse_attributes(input, cursor);
            pos = next_pos;

            // Handle common HTML5 implicit end tags before inserting.
            doc.apply_implicit_closes(&mut stack, &tag);

            let node_id = doc.alloc_child(
                current_parent(&stack),
                NodeKind::Element {
                    tag: tag.clone(),
                    attrs,
                },
            );

            if is_void_element(&tag) || (self_closing && !is_rawtext_or_rcdata(&tag)) {
                continue;
            }

            // Rawtext (`<script>`, `<style>`) and RCDATA (`<title>`, `<textarea>`)
            if is_rawtext_or_rcdata(&tag) {
                let raw_start = pos;
                let (raw_end, after_close) = find_rawtext_close(input, raw_start, &tag);
                let raw_slice = &input[raw_start..raw_end];
                let content = if matches!(tag.as_str(), "title" | "textarea") {
                    decode_entities(raw_slice)
                } else {
                    raw_slice.to_string()
                };
                if !content.is_empty() {
                    doc.alloc_child(node_id, NodeKind::Text(content));
                }
                pos = after_close;
                continue;
            }

            stack.push(node_id);
        }

        doc
    }

    fn alloc_child(&mut self, parent: NodeId, kind: NodeKind) -> NodeId {
        let id = NodeId(self.nodes.len());
        self.nodes.push(Node {
            kind,
            parent: Some(parent),
            children: Vec::new(),
            removed: false,
        });
        self.nodes[parent.0].children.push(id);
        id
    }

    fn push_text(&mut self, parent: NodeId, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(&last_id) = self.nodes[parent.0].children.last() {
            if let NodeKind::Text(existing) = &mut self.nodes[last_id.0].kind {
                existing.push_str(text);
                return;
            }
        }
        self.alloc_child(parent, NodeKind::Text(text.to_string()));
    }

    fn apply_implicit_closes(&self, stack: &mut Vec<NodeId>, new_tag: &str) {
        // `<p>` closes automatically when a block-level tag opens.
        if is_p_closing_tag(new_tag) {
            if let Some(pos) = stack.iter().rposition(|&id| self.tag_name(id) == Some("p")) {
                if pos > 0
                    && !stack[pos + 1..]
                        .iter()
                        .any(|&id| is_scope_boundary(self.tag_name(id)))
                {
                    stack.truncate(pos);
                }
            }
        }
        // `<li>` closes an open sibling `<li>` in the same list.
        if new_tag == "li" {
            close_sibling_in_scope(self, stack, &["li"], &["ul", "ol", "menu"]);
        } else if matches!(new_tag, "dt" | "dd") {
            close_sibling_in_scope(self, stack, &["dt", "dd"], &["dl"]);
        } else if matches!(new_tag, "td" | "th") {
            close_sibling_in_scope(self, stack, &["td", "th"], &["tr", "table"]);
        } else if new_tag == "tr" {
            close_sibling_in_scope(
                self,
                stack,
                &["tr", "td", "th"],
                &["table", "tbody", "thead", "tfoot"],
            );
        } else if new_tag == "option" {
            close_sibling_in_scope(self, stack, &["option"], &["select", "optgroup"]);
        } else if new_tag == "body" {
            if let Some(pos) = stack
                .iter()
                .rposition(|&id| self.tag_name(id) == Some("head"))
            {
                if pos > 0 {
                    stack.truncate(pos);
                }
            }
        }
    }

    pub(crate) fn tag_name(&self, id: NodeId) -> Option<&str> {
        match &self.nodes[id.0].kind {
            NodeKind::Element { tag, .. } => Some(tag.as_str()),
            _ => None,
        }
    }

    pub(crate) fn attr(&self, id: NodeId, key: &str) -> Option<&str> {
        match &self.nodes[id.0].kind {
            NodeKind::Element { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.as_str()),
            _ => None,
        }
    }

    pub(crate) fn has_attr(&self, id: NodeId, key: &str) -> bool {
        self.attr(id, key).is_some()
    }

    pub(crate) fn set_attr(&mut self, id: NodeId, key: &str, value: String) {
        if let NodeKind::Element { attrs, .. } = &mut self.nodes[id.0].kind {
            if let Some((_, existing)) = attrs.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key))
            {
                *existing = value;
            } else {
                attrs.push((key.to_ascii_lowercase(), value));
            }
        }
    }

    pub(crate) fn remove_attr(&mut self, id: NodeId, key: &str) {
        if let NodeKind::Element { attrs, .. } = &mut self.nodes[id.0].kind {
            attrs.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        }
    }

    pub(crate) fn retain_attrs(&mut self, id: NodeId, mut keep: impl FnMut(&str) -> bool) {
        if let NodeKind::Element { attrs, .. } = &mut self.nodes[id.0].kind {
            attrs.retain(|(k, _)| keep(k.as_str()));
        }
    }

    pub(crate) fn text_content(&self, root: NodeId) -> String {
        let mut out = String::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id.0];
            if node.removed {
                continue;
            }
            if let NodeKind::Text(text) = &node.kind {
                out.push_str(text);
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        out
    }

    pub(crate) fn set_text(&mut self, id: NodeId, content: &str) {
        for &child in &self.nodes[id.0].children.clone() {
            self.nodes[child.0].removed = true;
        }
        self.nodes[id.0].children.clear();
        if !content.is_empty() {
            self.alloc_child(id, NodeKind::Text(content.to_string()));
        }
    }

    /// Replace an element (such as `<link rel=stylesheet>`) in-place with a
    /// `<style>` element preserving non-link attributes and setting CSS text.
    pub(crate) fn replace_with_style(&mut self, id: NodeId, css: &str) {
        if let NodeKind::Element { tag, attrs } = &mut self.nodes[id.0].kind {
            *tag = "style".to_string();
            attrs.retain(|(k, _)| {
                !matches!(k.as_str(), "href" | "rel" | "integrity" | "crossorigin")
            });
        }
        self.set_text(id, css);
    }

    /// Detach `child` from its current parent and append it to `new_parent`.
    pub(crate) fn move_to_end(&mut self, child: NodeId, new_parent: NodeId) {
        if let Some(old_parent) = self.nodes[child.0].parent {
            self.nodes[old_parent.0].children.retain(|&c| c != child);
        }
        self.nodes[child.0].parent = Some(new_parent);
        self.nodes[new_parent.0].children.push(child);
    }

    /// Mark a node (and its subtree by skipping removed nodes) as removed.
    pub(crate) fn remove(&mut self, id: NodeId) {
        self.nodes[id.0].removed = true;
        if let Some(parent) = self.nodes[id.0].parent {
            self.nodes[parent.0].children.retain(|&c| c != id);
        }
    }

    /// Return all active element nodes in pre-order (document order) under `root`.
    pub(crate) fn elements_in_order(&self, root: NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id.0];
            if node.removed {
                continue;
            }
            if matches!(node.kind, NodeKind::Element { .. }) {
                out.push(id);
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        out
    }

    /// Find the first active element under `root` matching `predicate` in document order.
    pub(crate) fn find_first(
        &self,
        root: NodeId,
        mut predicate: impl FnMut(&Self, NodeId) -> bool,
    ) -> Option<NodeId> {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id.0];
            if node.removed {
                continue;
            }
            if matches!(node.kind, NodeKind::Element { .. }) && predicate(self, id) {
                return Some(id);
            }
            for &child in node.children.iter().rev() {
                stack.push(child);
            }
        }
        None
    }

    /// Serialize the document back to valid HTML.
    pub(crate) fn serialize(&self) -> String {
        let mut out = String::new();
        self.serialize_node(Self::ROOT, &mut out);
        out
    }

    fn serialize_node(&self, id: NodeId, out: &mut String) {
        let node = &self.nodes[id.0];
        if node.removed {
            return;
        }
        match &node.kind {
            NodeKind::Document => {
                for &child in &node.children {
                    self.serialize_node(child, out);
                }
            }
            NodeKind::Doctype(dt) => {
                out.push_str("<!DOCTYPE ");
                if dt.is_empty() {
                    out.push_str("html");
                } else {
                    out.push_str(dt);
                }
                out.push('>');
            }
            NodeKind::Comment(c) => {
                out.push_str("<!--");
                out.push_str(c);
                out.push_str("-->");
            }
            NodeKind::Text(t) => {
                let raw = node
                    .parent
                    .and_then(|p| self.tag_name(p))
                    .is_some_and(|tag| matches!(tag, "script" | "style"));
                if raw {
                    out.push_str(t);
                } else {
                    push_escaped_text(out, t);
                }
            }
            NodeKind::Element { tag, attrs } => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in attrs {
                    out.push(' ');
                    out.push_str(k);
                    out.push_str("=\"");
                    push_escaped_attr(out, v);
                    out.push('"');
                }
                out.push('>');
                if is_void_element(tag) {
                    return;
                }
                for &child in &node.children {
                    self.serialize_node(child, out);
                }
                out.push_str("</");
                out.push_str(tag);
                out.push('>');
            }
        }
    }

    /// Convert a subtree directly into clean Markdown in a single pass.
    pub(crate) fn to_markdown(&self, root: NodeId) -> String {
        let mut ctx = MdWriter::default();
        self.render_md_node(root, &mut ctx);
        ctx.finish()
    }

    fn render_md_node(&self, id: NodeId, out: &mut MdWriter) {
        let node = &self.nodes[id.0];
        if node.removed {
            return;
        }
        match &node.kind {
            NodeKind::Document => {
                for &child in &node.children {
                    self.render_md_node(child, out);
                }
            }
            NodeKind::Doctype(_) | NodeKind::Comment(_) => {}
            NodeKind::Text(text) => {
                if out.in_pre {
                    out.buf.push_str(text);
                } else {
                    out.push_inline_text(text);
                }
            }
            NodeKind::Element { tag, .. } => match tag.as_str() {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    let level = usize::from(tag.as_bytes()[1].saturating_sub(b'0')).clamp(1, 6);
                    out.ensure_blank_line();
                    for _ in 0..level {
                        out.buf.push('#');
                    }
                    out.buf.push(' ');
                    let mut inner = MdWriter::default();
                    for &child in &node.children {
                        self.render_md_node(child, &mut inner);
                    }
                    let heading = inner.finish();
                    out.buf.push_str(heading.trim());
                    out.ensure_blank_line();
                }
                "p" | "div" | "section" | "article" | "main" | "header" | "figure"
                | "figcaption" => {
                    let is_para = tag == "p";
                    if is_para {
                        out.ensure_blank_line();
                    } else {
                        out.ensure_newline();
                    }
                    for &child in &node.children {
                        self.render_md_node(child, out);
                    }
                    if is_para {
                        out.ensure_blank_line();
                    } else {
                        out.ensure_newline();
                    }
                }
                "br" => {
                    out.buf.push('\n');
                }
                "hr" => {
                    out.ensure_blank_line();
                    out.buf.push_str("---");
                    out.ensure_blank_line();
                }
                "strong" | "b" => {
                    let text = self.render_inline_children(id);
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push_space_if_needed();
                        out.buf.push_str("**");
                        out.buf.push_str(trimmed);
                        out.buf.push_str("**");
                    }
                }
                "em" | "i" => {
                    let text = self.render_inline_children(id);
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.push_space_if_needed();
                        out.buf.push('*');
                        out.buf.push_str(trimmed);
                        out.buf.push('*');
                    }
                }
                "code" | "kbd" | "samp" => {
                    if out.in_pre {
                        for &child in &node.children {
                            self.render_md_node(child, out);
                        }
                    } else {
                        let raw = self.text_content(id);
                        if !raw.is_empty() {
                            let fence = if raw.contains('`') { "``" } else { "`" };
                            out.push_space_if_needed();
                            out.buf.push_str(fence);
                            if fence.len() > 1 && (raw.starts_with('`') || raw.ends_with('`')) {
                                out.buf.push(' ');
                                out.buf.push_str(&raw);
                                out.buf.push(' ');
                            } else {
                                out.buf.push_str(&raw);
                            }
                            out.buf.push_str(fence);
                        }
                    }
                }
                "pre" => {
                    let lang = self.detect_pre_language(id);
                    let code_text = self.text_content(id);
                    let trimmed = code_text.trim_matches('\n');
                    out.ensure_blank_line();
                    out.buf.push_str("```");
                    if let Some(lang) = lang {
                        out.buf.push_str(&lang);
                    }
                    out.buf.push('\n');
                    out.buf.push_str(trimmed);
                    out.buf.push_str("\n```");
                    out.ensure_blank_line();
                }
                "a" => {
                    let label = self.render_inline_children(id);
                    let label = label.trim();
                    if let Some(href) = self.attr(id, "href").filter(|h| !h.is_empty()) {
                        let text = if label.is_empty() { href } else { label };
                        out.push_space_if_needed();
                        let _ = write!(out.buf, "[{text}]({href})");
                    } else if !label.is_empty() {
                        out.push_space_if_needed();
                        out.buf.push_str(label);
                    }
                }
                "img" => {
                    let alt = self.attr(id, "alt").unwrap_or("").trim();
                    if !alt.is_empty() {
                        out.push_space_if_needed();
                        out.buf.push_str(alt);
                    }
                }
                "blockquote" => {
                    let mut inner = MdWriter::default();
                    for &child in &node.children {
                        self.render_md_node(child, &mut inner);
                    }
                    let body = inner.finish();
                    if !body.is_empty() {
                        out.ensure_blank_line();
                        for (idx, line) in body.lines().enumerate() {
                            if idx > 0 {
                                out.buf.push('\n');
                            }
                            out.buf.push_str("> ");
                            out.buf.push_str(line);
                        }
                        out.ensure_blank_line();
                    }
                }
                "ul" | "ol" => {
                    let ordered = tag == "ol";
                    let depth = out.list_depth;
                    if depth == 0 {
                        out.ensure_blank_line();
                    } else {
                        out.ensure_newline();
                    }
                    out.list_depth += 1;
                    let mut item_num = 1usize;
                    for &child in &node.children {
                        if self.tag_name(child) == Some("li") && !self.nodes[child.0].removed {
                            out.ensure_newline();
                            for _ in 0..depth {
                                out.buf.push_str("  ");
                            }
                            if ordered {
                                let _ = write!(out.buf, "{item_num}. ");
                                item_num += 1;
                            } else {
                                out.buf.push_str("- ");
                            }
                            let mut item_writer = MdWriter {
                                list_depth: out.list_depth,
                                ..MdWriter::default()
                            };
                            for &grandchild in &self.nodes[child.0].children {
                                self.render_md_node(grandchild, &mut item_writer);
                            }
                            let rendered = item_writer.finish();
                            out.buf.push_str(rendered.trim());
                            out.buf.push('\n');
                        }
                    }
                    out.list_depth = depth;
                    if depth == 0 {
                        out.ensure_blank_line();
                    }
                }
                "table" => {
                    self.render_md_table(id, out);
                }
                _ => {
                    for &child in &node.children {
                        self.render_md_node(child, out);
                    }
                }
            },
        }
    }

    fn render_inline_children(&self, id: NodeId) -> String {
        let mut inner = MdWriter::default();
        for &child in &self.nodes[id.0].children {
            self.render_md_node(child, &mut inner);
        }
        inner.finish()
    }

    fn detect_pre_language(&self, pre_id: NodeId) -> Option<String> {
        let mut candidates = vec![pre_id];
        for &child in &self.nodes[pre_id.0].children {
            if self.tag_name(child) == Some("code") {
                candidates.push(child);
            }
        }
        for id in candidates {
            if let Some(class) = self.attr(id, "class") {
                for token in class.split_whitespace() {
                    if let Some(lang) = token
                        .strip_prefix("language-")
                        .or_else(|| token.strip_prefix("lang-"))
                    {
                        let clean: String = lang
                            .chars()
                            .filter(|c| {
                                c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '+' | '#')
                            })
                            .collect();
                        if !clean.is_empty() {
                            return Some(clean);
                        }
                    }
                }
            }
        }
        None
    }

    fn render_md_table(&self, table_id: NodeId, out: &mut MdWriter) {
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut tr_nodes = Vec::new();
        self.collect_tags(table_id, "tr", &mut tr_nodes);
        for tr in tr_nodes {
            let mut cells = Vec::new();
            for &child in &self.nodes[tr.0].children {
                if matches!(self.tag_name(child), Some("th" | "td")) && !self.nodes[child.0].removed
                {
                    let cell = self
                        .render_inline_children(child)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ")
                        .replace('|', "\\|");
                    cells.push(cell);
                }
            }
            if !cells.is_empty() {
                rows.push(cells);
            }
        }
        if rows.is_empty() {
            return;
        }
        let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        out.ensure_blank_line();
        for (row_idx, row) in rows.iter().enumerate() {
            out.buf.push('|');
            for col in 0..cols {
                out.buf.push(' ');
                if let Some(cell) = row.get(col) {
                    out.buf.push_str(cell);
                }
                out.buf.push_str(" |");
            }
            out.buf.push('\n');
            if row_idx == 0 {
                out.buf.push('|');
                for _ in 0..cols {
                    out.buf.push_str(" --- |");
                }
                out.buf.push('\n');
            }
        }
        out.ensure_blank_line();
    }

    fn collect_tags(&self, root: NodeId, target: &str, out: &mut Vec<NodeId>) {
        let node = &self.nodes[root.0];
        if node.removed {
            return;
        }
        if self.tag_name(root) == Some(target) {
            out.push(root);
            return;
        }
        for &child in &node.children {
            self.collect_tags(child, target, out);
        }
    }
}

#[derive(Default)]
struct MdWriter {
    buf: String,
    in_pre: bool,
    list_depth: usize,
}

impl MdWriter {
    fn ensure_newline(&mut self) {
        if !self.buf.is_empty() && !self.buf.ends_with('\n') {
            self.buf.push('\n');
        }
    }

    fn ensure_blank_line(&mut self) {
        if self.buf.is_empty() {
            return;
        }
        if self.buf.ends_with("\n\n") {
            return;
        }
        if self.buf.ends_with('\n') {
            self.buf.push('\n');
        } else {
            self.buf.push_str("\n\n");
        }
    }

    fn push_space_if_needed(&mut self) {
        if let Some(last) = self.buf.chars().last() {
            if !last.is_whitespace() && !matches!(last, '(' | '[' | '{' | '>') {
                self.buf.push(' ');
            }
        }
    }

    fn push_inline_text(&mut self, text: &str) {
        let mut saw_space = false;
        for ch in text.chars() {
            if ch.is_whitespace() {
                saw_space = true;
            } else {
                if saw_space && !self.buf.is_empty() && !self.buf.ends_with([' ', '\n', '\t']) {
                    self.buf.push(' ');
                }
                saw_space = false;
                self.buf.push(ch);
            }
        }
        if saw_space && !self.buf.is_empty() && !self.buf.ends_with([' ', '\n', '\t']) {
            self.buf.push(' ');
        }
    }

    fn finish(self) -> String {
        let mut out = String::with_capacity(self.buf.len());
        let mut blank_run = 0usize;
        for line in self.buf.lines() {
            let trimmed_end = line.trim_end();
            if trimmed_end.is_empty() {
                blank_run += 1;
                if blank_run == 1 && !out.is_empty() {
                    out.push('\n');
                }
            } else {
                if !out.is_empty() {
                    out.push('\n');
                }
                blank_run = 0;
                out.push_str(trimmed_end);
            }
        }
        out
    }
}

fn current_parent(stack: &[NodeId]) -> NodeId {
    stack.last().copied().unwrap_or(Document::ROOT)
}

fn close_sibling_in_scope(
    doc: &Document,
    stack: &mut Vec<NodeId>,
    close_tags: &[&str],
    scopes: &[&str],
) {
    for idx in (1..stack.len()).rev() {
        let Some(tag) = doc.tag_name(stack[idx]) else {
            continue;
        };
        if close_tags.contains(&tag) {
            stack.truncate(idx);
            return;
        }
        if scopes.contains(&tag) {
            return;
        }
    }
}

fn is_scope_boundary(tag: Option<&str>) -> bool {
    matches!(
        tag,
        Some("table" | "td" | "th" | "template" | "html" | "body" | "article" | "section")
    )
}

fn is_p_closing_tag(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "details"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "hr"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "ul"
    )
}

fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

fn is_rawtext_or_rcdata(tag: &str) -> bool {
    matches!(tag, "script" | "style" | "title" | "textarea")
}

fn parse_attributes(input: &str, mut pos: usize) -> (Vec<(String, String)>, bool, usize) {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut attrs = Vec::new();
    let mut self_closing = false;

    while pos < len {
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= len {
            break;
        }
        if bytes[pos] == b'>' {
            pos += 1;
            break;
        }
        if bytes[pos] == b'/' {
            pos += 1;
            if pos < len && bytes[pos] == b'>' {
                self_closing = true;
                pos += 1;
                break;
            }
            continue;
        }
        let name_start = pos;
        while pos < len
            && !bytes[pos].is_ascii_whitespace()
            && !matches!(bytes[pos], b'=' | b'>' | b'/')
        {
            pos += 1;
        }
        if pos == name_start {
            pos += 1;
            continue;
        }
        let name = input[name_start..pos].to_ascii_lowercase();
        while pos < len && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        let value = if pos < len && bytes[pos] == b'=' {
            pos += 1;
            while pos < len && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            if pos < len && matches!(bytes[pos], b'"' | b'\'') {
                let quote = bytes[pos];
                pos += 1;
                let val_start = pos;
                while pos < len && bytes[pos] != quote {
                    pos += 1;
                }
                let raw = &input[val_start..pos];
                if pos < len {
                    pos += 1;
                }
                decode_entities(raw)
            } else {
                let val_start = pos;
                while pos < len && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' {
                    pos += 1;
                }
                decode_entities(&input[val_start..pos])
            }
        } else {
            String::new()
        };
        if !attrs.iter().any(|(k, _)| k == &name) {
            attrs.push((name, value));
        }
    }

    (attrs, self_closing, pos)
}

fn find_rawtext_close(input: &str, start: usize, tag: &str) -> (usize, usize) {
    let bytes = input.as_bytes();
    let tag_bytes = tag.as_bytes();
    let len = bytes.len();
    let mut pos = start;
    while pos + 2 + tag_bytes.len() <= len {
        let Some(rel) = input[pos..].find("</") else {
            return (len, len);
        };
        let idx = pos + rel;
        let name_start = idx + 2;
        let name_end = name_start + tag_bytes.len();
        if name_end <= len
            && bytes[name_start..name_end].eq_ignore_ascii_case(tag_bytes)
            && (name_end == len
                || bytes[name_end].is_ascii_whitespace()
                || matches!(bytes[name_end], b'>' | b'/'))
        {
            let after = input[name_end..]
                .find('>')
                .map_or(len, |off| name_end + off + 1);
            return (idx, after);
        }
        pos = idx + 2;
    }
    (len, len)
}

fn push_escaped_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

fn push_escaped_attr(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

/// Decode common HTML character references (`&amp;`, `&#123;`, `&#x1F;`, etc.).
pub(crate) fn decode_entities(input: &str) -> String {
    if !input.contains('&') {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some(semi_rel) = input[i + 1..].find(';') {
                if semi_rel > 0 && semi_rel <= 12 {
                    let entity = &input[i + 1..i + 1 + semi_rel];
                    if let Some(decoded) = decode_entity(entity) {
                        out.push_str(&decoded);
                        i += semi_rel + 2;
                        continue;
                    }
                }
            }
        }
        let ch = input[i..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn decode_entity(entity: &str) -> Option<String> {
    if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        let code = u32::from_str_radix(hex, 16).ok()?;
        let ch = char::from_u32(code)?;
        return Some(ch.to_string());
    }
    if let Some(dec) = entity.strip_prefix('#') {
        let code = dec.parse::<u32>().ok()?;
        let ch = char::from_u32(code)?;
        return Some(ch.to_string());
    }
    let s = match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{00A0}",
        "ndash" => "–",
        "mdash" => "—",
        "lsquo" => "‘",
        "rsquo" => "’",
        "ldquo" => "“",
        "rdquo" => "”",
        "hellip" => "…",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "times" => "×",
        "divide" => "÷",
        "plusmn" => "±",
        "rarr" => "→",
        "larr" => "←",
        _ => return None,
    };
    Some(s.to_string())
}
