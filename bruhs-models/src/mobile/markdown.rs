//! Presentation data for paired phones. Only text and explicit link actions
//! leave the host. Raw markup is never interpreted by either phone client.
use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileQuestionMarkdown {
    pub question: Option<MobileMarkdown>,
    pub previews: Vec<Option<MobileMarkdown>>,
}

pub type MobileMarkdown = Vec<MobileMarkdownNode>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MobileMarkdownNode {
    Text(String),
    Element {
        tag: MobileMarkdownTag,
        children: MobileMarkdown,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<MobileMarkdownLink>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<u64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileMarkdownTag {
    Paragraph,
    Heading,
    Quote,
    CodeBlock,
    OrderedList,
    List,
    Item,
    Table,
    Row,
    Cell,
    Strong,
    Emphasis,
    Strike,
    Code,
    Link,
    Plain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MobileMarkdownLink {
    Web { url: String },
    Project { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MobileProjectLink {
    pub path: String,
    pub line: Option<usize>,
}

const MAX_EVENTS: usize = 768;
const MAX_DEPTH: usize = 16;

fn events(input: &str) -> Parser<'_> {
    Parser::new_ext(
        input,
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES,
    )
}

pub fn mobile_question_markdown(input: &str) -> Option<MobileMarkdown> {
    use MobileMarkdownTag as M;
    let mut root = Vec::new();
    let mut stack = Vec::new();
    let mut styled = false;
    for (index, event) in events(input).enumerate() {
        if index >= MAX_EVENTS || stack.len() > MAX_DEPTH {
            return None;
        }
        let node = match event {
            Event::Start(tag) => {
                let start = if let Tag::List(start) = &tag {
                    *start
                } else {
                    None
                };
                let (tag, target) = match tag {
                    Tag::Paragraph => (M::Paragraph, None),
                    Tag::Heading { .. } => (M::Heading, None),
                    Tag::BlockQuote(_) => (M::Quote, None),
                    Tag::CodeBlock(_) => (M::CodeBlock, None),
                    Tag::List(Some(_)) => (M::OrderedList, None),
                    Tag::List(None) => (M::List, None),
                    Tag::Item => (M::Item, None),
                    Tag::Table(_) => (M::Table, None),
                    Tag::TableHead | Tag::TableRow => (M::Row, None),
                    Tag::TableCell => (M::Cell, None),
                    Tag::Strong => (M::Strong, None),
                    Tag::Emphasis => (M::Emphasis, None),
                    Tag::Strikethrough => (M::Strike, None),
                    Tag::Link { dest_url, .. } => (M::Link, link_target(&dest_url)),
                    _ => (M::Plain, None),
                };
                styled |= tag != M::Paragraph;
                stack.push((tag, Vec::new(), target, start));
                continue;
            }
            Event::End(_) => {
                let (tag, children, target, start) = stack.pop()?;
                MobileMarkdownNode::Element {
                    tag,
                    children,
                    target,
                    start,
                }
            }
            Event::Code(text) => {
                styled = true;
                MobileMarkdownNode::Element {
                    tag: M::Code,
                    children: vec![MobileMarkdownNode::Text(text.into_string())],
                    target: None,
                    start: None,
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                MobileMarkdownNode::Text(text.into_string())
            }
            Event::SoftBreak | Event::HardBreak => MobileMarkdownNode::Text("\n".into()),
            Event::Rule => MobileMarkdownNode::Text("―\n".into()),
            _ => continue,
        };
        stack
            .last_mut()
            .map_or(&mut root, |frame| &mut frame.1)
            .push(node);
    }
    (styled && stack.is_empty()).then_some(root)
}

fn link_target(href: &str) -> Option<MobileMarkdownLink> {
    if href.len() > 2048 || href.chars().any(char::is_whitespace) || href.contains('\\') {
        return None;
    }
    if ["https://", "http://"].iter().any(|scheme| {
        href.get(..scheme.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(scheme))
    }) {
        return Some(MobileMarkdownLink::Web {
            url: href.to_owned(),
        });
    }
    project_link(href).map(|_| MobileMarkdownLink::Project { id: link_id(href) })
}

fn link_id(href: &str) -> String {
    format!("{:x}", Sha256::digest(href.as_bytes()))
}

fn project_link(href: &str) -> Option<MobileProjectLink> {
    let href = href
        .strip_prefix("file://localhost")
        .filter(|path| path.starts_with('/'))
        .or_else(|| {
            href.strip_prefix("file://")
                .filter(|path| path.starts_with('/'))
        })
        .unwrap_or(href);
    let (path, anchor) = href.split_once('#').unwrap_or((href, ""));
    let line = anchor
        .strip_prefix('L')
        .unwrap_or(anchor)
        .parse::<usize>()
        .ok()
        .filter(|line| *line != 0);
    let path = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .ok()?;
    if path.is_empty()
        || path.starts_with(['~', '?'])
        || path.starts_with("//")
        || path.contains([':', '\\', '?'])
        || path.chars().any(char::is_control)
    {
        return None;
    }
    Some(MobileProjectLink {
        path: path.into_owned(),
        line,
    })
}

pub fn mobile_question_project_link(input: &str, id: &str) -> Option<MobileProjectLink> {
    events(input).take(MAX_EVENTS).find_map(|event| {
        let Event::Start(Tag::Link { dest_url, .. }) = event else {
            return None;
        };
        match link_target(&dest_url) {
            Some(MobileMarkdownLink::Project { id: actual }) if id == actual => {
                project_link(&dest_url)
            }
            _ => None,
        }
    })
}
