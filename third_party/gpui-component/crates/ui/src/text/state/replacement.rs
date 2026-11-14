//! Reuse only a paragraph tail whose preceding blocks cannot change on append.
use super::{ParsedContent, TextViewFormat, format, node};
use crate::{
    highlighter::HighlightTheme,
    text::{MarkdownExtensions, document::ParsedDocument, node::NodeContext},
};
use gpui::SharedString;
use std::sync::Arc;

pub(super) fn parse_replacement(
    format: TextViewFormat,
    previous: &ParsedContent,
    source: &str,
    highlight_theme: &HighlightTheme,
    extensions: &Arc<MarkdownExtensions>,
) -> Result<ParsedContent, SharedString> {
    if let Some(restart) = paragraph_restart(format, previous, source, extensions) {
        let mut node_cx = NodeContext {
            offset: restart,
            markdown_extensions: extensions.clone(),
            ..NodeContext::default()
        };
        if let Ok(tail) = format::markdown::parse(&source[restart..], &mut node_cx, highlight_theme)
            && !node_cx.has_global_definitions
            && matches!(tail.blocks.first(), Some(node::BlockNode::Paragraph(_)))
        {
            // A new list/fence/heading can absorb a preceding block. A paragraph
            // separated by a blank line cannot; definitions fall back globally.
            let prefix = &previous.document.blocks[..previous.document.blocks.len() - 1];
            let blocks = prefix.iter().cloned().chain(tail.blocks).collect();
            return Ok(ParsedContent {
                document: ParsedDocument {
                    source: source.to_string().into(),
                    blocks,
                },
                node_cx,
            });
        }
    }

    let mut node_cx = NodeContext {
        markdown_extensions: extensions.clone(),
        ..NodeContext::default()
    };
    let document = match format {
        TextViewFormat::Markdown => format::markdown::parse(source, &mut node_cx, highlight_theme),
        TextViewFormat::Html => format::html::parse(source, &mut node_cx),
    }?;
    Ok(ParsedContent { document, node_cx })
}

fn paragraph_restart(
    format: TextViewFormat,
    previous: &ParsedContent,
    source: &str,
    extensions: &Arc<MarkdownExtensions>,
) -> Option<usize> {
    if format != TextViewFormat::Markdown
        || extensions.revision() != 0
        || previous.node_cx.has_global_definitions
        || previous.document.blocks.len() < 2
    {
        return None;
    }
    let old = previous.document.source.as_str();
    if source.len() <= old.len() || !source.starts_with(old) {
        return None;
    }
    let last = previous.document.blocks.last()?;
    if !matches!(last, node::BlockNode::Paragraph(_)) {
        return None;
    }
    let before = old.get(..last.span()?.start)?;
    let restart = before.rfind(['\n', '\r'])? + 1;
    let boundary = old.get(..restart)?;
    let preceding = boundary
        .strip_suffix("\r\n")
        .or_else(|| boundary.strip_suffix(['\n', '\r']))?;
    let line = preceding.rsplit(['\n', '\r']).next()?;
    line.bytes()
        .all(|c| matches!(c, b' ' | b'\t'))
        .then_some(restart)
}
