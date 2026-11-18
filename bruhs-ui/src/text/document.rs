//! A parsed rich-text document: blocks of styled inline text.
//!
//! Every run of inline text (a paragraph, a heading, a list item's text, a
//! table cell, a code block) is a *segment*, numbered in document order.
//! Rendering walks the blocks in the same order, so a selection can name a
//! position as (segment, byte offset).

use std::ops::Range;

use gpui::SharedString;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// A link in the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub url: SharedString,
    pub title: SharedString,
}

/// How a span of inline text is drawn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub link: Option<Link>,
}

/// A run of text with spans of [`InlineStyle`]. Spans are contiguous and
/// cover the whole text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub segment: usize,
    pub text: String,
    pub spans: Vec<(Range<usize>, InlineStyle)>,
}

impl Inline {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The link at byte `ix`, if any.
    pub fn link_at(&self, ix: usize) -> Option<&Link> {
        self.spans
            .iter()
            .find(|(range, _)| range.contains(&ix))
            .and_then(|(_, style)| style.link.as_ref())
    }

    fn push(&mut self, text: &str, style: &InlineStyle) {
        if text.is_empty() {
            return;
        }
        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        match self.spans.last_mut() {
            Some((range, last)) if last == style && range.end == start => range.end = end,
            _ => self.spans.push((start..end, style.clone())),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnAlign {
    Start,
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// `Some(checked)` for a task list item.
    pub task: Option<bool>,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Inline),
    Heading(u8, Inline),
    CodeBlock {
        language: Option<SharedString>,
        code: Inline,
    },
    List {
        /// The first number of an ordered list.
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    Table {
        aligns: Vec<ColumnAlign>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    },
    Rule,
}

/// A whole parsed document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
    /// Each segment's text, by segment number.
    pub segments: Vec<String>,
}

impl Document {
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

/// Builds blocks from a stream of block and inline events.
#[derive(Default)]
struct Builder {
    /// Open containers, innermost last; each collects its child blocks.
    stack: Vec<Container>,
    blocks: Vec<Block>,
    inline: Option<Inline>,
    styles: Vec<InlineStyle>,
    segments: Vec<String>,
    code: Option<(Option<SharedString>, String)>,
    table: Option<TableBuilder>,
}

enum Container {
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Item {
        task: Option<bool>,
        blocks: Vec<Block>,
    },
    Heading(u8),
}

#[derive(Default)]
struct TableBuilder {
    aligns: Vec<ColumnAlign>,
    head: Vec<Inline>,
    rows: Vec<Vec<Inline>>,
    in_head: bool,
}

impl Builder {
    fn style(&self) -> InlineStyle {
        self.styles.last().cloned().unwrap_or_default()
    }

    fn push_style(&mut self, f: impl FnOnce(&mut InlineStyle)) {
        let mut style = self.style();
        f(&mut style);
        self.styles.push(style);
    }

    fn inline(&mut self) -> &mut Inline {
        self.inline.get_or_insert_with(Inline::default)
    }

    fn text(&mut self, text: &str) {
        if let Some((_, code)) = self.code.as_mut() {
            code.push_str(text);
            return;
        }
        let style = self.style();
        self.inline().push(text, &style);
    }

    fn numbered(&mut self, mut inline: Inline) -> Inline {
        inline.segment = self.segments.len();
        self.segments.push(inline.text.clone());
        inline
    }

    /// Close the open inline text into a paragraph of the innermost
    /// container (a tight list item's text has no paragraph of its own).
    fn flush_paragraph(&mut self) {
        if let Some(inline) = self.inline.take() {
            if !inline.is_empty() {
                let inline = self.numbered(inline);
                self.push_block(Block::Paragraph(inline));
            }
        }
    }

    fn push_block(&mut self, block: Block) {
        match self.stack.last_mut() {
            Some(Container::Quote(blocks)) | Some(Container::Item { blocks, .. }) => {
                blocks.push(block);
            }
            Some(Container::List { items, .. }) => {
                // Content directly in a list belongs to its last item.
                if let Some(item) = items.last_mut() {
                    item.blocks.push(block);
                }
            }
            Some(Container::Heading(_)) | None => self.blocks.push(block),
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.flush_paragraph(),
            Tag::Heading { level, .. } => {
                self.flush_paragraph();
                self.stack.push(Container::Heading(heading_level(level)));
            }
            Tag::BlockQuote(_) => {
                self.flush_paragraph();
                self.stack.push(Container::Quote(Vec::new()));
            }
            Tag::CodeBlock(kind) => {
                self.flush_paragraph();
                let language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|lang| !lang.is_empty())
                        .map(|lang| SharedString::from(lang.to_string())),
                    CodeBlockKind::Indented => None,
                };
                self.code = Some((language, String::new()));
            }
            Tag::List(start) => {
                self.flush_paragraph();
                self.stack.push(Container::List {
                    start,
                    items: Vec::new(),
                });
            }
            Tag::Item => {
                self.flush_paragraph();
                self.stack.push(Container::Item {
                    task: None,
                    blocks: Vec::new(),
                });
            }
            Tag::Table(aligns) => {
                self.flush_paragraph();
                self.table = Some(TableBuilder {
                    aligns: aligns
                        .into_iter()
                        .map(|align| match align {
                            Alignment::Center => ColumnAlign::Center,
                            Alignment::Right => ColumnAlign::End,
                            Alignment::Left | Alignment::None => ColumnAlign::Start,
                        })
                        .collect(),
                    ..TableBuilder::default()
                });
            }
            Tag::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = true;
                }
            }
            Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.inline = Some(Inline::default()),
            Tag::Emphasis => self.push_style(|style| style.emphasis = true),
            Tag::Strong => self.push_style(|style| style.strong = true),
            Tag::Strikethrough => self.push_style(|style| style.strikethrough = true),
            Tag::Link {
                dest_url, title, ..
            } => {
                let link = Link {
                    url: dest_url.to_string().into(),
                    title: title.to_string().into(),
                };
                self.push_style(|style| style.link = Some(link));
            }
            // An image shows its alt text, like a browser that can't load it.
            Tag::Image { .. } => self.push_style(|_| {}),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_paragraph(),
            TagEnd::Heading(_) => {
                let level = match self.stack.pop() {
                    Some(Container::Heading(level)) => level,
                    _ => 1,
                };
                let inline = self.inline.take().unwrap_or_default();
                let inline = self.numbered(inline);
                self.push_block(Block::Heading(level, inline));
            }
            TagEnd::BlockQuote(_) => {
                self.flush_paragraph();
                if let Some(Container::Quote(blocks)) = self.stack.pop() {
                    self.push_block(Block::Quote(blocks));
                }
            }
            TagEnd::CodeBlock => {
                if let Some((language, mut code)) = self.code.take() {
                    if code.ends_with('\n') {
                        code.pop();
                    }
                    let mut inline = Inline::default();
                    inline.push(&code, &InlineStyle::default());
                    let code = self.numbered(inline);
                    self.push_block(Block::CodeBlock { language, code });
                }
            }
            TagEnd::List(_) => {
                self.flush_paragraph();
                if let Some(Container::List { start, items }) = self.stack.pop() {
                    self.push_block(Block::List { start, items });
                }
            }
            TagEnd::Item => {
                self.flush_paragraph();
                if let Some(Container::Item { task, blocks }) = self.stack.pop() {
                    if let Some(Container::List { items, .. }) = self.stack.last_mut() {
                        items.push(ListItem { task, blocks });
                    }
                }
            }
            TagEnd::TableCell => {
                let inline = self.inline.take().unwrap_or_default();
                let inline = self.numbered(inline);
                if let Some(table) = self.table.as_mut() {
                    if table.in_head {
                        table.head.push(inline);
                    } else if let Some(row) = table.rows.last_mut() {
                        row.push(inline);
                    }
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.in_head = false;
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    self.push_block(Block::Table {
                        aligns: table.aligns,
                        head: table.head,
                        rows: table
                            .rows
                            .into_iter()
                            .filter(|row| !row.is_empty())
                            .collect(),
                    });
                }
            }
            TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Strikethrough
            | TagEnd::Link
            | TagEnd::Image => {
                self.styles.pop();
            }
            _ => {}
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => {
                let mut style = self.style();
                style.code = true;
                self.inline().push(&code, &style);
            }
            // Raw HTML and math show as their source text.
            Event::Html(html) | Event::InlineHtml(html) => self.text(&html),
            Event::InlineMath(math) | Event::DisplayMath(math) => self.text(&math),
            Event::SoftBreak => self.text(" "),
            Event::HardBreak => self.text("\n"),
            Event::Rule => {
                self.flush_paragraph();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(checked) => {
                if let Some(Container::Item { task, .. }) = self.stack.last_mut() {
                    *task = Some(checked);
                }
            }
            Event::FootnoteReference(name) => self.text(&format!("[{name}]")),
        }
    }

    fn finish(mut self) -> Document {
        self.flush_paragraph();
        // Close anything an unfinished stream left open.
        while let Some(container) = self.stack.pop() {
            match container {
                Container::Quote(blocks) => self.push_block(Block::Quote(blocks)),
                Container::List { start, items } => self.push_block(Block::List { start, items }),
                Container::Item { task, blocks } => {
                    if let Some(Container::List { items, .. }) = self.stack.last_mut() {
                        items.push(ListItem { task, blocks });
                    }
                }
                Container::Heading(_) => {}
            }
        }
        Document {
            blocks: self.blocks,
            segments: self.segments,
        }
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Parse CommonMark with GitHub tables, strikethrough and task lists.
pub fn parse_markdown(source: &str) -> Document {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder::default();
    for event in Parser::new_ext(source, options) {
        builder.event(event);
    }
    builder.finish()
}

/// Parse the HTML subset rich text uses: paragraphs, headings, lists,
/// quotes, `pre`, `hr`, `br`, and `strong`/`b`, `em`/`i`, `del`/`s`, `code`
/// and `a href`. Other tags are dropped and their text kept.
pub fn parse_html(source: &str) -> Document {
    let mut builder = Builder::default();
    let mut rest = source;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            builder.event(Event::Text(decode_entities(rest).into()));
            break;
        };
        if open > 0 {
            let text = decode_entities(&rest[..open]);
            // Whitespace between blocks is layout, not text.
            if builder.code.is_some() || builder.inline.is_some() || !text.trim().is_empty() {
                builder.event(Event::Text(text.into()));
            }
        }
        let Some(close) = rest[open..].find('>') else {
            builder.event(Event::Text(decode_entities(&rest[open..]).into()));
            break;
        };
        let tag = &rest[open + 1..open + close];
        rest = &rest[open + close + 1..];
        html_tag(&mut builder, tag);
    }
    builder.finish()
}

fn html_tag(builder: &mut Builder, tag: &str) {
    let closing = tag.starts_with('/');
    let body = tag.trim_start_matches('/').trim_end_matches('/').trim();
    let name = body
        .split(|c: char| c.is_whitespace())
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let event = |start: Tag<'static>, end: TagEnd| {
        if closing {
            Event::End(end)
        } else {
            Event::Start(start)
        }
    };
    let heading = |level| {
        event(
            Tag::Heading {
                level,
                id: None,
                classes: Vec::new(),
                attrs: Vec::new(),
            },
            TagEnd::Heading(level),
        )
    };
    let event = match name.as_str() {
        "p" | "div" => event(Tag::Paragraph, TagEnd::Paragraph),
        "h1" => heading(HeadingLevel::H1),
        "h2" => heading(HeadingLevel::H2),
        "h3" => heading(HeadingLevel::H3),
        "h4" => heading(HeadingLevel::H4),
        "h5" => heading(HeadingLevel::H5),
        "h6" => heading(HeadingLevel::H6),
        "strong" | "b" => event(Tag::Strong, TagEnd::Strong),
        "em" | "i" => event(Tag::Emphasis, TagEnd::Emphasis),
        "del" | "s" => event(Tag::Strikethrough, TagEnd::Strikethrough),
        "blockquote" => event(Tag::BlockQuote(None), TagEnd::BlockQuote(None)),
        "ul" => event(Tag::List(None), TagEnd::List(false)),
        "ol" => event(Tag::List(Some(1)), TagEnd::List(true)),
        "li" => event(Tag::Item, TagEnd::Item),
        "pre" => event(
            Tag::CodeBlock(CodeBlockKind::Fenced("".into())),
            TagEnd::CodeBlock,
        ),
        "a" => {
            let url = attribute(body, "href").unwrap_or_default();
            event(
                Tag::Link {
                    link_type: pulldown_cmark::LinkType::Inline,
                    dest_url: url.into(),
                    title: "".into(),
                    id: "".into(),
                },
                TagEnd::Link,
            )
        }
        "br" => Event::HardBreak,
        "hr" => Event::Rule,
        "code" if builder.code.is_none() => {
            // Inline code: the text up to `</code>` arrives as text events,
            // so mark it with the code style.
            if closing {
                builder.styles.pop();
            } else {
                builder.push_style(|style| style.code = true);
            }
            return;
        }
        _ => return,
    };
    builder.event(event);
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let value = &tag[at..];
    let (quote, value) = match value.chars().next()? {
        q @ ('"' | '\'') => (Some(q), &value[1..]),
        _ => (None, value),
    };
    let end = match quote {
        Some(q) => value.find(q)?,
        None => value.find(char::is_whitespace).unwrap_or(value.len()),
    };
    Some(decode_entities(&value[..end]))
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", "\u{a0}")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(doc: &Document) -> Vec<&'static str> {
        doc.blocks
            .iter()
            .map(|block| match block {
                Block::Paragraph(_) => "p",
                Block::Heading(..) => "h",
                Block::CodeBlock { .. } => "code",
                Block::List { .. } => "list",
                Block::Quote(_) => "quote",
                Block::Table { .. } => "table",
                Block::Rule => "hr",
            })
            .collect()
    }

    #[test]
    fn markdown_blocks_and_inline_styles() {
        let doc = parse_markdown(
            "# Title\n\nSome **bold** and *it* with `code` and [a link](https://x.dev).\n\n```rust\nfn main() {}\n```\n\n- one\n- [x] two\n\n> quoted\n\n---\n\n| a | b |\n|---|:-:|\n| 1 | 2 |\n",
        );
        assert_eq!(
            kinds(&doc),
            ["h", "p", "code", "list", "quote", "hr", "table"]
        );
        let Block::Paragraph(p) = &doc.blocks[1] else {
            panic!()
        };
        assert_eq!(p.text, "Some bold and it with code and a link.");
        let bold = p.spans.iter().find(|(_, s)| s.strong).unwrap();
        assert_eq!(&p.text[bold.0.clone()], "bold");
        assert_eq!(
            p.link_at(p.text.find("a link").unwrap())
                .unwrap()
                .url
                .as_ref(),
            "https://x.dev"
        );
        let Block::CodeBlock { language, code } = &doc.blocks[2] else {
            panic!()
        };
        assert_eq!(language.as_deref(), Some("rust"));
        assert_eq!(code.text, "fn main() {}");
        let Block::List { items, .. } = &doc.blocks[3] else {
            panic!()
        };
        assert_eq!(items[1].task, Some(true));
        let Block::Table { aligns, head, rows } = &doc.blocks[6] else {
            panic!()
        };
        assert_eq!(aligns, &[ColumnAlign::Start, ColumnAlign::Center]);
        assert_eq!((head.len(), rows.len()), (2, 1));
    }

    #[test]
    fn segments_are_numbered_in_document_order() {
        let doc = parse_markdown("a\n\n- b\n- c\n\n```\nd\n```\n");
        assert_eq!(doc.segments, ["a", "b", "c", "d"]);
    }

    #[test]
    fn an_unfinished_stream_still_closes_its_blocks() {
        let doc = parse_markdown("- one\n- two\n\n```rust\nlet x");
        assert_eq!(kinds(&doc), ["list", "code"]);
    }

    #[test]
    fn html_subset() {
        let doc = parse_html(
            "<p>First <strong>bold</strong> &amp; <a href=\"/x\">link</a></p><ul><li>one</li></ul><pre>a &lt; b</pre>",
        );
        assert_eq!(kinds(&doc), ["p", "list", "code"]);
        let Block::Paragraph(p) = &doc.blocks[0] else {
            panic!()
        };
        assert_eq!(p.text, "First bold & link");
        let Block::CodeBlock { code, .. } = &doc.blocks[2] else {
            panic!()
        };
        assert_eq!(code.text, "a < b");
    }
}
