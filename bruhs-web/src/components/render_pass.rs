//! Post-render pass for assistant markdown: pure-Rust/WASM syntax
//! highlighting for fenced code, TeX math → SVG for formulas, and 3-stage
//! diagram layout → SVG for Mermaid fences. Runs when a stream settles so
//! partial blocks never flash half-highlighted, and only over prose blocks it
//! has not processed yet (each is marked once done), so a settled turn does
//! not re-process the whole thread.

use bruhs_models::diagram::{render_mermaid_svg, Theme as DiagramTheme};
use bruhs_models::syntax::highlight_to_hljs_html;
use bruhs_models::tex_math::{render_tex_with_metrics, MathOptions};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{AgentMessage, MessagePart};

/// Marks a `.prose-msg` block the pass has already processed. Markdown DOM
/// reconciliation removes the marker when its content changes, so it is
/// processed again without replacing unchanged prose nodes.
const DONE_ATTR: &str = "data-render-pass";

/// Install the effect once per chat page. Rendering happens synchronously in
/// WASM using the shared pure-Rust engines in `bruhs-models` (zero external JS
/// bundles).
pub fn install_render_pass(
    messages: ReadSignal<Vec<AgentMessage>>,
    is_streaming: ReadSignal<bool>,
) {
    Effect::new(move |_| {
        if is_streaming.get() {
            return;
        }
        let (needs_math, needs_mermaid) = messages.with(|msgs| {
            let texts = || {
                msgs.iter().flat_map(|m| {
                    m.parts.iter().filter_map(|p| match p {
                        MessagePart::Text(t) => Some(t.as_str()),
                        _ => None,
                    })
                })
            };
            (
                texts().any(has_math),
                texts().any(|t| t.contains("```mermaid")),
            )
        });

        schedule_render_pass(needs_math, needs_mermaid);
    });
}

/// Run the pass once the DOM has settled, over every `.prose-msg` block it
/// has not processed yet. Pages with their own prose (a room, a bruh's
/// chat) call it when their lines change.
pub fn schedule_render_pass(needs_math: bool, needs_mermaid: bool) {
    gloo_timers::callback::Timeout::new(0, move || {
        run_pass(needs_math, needs_mermaid);
    })
    .forget();
}

/// Whether `markdown` has math delimiters: `$$…$$`, `\[…\]`, `\(…\)`, or
/// inline `$…$` whose content starts and ends with a non-space and whose
/// closing `$` is not followed by a digit (so `$5 and $10` is prices, not
/// math). Code spans and fenced blocks do not count.
pub fn has_math(markdown: &str) -> bool {
    let prose = without_code(markdown);
    split_math_segments(&prose)
        .iter()
        .any(|seg| matches!(seg, MathSegment::Math { .. }))
}

/// `markdown` with fenced code blocks and inline code spans removed.
fn without_code(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let mut in_span = false;
        for c in line.chars() {
            if c == '`' {
                in_span = !in_span;
            } else if !in_span {
                out.push(c);
            }
        }
        out.push('\n');
    }
    out
}

#[derive(Debug, PartialEq, Eq)]
enum MathSegment<'a> {
    Text(&'a str),
    Math {
        source: &'a str,
        raw: &'a str,
        display: bool,
    },
}

/// Split plain text into literal runs and delimited TeX math spans (`$$…$$`,
/// `\[…\]`, `\(…\)`, and currency-safe inline `$…$`).
fn split_math_segments(text: &str) -> Vec<MathSegment<'_>> {
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut segments = Vec::new();
    let mut literal_start = 0;
    let mut i = 0;

    while i < len {
        let escaped = is_escaped(bytes, i);
        if !escaped && bytes[i] == b'\\' && i + 1 < len && matches!(bytes[i + 1], b'[' | b'(') {
            let display = bytes[i + 1] == b'[';
            let close_delim = if display { "\\]" } else { "\\)" };
            let content_start = i + 2;
            if let Some(close_pos) = find_unescaped(text, content_start, close_delim) {
                let source = &text[content_start..close_pos];
                if !source.trim().is_empty() {
                    if i > literal_start {
                        segments.push(MathSegment::Text(&text[literal_start..i]));
                    }
                    let end = close_pos + close_delim.len();
                    segments.push(MathSegment::Math {
                        source,
                        raw: &text[i..end],
                        display,
                    });
                    i = end;
                    literal_start = i;
                    continue;
                }
            }
            i += 2;
            continue;
        }

        if !escaped && bytes[i] == b'$' {
            let display = i + 1 < len && bytes[i + 1] == b'$';
            if display {
                let content_start = i + 2;
                if let Some(close_pos) = find_unescaped(text, content_start, "$$") {
                    let source = &text[content_start..close_pos];
                    if !source.trim().is_empty() {
                        if i > literal_start {
                            segments.push(MathSegment::Text(&text[literal_start..i]));
                        }
                        let end = close_pos + 2;
                        segments.push(MathSegment::Math {
                            source,
                            raw: &text[i..end],
                            display: true,
                        });
                        i = end;
                        literal_start = i;
                        continue;
                    }
                    i = close_pos + 2;
                    continue;
                }
                i += 2;
                continue;
            }

            let content_start = i + 1;
            if let Some(close_pos) = find_unescaped_char(text, content_start, b'$') {
                let source = &text[content_start..close_pos];
                let after = bytes.get(close_pos + 1).copied();
                let valid = !source.is_empty()
                    && !source.chars().next().is_some_and(char::is_whitespace)
                    && !source.chars().next_back().is_some_and(char::is_whitespace)
                    && !after.is_some_and(|b| b.is_ascii_digit());
                if valid {
                    if i > literal_start {
                        segments.push(MathSegment::Text(&text[literal_start..i]));
                    }
                    let end = close_pos + 1;
                    segments.push(MathSegment::Math {
                        source,
                        raw: &text[i..end],
                        display: false,
                    });
                    i = end;
                    literal_start = i;
                    continue;
                }
                // Not valid inline math; the closing `$` may open the next candidate.
                i = close_pos;
                continue;
            }
            i += 1;
            continue;
        }

        i += utf8_step(bytes[i]);
    }

    if literal_start < len {
        segments.push(MathSegment::Text(&text[literal_start..]));
    }
    segments
}

fn is_escaped(bytes: &[u8], pos: usize) -> bool {
    let mut slashes = 0usize;
    let mut idx = pos;
    while idx > 0 && bytes[idx - 1] == b'\\' {
        slashes += 1;
        idx -= 1;
    }
    slashes % 2 == 1
}

fn find_unescaped(text: &str, start: usize, needle: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut offset = start;
    while offset < text.len() {
        let rel = text[offset..].find(needle)?;
        let pos = offset + rel;
        if !is_escaped(bytes, pos) {
            return Some(pos);
        }
        offset = pos + 1;
    }
    None
}

fn find_unescaped_char(text: &str, start: usize, target: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == target && !is_escaped(bytes, i) {
            return Some(i);
        }
        i += utf8_step(bytes[i]);
    }
    None
}

const fn utf8_step(first: u8) -> usize {
    if first < 0x80 {
        1
    } else if first < 0xE0 {
        2
    } else if first < 0xF0 {
        3
    } else {
        4
    }
}

fn run_pass(needs_math: bool, needs_mermaid: bool) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(blocks) = document.query_selector_all(&format!(".prose-msg:not([{DONE_ATTR}])")) else {
        return;
    };
    if blocks.length() == 0 {
        return;
    }

    let mermaid_theme = needs_mermaid.then(DiagramTheme::bruhs_dark);

    for i in 0..blocks.length() {
        let Some(block) = blocks
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        // 1. Mermaid first, so diagram sources are not highlighted as code.
        if let Some(theme) = &mermaid_theme {
            convert_mermaid(&document, &block, theme);
        }
        // 2. Native Rust syntax highlighting on this block's fenced code.
        highlight_code_blocks(&block);
        // 3. Native Rust TeX math → SVG over this block's prose text nodes.
        if needs_math {
            render_math_in_element(&document, &block);
        }
        if let Err(error) = block.set_attribute(DONE_ATTR, "") {
            log::warn!("render pass: could not mark a block done: {error:?}");
        }
    }
}

/// Replace each `pre > code.language-mermaid` in `block` with an
/// "Open as board" button + `<div class="mermaid">` holding the rendered SVG.
fn convert_mermaid(document: &web_sys::Document, block: &web_sys::Element, theme: &DiagramTheme) {
    let Ok(codes) = block.query_selector_all("pre > code.language-mermaid") else {
        return;
    };
    for i in 0..codes.length() {
        let Some(code) = codes.item(i) else {
            continue;
        };
        let (Some(pre), Ok(div)) = (code.parent_element(), document.create_element("div")) else {
            continue;
        };
        let Some(parent) = pre.parent_element() else {
            continue;
        };
        if div.set_attribute("class", "mermaid").is_err() {
            continue;
        }
        let source = code.text_content().unwrap_or_default();
        match render_mermaid_svg(&source, theme) {
            Ok(svg) => {
                div.set_inner_html(&svg);
            }
            Err(error) => {
                let _ = div.set_attribute(
                    "class",
                    "mermaid rounded-grape border border-lychee-400/30 bg-longan-950 p-3 text-code-sm text-rambutan-400",
                );
                if let Ok(title) = document.create_element("div") {
                    let _ = title.set_attribute("class", "font-medium text-lychee-400");
                    title.set_text_content(Some(&format!(
                        "Mermaid diagram failed to render: {error}"
                    )));
                    let _ = div.append_child(&title);
                }
                if let Ok(fallback) = document.create_element("pre") {
                    fallback.set_text_content(Some(&source));
                    let _ = div.append_child(&fallback);
                }
            }
        }
        let Ok(wrapper) = document.create_element("div") else {
            continue;
        };
        let Ok(button) = document.create_element("button") else {
            continue;
        };
        let _ = button.set_attribute("type", "button");
        let _ = button.set_attribute("data-open-whiteboard", &source);
        let _ = button.set_attribute(
            "class",
            "my-2 min-h-control rounded-grape border border-longan-700 px-3 text-label-sm hover:bg-longan-800",
        );
        button.set_text_content(Some("Open as board"));
        if wrapper.append_child(&button).is_err() || wrapper.append_child(&div).is_err() {
            continue;
        }
        let _ = parent.replace_child(&wrapper, &pre);
    }
}

/// Highlight every `pre code` element inside `block` using `bruhs_models::syntax`
/// and tag it with the `hljs` class for token styles and DOM checks.
fn highlight_code_blocks(block: &web_sys::Element) {
    let Ok(codes) = block.query_selector_all("pre code") else {
        return;
    };
    for j in 0..codes.length() {
        let Some(code) = codes
            .item(j)
            .and_then(|node| node.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let class_name = code.class_name();
        let lang = class_name
            .split_whitespace()
            .find_map(|cls| cls.strip_prefix("language-"));
        if let Some(lang) = lang {
            let source = code.text_content().unwrap_or_default();
            if let Some(html) = highlight_to_hljs_html(lang, &source) {
                code.set_inner_html(&html);
            }
        }
        if !class_name.split_whitespace().any(|cls| cls == "hljs") {
            let updated = if class_name.trim().is_empty() {
                "hljs".to_string()
            } else {
                format!("{class_name} hljs")
            };
            let _ = code.set_attribute("class", &updated);
        }
    }
}

/// Walk `element`'s descendants and replace TeX math runs inside prose text
/// nodes with inline/display SVGs rendered by `bruhs_models::tex_math`.
fn render_math_in_element(document: &web_sys::Document, element: &web_sys::Element) {
    let children = element.child_nodes();
    let mut nodes = Vec::with_capacity(usize::try_from(children.length()).unwrap_or(0));
    for idx in 0..children.length() {
        if let Some(child) = children.item(idx) {
            nodes.push(child);
        }
    }

    for node in nodes {
        match node.node_type() {
            web_sys::Node::ELEMENT_NODE => {
                let Ok(child_el) = node.dyn_into::<web_sys::Element>() else {
                    continue;
                };
                if should_skip_math_subtree(&child_el) {
                    continue;
                }
                render_math_in_element(document, &child_el);
            }
            web_sys::Node::TEXT_NODE => {
                let text = node.text_content().unwrap_or_default();
                let segments = split_math_segments(&text);
                if !segments
                    .iter()
                    .any(|seg| matches!(seg, MathSegment::Math { .. }))
                {
                    continue;
                }
                for segment in segments {
                    let replacement: web_sys::Node = match segment {
                        MathSegment::Text(literal) => document.create_text_node(literal).into(),
                        MathSegment::Math {
                            source,
                            raw,
                            display,
                        } => {
                            let options = MathOptions {
                                font_size: if display { 16.0 } else { 14.0 },
                                display,
                            };
                            match render_tex_with_metrics(source, &options) {
                                Ok(rendered) => {
                                    let Ok(span) = document.create_element("span") else {
                                        continue;
                                    };
                                    if display {
                                        let _ = span.set_attribute(
                                            "class",
                                            "bruhs-math-display block my-2 overflow-x-auto text-center",
                                        );
                                    } else {
                                        let depth = (rendered.height - rendered.baseline).max(0.0);
                                        let _ = span.set_attribute(
                                            "class",
                                            "bruhs-math-inline inline-block",
                                        );
                                        let _ = span.set_attribute(
                                            "style",
                                            &format!("vertical-align:-{depth:.1}px;"),
                                        );
                                    }
                                    span.set_inner_html(&rendered.svg);
                                    span.into()
                                }
                                Err(_) => document.create_text_node(raw).into(),
                            }
                        }
                    };
                    let _ = element.insert_before(&replacement, Some(&node));
                }
                let _ = element.remove_child(&node);
            }
            _ => {}
        }
    }
}

fn should_skip_math_subtree(element: &web_sys::Element) -> bool {
    let tag = element.tag_name();
    if matches!(
        tag.as_str(),
        "PRE" | "CODE" | "SCRIPT" | "STYLE" | "TEXTAREA" | "SVG" | "BUTTON"
    ) {
        return true;
    }
    let cls = element.class_name();
    cls.split_whitespace()
        .any(|c| c == "mermaid" || c.starts_with("bruhs-math-"))
}

#[cfg(test)]
mod tests {
    use super::{has_math, split_math_segments, MathSegment};

    #[test]
    fn math_needs_real_delimiters() {
        for math in [
            "Euler: $e^{i\\pi} + 1 = 0$.",
            "$$\\sum_{i=1}^n i$$",
            "Display:\n$$\nx^2\n$$",
            "\\[ a = b \\]",
            "inline \\(x\\) here",
            "$x$",
            "costs $5 but $x_1$ is math",
        ] {
            assert!(has_math(math), "{math:?}");
        }
        for prose in [
            "It costs $5.",
            "From $5 to $10.",
            "Pay $5 and $ 10",
            "$ x $",
            "a lone $ sign",
            "escaped \\$5 and \\$6",
            "`$x$` in code",
            "```\nlet s = \"$a$\";\n```",
            "$$ $$",
            "",
        ] {
            assert!(!has_math(prose), "{prose:?}");
        }
    }

    #[test]
    fn splits_inline_display_and_currency_safely() {
        let segs = split_math_segments("Costs $5, solve $x^2 + 1 = 0$ and \\[a = b\\].");
        assert_eq!(
            segs,
            vec![
                MathSegment::Text("Costs $5, solve "),
                MathSegment::Math {
                    source: "x^2 + 1 = 0",
                    raw: "$x^2 + 1 = 0$",
                    display: false,
                },
                MathSegment::Text(" and "),
                MathSegment::Math {
                    source: "a = b",
                    raw: "\\[a = b\\]",
                    display: true,
                },
                MathSegment::Text("."),
            ]
        );
    }

    #[test]
    fn handles_unicode_and_escaped_delimiters() {
        let segs = split_math_segments("猫 \\$not\\$ \\(α + β\\) 友");
        assert_eq!(
            segs,
            vec![
                MathSegment::Text("猫 \\$not\\$ "),
                MathSegment::Math {
                    source: "α + β",
                    raw: "\\(α + β\\)",
                    display: false,
                },
                MathSegment::Text(" 友"),
            ]
        );
    }
}
