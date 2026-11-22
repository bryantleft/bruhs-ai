//! Editable Markdown keeps the original UTF-8 source for sending, undo, and IME.
//! A projection maps source ranges to their painted text and back for hit testing.
use super::code_fence::Fence;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use std::ops::Range;

/// A Markdown edit and the source selection to restore after applying it.
pub(super) struct MarkdownEdit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

impl MarkdownEdit {
    pub(super) fn insert(range: Range<usize>, text: String, caret: usize) -> Self {
        Self {
            range,
            text,
            selection: caret..caret,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Mark {
    Bold,
    Italic,
    Strike,
    Code,
}
impl Mark {
    pub(super) fn delimiter(self) -> &'static str {
        match self {
            Self::Bold => "**",
            Self::Italic => "*",
            Self::Strike => "~~",
            Self::Code => "`",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Inline {
    pub kind: Mark,
    pub outer: Range<usize>,
    pub inner: Range<usize>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Face {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub block: bool,
    pub muted: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Part {
    pub source: Range<usize>,
    pub drawn: Range<usize>,
    pub face: Face,
    replaced: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Projection {
    pub text: String,
    pub parts: Vec<Part>,
    pub marks: Vec<Inline>,
    pub tasks: Vec<Range<usize>>,
    pub fences: Vec<Fence>,
    pub list_lines: Vec<super::list_layout::ListLine>,
}
#[derive(Clone, Copy)]
enum Effect {
    Bold,
    Italic,
    Strike,
    Code,
    Muted,
    Hide,
    Block,
}
#[derive(Clone)]
struct Boundary {
    at: usize,
    effect: Effect,
    delta: i32,
}

/// Continue the containing Markdown block when Enter is a line break. Source
/// offsets from the parser exclude list-like text inside fenced code.
pub(super) fn block_newline(
    source: &str,
    selection: Range<usize>,
) -> Option<(Range<usize>, String)> {
    let cursor = selection.start;
    let line_start = source[..cursor].rfind('\n').map_or(0, |at| at + 1);
    let line_end = source[cursor..]
        .find('\n')
        .map_or(source.len(), |at| cursor + at);
    let mut items = Vec::new();
    let mut quote = false;
    for (event, range) in Parser::new_ext(source, Options::ENABLE_TASKLISTS).into_offset_iter() {
        if range.start > cursor {
            break;
        }
        if range.end < cursor || (range.end == cursor && cursor != source.len()) {
            continue;
        }
        match event {
            Event::Start(Tag::CodeBlock(_)) => return None,
            Event::Start(Tag::BlockQuote(_)) => quote = true,
            Event::Start(Tag::Item) => items.push(range),
            _ => {}
        }
    }
    let line = &source[line_start..line_end];
    // CommonMark can treat an empty nested marker as paragraph text or a
    // setext underline until the user types its body. The editor still needs
    // to lift that item when Return is pressed at the end of the marker.
    if let Some(parent) = items.last().filter(|r| r.start < line_start) {
        let marker = line_start + line.len() - line.trim_start_matches([' ', '\t']).len();
        if let Some((_, body_start)) = next_item(source, marker) {
            if marker > line_start && body_start == line_end && cursor == line_end {
                let (parent_prefix, _) = next_item(source, parent.start)?;
                return Some((line_start..selection.end.max(line_end), parent_prefix));
            }
        }
    }
    if let Some(item_range) = items.last() {
        let item = item_range.start;
        let (next, body_start) = next_item(source, item)?;
        let has_children =
            item_range.end > line_end && !source[line_end..item_range.end].trim().is_empty();
        if cursor < body_start {
            return None;
        }
        if !has_children
            && body_start >= line_start
            && source[body_start..line_end].trim().is_empty()
        {
            let parent = items
                .iter()
                .rev()
                .nth(1)
                .and_then(|range| next_item(source, range.start));
            let prefix = parent.map_or_else(
                || {
                    if quote {
                        source[line_start..item].trim_end().to_owned() + " "
                    } else {
                        String::new()
                    }
                },
                |(marker, _)| marker,
            );
            return Some((line_start..selection.end.max(line_end), prefix));
        }
        return Some((selection, format!("\n{next}")));
    }
    if quote {
        let mut prefix = 0;
        for (offset, byte) in line.bytes().enumerate() {
            if byte == b'>' {
                prefix = offset + 1;
            } else if byte != b' ' {
                break;
            }
        }
        if prefix > 0 {
            if cursor < line_start + prefix {
                return None;
            }
            if line[prefix..].trim().is_empty() {
                let outer = line[..prefix - 1].trim_end();
                let replacement = if outer.is_empty() {
                    "\n".into()
                } else {
                    format!("{outer} ")
                };
                return Some((line_start..selection.end.max(line_end), replacement));
            }
            return Some((selection, format!("\n{} ", &line[..prefix])));
        }
    }
    None
}

fn next_item(source: &str, item: usize) -> Option<(String, usize)> {
    let start = source[..item].rfind('\n').map_or(0, |at| at + 1);
    let marker = source[item..].split('\n').next()?;
    let digits = marker.bytes().take_while(u8::is_ascii_digit).count();
    let symbol = marker.as_bytes().get(digits).copied()?;
    let next = match (digits, symbol) {
        (0, b'-' | b'+' | b'*') => (symbol as char).to_string(),
        (1..=9, b'.' | b')') => format!(
            "{:0digits$}{}",
            marker[..digits].parse::<u32>().ok()? + 1,
            symbol as char
        ),
        _ => return None,
    };
    let rest = &marker[digits + 1..];
    let body = rest.trim_start_matches([' ', '\t']);
    let gap = &rest[..rest.len() - body.len()];
    let mut next = format!(
        "{}{next}{}",
        &source[start..item],
        if gap.is_empty() { " " } else { gap }
    );
    let mut body_start = item + digits + 1 + gap.len();
    if let Some(after) = ["[ ]", "[x]", "[X]"]
        .iter()
        .find_map(|task| body.strip_prefix(task))
    {
        if after.is_empty() || after.starts_with([' ', '\t']) {
            next.push_str("[ ] ");
            body_start += 3 + after.len() - after.trim_start_matches([' ', '\t']).len();
        }
    }
    Some((next, body_start))
}

impl Projection {
    pub fn new(source: &str, selection: Option<Range<usize>>) -> Self {
        let mut boundaries = Vec::new();
        let mut replacements = Vec::<(Range<usize>, &str)>::new();
        let mut marks = Vec::new();
        let mut tasks = Vec::new();
        let mut fences = Vec::new();
        let mut code_lines = Vec::new();
        let mut add = |range: Range<usize>, effect| {
            if !range.is_empty() {
                boundaries.push(Boundary {
                    at: range.start,
                    effect,
                    delta: 1,
                });
                boundaries.push(Boundary {
                    at: range.end,
                    effect,
                    delta: -1,
                });
            }
        };
        let active = |r: &Range<usize>| {
            selection.as_ref().is_some_and(|s| {
                if s.is_empty() {
                    r.start <= s.start && s.start <= r.end
                } else {
                    s.start < r.end && r.start < s.end
                }
            })
        };
        let mut lists = super::list_layout::ListLayout::default();
        let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
        for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
            lists.observe(source, &event, &range);
            let raw = &source[range.clone()];
            let mark = match event {
                Event::Start(Tag::Strong) => Some((Mark::Bold, 2)),
                Event::Start(Tag::Emphasis) => Some((Mark::Italic, 1)),
                Event::Start(Tag::Strikethrough) => Some((
                    Mark::Strike,
                    raw.bytes().take_while(|b| *b == b'~').count().min(2),
                )),
                Event::Code(_) => {
                    Some((Mark::Code, raw.bytes().take_while(|b| *b == b'`').count()))
                }
                Event::Start(Tag::Heading { .. }) => {
                    let prefix = raw.bytes().take_while(|b| *b == b'#').count();
                    let prefix = if prefix > 0 {
                        prefix
                            + raw[prefix..]
                                .bytes()
                                .take_while(u8::is_ascii_whitespace)
                                .count()
                    } else {
                        0
                    };
                    add(range.start..range.start + prefix, Effect::Hide);
                    add(range.start + prefix..range.end, Effect::Bold);
                    None
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    for offset in
                        std::iter::once(0).chain(raw.match_indices('\n').map(|(at, _)| at + 1))
                    {
                        let row = &raw[offset..];
                        let start = offset + row.len() - row.trim_start_matches(' ').len();
                        if raw[start..].starts_with('>') {
                            replacements.push((range.start + start..range.start + start + 1, "│"));
                        }
                    }
                    None
                }
                Event::Start(Tag::Item) => {
                    if matches!(raw.as_bytes().first(), Some(b'-' | b'+' | b'*'))
                        && raw.as_bytes().get(1).is_some_and(u8::is_ascii_whitespace)
                    {
                        if ["[ ]", "[x]", "[X]"]
                            .iter()
                            .any(|t| raw[1..].trim_start().starts_with(t))
                        {
                            add(
                                range.start
                                    ..range.start
                                        + 1
                                        + (raw[1..].len() - raw[1..].trim_start().len()),
                                Effect::Hide,
                            );
                        } else {
                            let bullet = match lists.depth() {
                                1 => "•",
                                2 => "◦",
                                _ => "▪",
                            };
                            replacements.push((range.start..range.start + 1, bullet));
                        }
                    }
                    None
                }
                Event::TaskListMarker(checked) => {
                    tasks.push(range.clone());
                    replacements.push((range.clone(), if checked { "☑" } else { "☐" }));
                    None
                }
                Event::Rule => {
                    let n = raw.trim_end().len();
                    replacements.push((range.start..range.start + n, "────────"));
                    None
                }
                Event::Start(Tag::CodeBlock(block)) => {
                    add(range.clone(), Effect::Code);
                    if matches!(block, CodeBlockKind::Fenced(_)) {
                        add(range.clone(), Effect::Block);
                    }
                    if let CodeBlockKind::Fenced(_) = block {
                        if let Some(fence) = Fence::parse(source, range.clone()) {
                            // The two visual lines reserve space for the native language button.
                            replacements.push((fence.header.clone(), " \n "));
                            add(fence.closing.clone(), Effect::Hide);
                            let mut line_start = fence.body.start;
                            for line in source[fence.body.clone()].split_inclusive('\n') {
                                code_lines.push(line_start);
                                line_start += line.len();
                            }
                            fences.push(fence);
                        }
                    }
                    None
                }
                _ => None,
            };
            if let Some((kind, width)) = mark {
                if width == 0 || raw.len() <= 2 * width {
                    continue;
                }
                let inner = range.start + width..range.end - width;
                let effect = match kind {
                    Mark::Bold => Effect::Bold,
                    Mark::Italic => Effect::Italic,
                    Mark::Strike => Effect::Strike,
                    Mark::Code => Effect::Code,
                };
                let marker = if active(&range) {
                    Effect::Muted
                } else {
                    Effect::Hide
                };
                add(range.start..inner.start, marker);
                add(inner.clone(), effect);
                add(inner.end..range.end, marker);
                marks.push(Inline {
                    kind,
                    outer: range,
                    inner,
                });
            }
        }
        for (range, _) in &replacements {
            add(range.clone(), Effect::Muted);
        }
        boundaries.sort_by_key(|e| e.at);
        replacements.sort_by_key(|(r, _)| r.start);
        let mut parts = Vec::new();
        let mut text = String::with_capacity(source.len());
        let mut pos = 0;
        let mut levels = [0i32; 7];
        let mut changes = boundaries.into_iter().peekable();
        let mut replace = replacements.into_iter().peekable();
        let mut indents = code_lines.into_iter().peekable();
        while pos < source.len() {
            while changes.peek().is_some_and(|c| c.at == pos) {
                let c = changes.next().unwrap();
                levels[c.effect as usize] += c.delta;
            }
            if indents.next_if_eq(&pos).is_some() {
                let start = text.len();
                text.push_str("  ");
                parts.push(Part {
                    source: pos..pos,
                    drawn: start..text.len(),
                    face: Face {
                        code: true,
                        ..Face::default()
                    },
                    replaced: true,
                });
            }
            let end = changes
                .peek()
                .map_or(source.len(), |c| c.at)
                .min(indents.peek().copied().unwrap_or(source.len()));
            while replace.peek().is_some_and(|(r, _)| r.end <= pos) {
                replace.next();
            }
            let replacement = replace
                .peek()
                .filter(|(r, _)| r.start == pos && r.end == end)
                .map(|(_, v)| *v);
            let start = text.len();
            if levels[Effect::Hide as usize] == 0 {
                text.push_str(replacement.unwrap_or(&source[pos..end]));
            }
            let face = Face {
                bold: levels[Effect::Bold as usize] > 0,
                italic: levels[Effect::Italic as usize] > 0,
                strike: levels[Effect::Strike as usize] > 0,
                code: levels[Effect::Code as usize] > 0,
                block: levels[Effect::Block as usize] > 0,
                muted: levels[Effect::Muted as usize] > 0,
            };
            parts.push(Part {
                source: pos..end,
                drawn: start..text.len(),
                face,
                replaced: replacement.is_some(),
            });
            pos = end;
        }
        Self {
            text,
            parts,
            marks,
            tasks,
            fences,
            list_lines: lists.finish(),
        }
    }
    pub fn to_display(&self, offset: usize) -> usize {
        let index = self.parts.partition_point(|p| p.source.end <= offset);
        let Some(p) = self.parts.get(index) else {
            return self.text.len();
        };
        if p.replaced || p.drawn.is_empty() {
            p.drawn.start
        } else {
            p.drawn.start + offset - p.source.start
        }
    }
    pub fn to_value(&self, offset: usize) -> usize {
        let offset = offset.min(self.text.len());
        let index = self.parts.partition_point(|p| p.drawn.end <= offset);
        let Some(p) = self.parts.get(index) else {
            return self.parts.last().map_or(0, |p| p.source.end);
        };
        if p.replaced {
            if offset - p.drawn.start > p.drawn.len() / 2 {
                p.source.end
            } else {
                p.source.start
            }
        } else {
            p.source.start + offset - p.drawn.start
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Projection;
    #[test]
    fn newline_continues_and_exits_the_parsed_block() {
        for (source, expected) in [
            ("- one", "- one\n- "),
            ("3. numbered", "3. numbered\n4. "),
            ("- [x] first", "- [x] first\n- [ ] "),
            ("- first\n- [ ] ", "- first\n"),
            ("- first\n  - sub\n  - ", "- first\n  - sub\n- "),
            ("> quote", "> quote\n> "),
            ("> quote\n> ", "> quote\n\n"),
            ("- 猫", "- 猫\n- "),
        ] {
            let end = source.len();
            let (range, value) = super::block_newline(source, end..end).expect(source);
            let mut result = source.to_owned();
            result.replace_range(range, &value);
            assert_eq!(result, expected);
        }
        for source in ["plain", "# heading", "```md\n- code", "    - code"] {
            let end = source.len();
            assert!(super::block_newline(source, end..end).is_none(), "{source}");
        }
    }

    #[test]
    fn rich_source_reappears_at_selection_and_utf8_positions_round_trip() {
        let source = "**猫** and *italic* and ~~strike~~ and `code` end";
        let plain = Projection::new(source, None);
        assert_eq!(plain.text, "猫 and italic and strike and code end");
        assert!(plain
            .parts
            .iter()
            .any(|r| r.face.bold && &plain.text[r.drawn.clone()] == "猫"));
        for (at, _) in plain.text.char_indices() {
            assert_eq!(plain.to_display(plain.to_value(at)), at);
        }
        let editing = Projection::new(source, Some(2..2));
        assert_eq!(editing.text, "**猫** and italic and strike and code end");
        assert_eq!(Projection::new("**unfinished", None).text, "**unfinished");
        assert_eq!(Projection::new(r"\*literal\*", None).text, r"\*literal\*");
    }
    #[test]
    fn block_structure_and_fenced_literal_text() {
        let p = Projection::new(
            "# Heading\n\n- item\n- [x] done\n\n> quote\n\n```rust\n**literal**\n```",
            None,
        );
        assert!(p.text.starts_with("Heading\n\n• item\n☑ done\n\n│ quote"));
        assert!(p.text.contains("**literal**"));
        assert!(p
            .parts
            .iter()
            .any(|r| r.face.code && p.text[r.drawn.clone()].contains("**literal**")));
    }
}
