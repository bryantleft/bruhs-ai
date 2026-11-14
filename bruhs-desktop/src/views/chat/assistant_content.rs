//! Assistant content: splitting markdown into prose, math and mermaid blocks, and rendering those to SVG.

use super::*;
use bruhs_models::logic::rich_response::{response_blocks, ResponseBlock};
pub(super) const CHAT_DIAGRAM_ERROR_FONT_FAMILY: &str = "Monaspace Argon";
pub(super) const CHAT_DIAGRAM_ERROR_TEXT_SIZE_PX: f32 = 10.5;
pub(super) const CHAT_DIAGRAM_ERROR_RADIUS_PX: f32 = 8.0;
pub(super) const CHAT_DIAGRAM_ERROR_X_PADDING_PX: f32 = 10.0;
pub(super) const CHAT_DIAGRAM_ERROR_Y_PADDING_PX: f32 = 8.0;
pub(super) const CHAT_DIAGRAM_ERROR_LINE_HEIGHT: f32 = 1.45;
pub(super) const CHAT_DIAGRAM_ERROR_BORDER_OPACITY: f32 = 0.30;
pub(super) const CHAT_MERMAID_CARD_RADIUS_PX: f32 = 8.0;
pub(super) const CHAT_MERMAID_CARD_PADDING_PX: f32 = 8.0;
pub(super) const CHAT_MERMAID_CARD_BORDER_OPACITY: f32 = 0.60;
pub(super) const CHAT_MERMAID_MIN_WIDTH_PX: f32 = 180.0;
pub(super) const CHAT_CONTENT_BLOCK_MAX_WIDTH: f32 = 1.0;
pub(super) const CHAT_MATH_DISPLAY_Y_PADDING_PX: f32 = 3.0;
pub(super) const CHAT_PROSE_PARAGRAPH_GAP_REM: f32 = 0.5;
pub(super) const CHAT_PROSE_HEADING_1_TEXT_SIZE_PX: f32 = 14.0;
pub(super) const CHAT_PROSE_HEADING_2_TEXT_SIZE_PX: f32 = 13.0;
pub(super) const CHAT_PROSE_HEADING_FALLBACK_TEXT_SIZE_PX: f32 = 12.0;
pub(super) const CHAT_PROSE_CODE_TEXT_SIZE_PX: f32 = 10.5;
pub(super) const CHAT_PROSE_CODE_RADIUS_PX: f32 = 8.0;
pub(super) const CHAT_PROSE_CODE_X_PADDING_PX: f32 = 12.0;
pub(super) const CHAT_PROSE_CODE_Y_PADDING_PX: f32 = 10.0;
pub(super) const CHAT_PROSE_CODE_LINE_HEIGHT: f32 = 1.55;
pub(super) const CHAT_PROSE_CODE_BORDER_OPACITY: f32 = 1.0;

pub(super) fn desktop_markdown_style(cx: &App) -> TextViewStyle {
    let mut style = TextViewStyle::default()
        .paragraph_gap(rems(CHAT_PROSE_PARAGRAPH_GAP_REM))
        .heading_font_size(|level, _base| match level {
            1 => px(CHAT_PROSE_HEADING_1_TEXT_SIZE_PX),
            2 => px(CHAT_PROSE_HEADING_2_TEXT_SIZE_PX),
            _ => px(CHAT_PROSE_HEADING_FALLBACK_TEXT_SIZE_PX),
        })
        .code_block(
            StyleRefinement::default()
                .bg(theme::longan_950())
                .border_1()
                .border_color(theme::longan_800().opacity(CHAT_PROSE_CODE_BORDER_OPACITY))
                .rounded(px(CHAT_PROSE_CODE_RADIUS_PX))
                .px(px(CHAT_PROSE_CODE_X_PADDING_PX))
                .py(px(CHAT_PROSE_CODE_Y_PADDING_PX))
                .text_size(px(CHAT_PROSE_CODE_TEXT_SIZE_PX))
                .line_height(relative(CHAT_PROSE_CODE_LINE_HEIGHT))
                .font_family(TOOL_CODE_FONT_FAMILY)
                .text_color(theme::rambutan_300()),
        );
    style.highlight_theme = cx.theme().highlight_theme.clone();
    style.is_dark = true;
    style
}

#[cfg(test)]
pub(super) fn desktop_assistant_content_blocks(raw: &str) -> Vec<DesktopAssistantContentBlock> {
    desktop_assistant_content_blocks_while(raw, false)
}

#[cfg(test)]
pub(super) fn desktop_assistant_content_blocks_while(
    raw: &str,
    streaming: bool,
) -> Vec<DesktopAssistantContentBlock> {
    parse_assistant_content(raw, streaming)
        .into_iter()
        .map(|block| match block {
            ParsedAssistantBlock::Markdown(text) => DesktopAssistantContentBlock::Markdown(text),
            ParsedAssistantBlock::Math { source, display } => {
                DesktopAssistantContentBlock::Math(DesktopMathBlock {
                    render: render_desktop_math(&source, display),
                    source,
                    display,
                })
            }
            ParsedAssistantBlock::Mermaid(source) => {
                DesktopAssistantContentBlock::Mermaid(DesktopMermaidBlock {
                    render: render_desktop_mermaid(&source),
                    source,
                })
            }
            ParsedAssistantBlock::Rich(block) => DesktopAssistantContentBlock::Rich(block),
        })
        .collect()
}

/// Text and block boundaries only: SVG generation must never run during layout.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ParsedAssistantBlock {
    Markdown(String),
    Math { source: String, display: bool },
    Mermaid(String),
    Rich(ResponseBlock),
}

/// `raw` split into prose, math and mermaid blocks. While `streaming`, a
/// mermaid fence that is not closed yet stays a code block: rendering every
/// partial diagram would render (and cache) one per streamed token.
pub(super) fn parse_assistant_content(raw: &str, streaming: bool) -> Vec<ParsedAssistantBlock> {
    response_blocks(raw, streaming)
        .into_iter()
        .flat_map(|block| match block {
            ResponseBlock::Markdown(markdown)
                if markdown.trim().is_empty() && !raw.trim().is_empty() =>
            {
                Vec::new()
            }
            ResponseBlock::Markdown(markdown) => parse_prose_content(&markdown, streaming),
            block => vec![ParsedAssistantBlock::Rich(block)],
        })
        .collect()
}

fn parse_prose_content(raw: &str, streaming: bool) -> Vec<ParsedAssistantBlock> {
    // The prose renderer already parses Markdown. Only parse it here when a
    // fence's info string could name Mermaid, not when prose mentions the word.
    let parsed = if may_have_mermaid_fence(raw) {
        markdown::to_mdast(raw, &ParseOptions::gfm()).ok()
    } else {
        None
    };
    let Some(ast) = parsed else {
        let mut blocks = Vec::new();
        push_markdown_block(&mut blocks, raw);
        if blocks.is_empty() {
            blocks.push(ParsedAssistantBlock::Markdown(raw.to_string()));
        }
        return blocks;
    };

    let mut ranges = Vec::new();
    collect_mermaid_code_ranges(&ast, raw, &mut ranges);
    if streaming {
        ranges.retain(|range| code_fence_closed(&raw[range.start..range.end]));
    }

    if ranges.is_empty() {
        let mut blocks = Vec::new();
        push_markdown_block(&mut blocks, raw);
        if blocks.is_empty() {
            blocks.push(ParsedAssistantBlock::Markdown(raw.to_string()));
        }
        return blocks;
    }

    ranges.sort_by_key(|range| range.start);

    let mut blocks = Vec::new();
    let mut cursor = 0;
    for range in ranges {
        if range.start < cursor || range.end > raw.len() {
            continue;
        }

        push_markdown_block(&mut blocks, &raw[cursor..range.start]);
        blocks.push(ParsedAssistantBlock::Mermaid(range.source));
        cursor = range.end;
    }

    push_markdown_block(&mut blocks, &raw[cursor..]);

    if blocks.is_empty() {
        blocks.push(ParsedAssistantBlock::Markdown(String::new()));
    }

    blocks
}

pub(super) const MERMAID_LANG: &str = "mermaid";

fn may_have_mermaid_fence(raw: &str) -> bool {
    raw.split(['\n', '\r']).any(|line| {
        let line = line.trim_start_matches([' ', '\t']);
        let Some(fence) = line.as_bytes().first().filter(|c| matches!(c, b'`' | b'~')) else {
            return false;
        };
        let count = line.bytes().take_while(|c| c == fence).count();
        if count < 3 {
            return false;
        }
        let language = line[count..].split_whitespace().next().unwrap_or_default();
        // Markdown decodes escapes and character references in info strings.
        // Let its parser decide those uncommon cases rather than duplicating it.
        language.eq_ignore_ascii_case(MERMAID_LANG) || language.contains(['&', '\\'])
    })
}

/// Whether the fenced code block `block` (opening fence through the end of
/// the node) has its closing fence yet.
pub(super) fn code_fence_closed(block: &str) -> bool {
    let mut lines = block.trim_end_matches(['\n', '\r']).split(['\n', '\r']);
    let opening = lines.next().unwrap_or_default().trim_start();
    let Some(fence) = opening.chars().next().filter(|c| matches!(c, '`' | '~')) else {
        // Not a fenced block (indented code has no closing fence).
        return true;
    };
    let open_len = opening.chars().take_while(|c| *c == fence).count();
    lines.next_back().is_some_and(|last| {
        let last = last.trim();
        last.chars().count() >= open_len && last.chars().all(|c| c == fence)
    })
}

pub(super) fn collect_mermaid_code_ranges(
    node: &mdast::Node,
    raw: &str,
    ranges: &mut Vec<DesktopMermaidRange>,
) {
    if let mdast::Node::Root(root) = node {
        for child in &root.children {
            if let mdast::Node::Code(code) = child {
                let Some(lang) = code.lang.as_deref() else {
                    continue;
                };
                if !lang.eq_ignore_ascii_case(MERMAID_LANG) {
                    continue;
                }
                let Some(position) = code.position.as_ref() else {
                    continue;
                };
                let Some(start) = markdown_offset_to_byte(raw, position.start.offset) else {
                    continue;
                };
                let Some(end) = markdown_offset_to_byte(raw, position.end.offset) else {
                    continue;
                };
                if start <= end && raw.is_char_boundary(start) && raw.is_char_boundary(end) {
                    ranges.push(DesktopMermaidRange {
                        start,
                        end,
                        source: code.value.clone(),
                    });
                }
            }
        }
    }
}

pub(super) fn markdown_offset_to_byte(raw: &str, offset: usize) -> Option<usize> {
    if offset <= raw.len() && raw.is_char_boundary(offset) {
        return Some(offset);
    }

    raw.char_indices()
        .map(|(byte_index, _)| byte_index)
        .chain(std::iter::once(raw.len()))
        .nth(offset)
}

pub(super) fn push_markdown_block(blocks: &mut Vec<ParsedAssistantBlock>, markdown: &str) {
    let markdown = markdown.trim_matches('\n');
    if markdown.trim().is_empty() {
        return;
    }

    let math_ranges = desktop_math_ranges(markdown);
    if math_ranges.is_empty() {
        blocks.push(ParsedAssistantBlock::Markdown(markdown.to_string()));
        return;
    }

    let mut cursor = 0;
    for range in math_ranges {
        if range.start < cursor || range.end > markdown.len() {
            continue;
        }

        push_plain_markdown_block(blocks, &markdown[cursor..range.start]);
        blocks.push(ParsedAssistantBlock::Math {
            source: range.source,
            display: range.display,
        });
        cursor = range.end;
    }

    push_plain_markdown_block(blocks, &markdown[cursor..]);
}

pub(super) fn push_plain_markdown_block(blocks: &mut Vec<ParsedAssistantBlock>, markdown: &str) {
    if !markdown.trim().is_empty() {
        blocks.push(ParsedAssistantBlock::Markdown(
            markdown.trim_matches('\n').to_string(),
        ));
    }
}

pub(super) fn desktop_math_ranges(raw: &str) -> Vec<DesktopMathRange> {
    let mut ranges = Vec::new();
    let mut index = 0;
    // Once a suffix has no closing delimiter, later openers cannot find one
    // either. Do not rescan that suffix for every unfinished streamed opener.
    let mut bracket_end_missing = false;
    let mut parenthesis_end_missing = false;

    while index < raw.len() {
        if !raw.is_char_boundary(index) {
            index += 1;
            continue;
        }

        if let Some(next_index) = fenced_code_end(raw, index) {
            index = next_index;
            continue;
        }

        if raw.as_bytes()[index] == b'`' {
            if let Some(next_index) = inline_code_end(raw, index) {
                index = next_index;
                continue;
            }
        }

        if starts_unescaped(raw, index, "$$") {
            if let Some(end) = find_unescaped(raw, "$$", index + 2) {
                push_math_range(&mut ranges, raw, index, end, 2, true);
                index = end + 2;
                continue;
            }
        } else if !bracket_end_missing && starts_unescaped(raw, index, "\\[") {
            if let Some(end) = find_unescaped(raw, "\\]", index + 2) {
                push_math_range(&mut ranges, raw, index, end, 2, true);
                index = end + 2;
                continue;
            }
            bracket_end_missing = true;
        } else if !parenthesis_end_missing && starts_unescaped(raw, index, "\\(") {
            if let Some(end) = find_unescaped(raw, "\\)", index + 2) {
                push_math_range(&mut ranges, raw, index, end, 2, false);
                index = end + 2;
                continue;
            }
            parenthesis_end_missing = true;
        } else if starts_single_dollar_math(raw, index) {
            if let Some(end) = find_single_dollar_end(raw, index + 1) {
                push_math_range(&mut ranges, raw, index, end, 1, false);
                index = end + 1;
                continue;
            }
        }

        index += raw[index..].chars().next().map_or(1, char::len_utf8);
    }

    ranges
}

pub(super) fn push_math_range(
    ranges: &mut Vec<DesktopMathRange>,
    raw: &str,
    start: usize,
    content_end: usize,
    delimiter_len: usize,
    display: bool,
) {
    let source = raw[start + delimiter_len..content_end].trim().to_string();
    if source.is_empty() {
        return;
    }

    ranges.push(DesktopMathRange {
        start,
        end: content_end + delimiter_len,
        source,
        display,
    });
}

pub(super) fn fenced_code_end(raw: &str, index: usize) -> Option<usize> {
    if !is_line_start(raw, index) {
        return None;
    }

    let fence = if raw[index..].starts_with("```") {
        "```"
    } else if raw[index..].starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };

    let mut search = raw[index..]
        .find('\n')
        .map_or(raw.len(), |pos| index + pos + 1);
    while search < raw.len() {
        if is_line_start(raw, search) && raw[search..].starts_with(fence) {
            return raw[search..]
                .find('\n')
                .map_or(Some(raw.len()), |pos| Some(search + pos + 1));
        }
        search = raw[search..]
            .find('\n')
            .map_or(raw.len(), |pos| search + pos + 1);
    }

    Some(raw.len())
}

pub(super) fn inline_code_end(raw: &str, index: usize) -> Option<usize> {
    let tick_count = raw[index..]
        .bytes()
        .take_while(|byte| *byte == b'`')
        .count();
    if tick_count == 0 {
        return None;
    }

    let delimiter = "`".repeat(tick_count);
    raw[index + tick_count..]
        .find(&delimiter)
        .map(|pos| index + tick_count + pos + tick_count)
}

pub(super) fn is_line_start(raw: &str, index: usize) -> bool {
    index == 0 || raw.as_bytes().get(index.wrapping_sub(1)) == Some(&b'\n')
}

pub(super) fn starts_unescaped(raw: &str, index: usize, delimiter: &str) -> bool {
    raw[index..].starts_with(delimiter) && !is_escaped(raw, index)
}

pub(super) fn starts_single_dollar_math(raw: &str, index: usize) -> bool {
    raw.as_bytes().get(index) == Some(&b'$')
        && raw.as_bytes().get(index + 1) != Some(&b'$')
        && !is_escaped(raw, index)
}

pub(super) fn find_unescaped(raw: &str, delimiter: &str, start: usize) -> Option<usize> {
    let mut search = start;
    while search < raw.len() {
        let relative = raw[search..].find(delimiter)?;
        let index = search + relative;
        if !is_escaped(raw, index) {
            return Some(index);
        }
        search = index + delimiter.len();
    }

    None
}

pub(super) fn find_single_dollar_end(raw: &str, start: usize) -> Option<usize> {
    let mut search = start;
    while search < raw.len() {
        let relative = raw[search..].find('$')?;
        let index = search + relative;
        if raw.as_bytes().get(index + 1) != Some(&b'$') && !is_escaped(raw, index) {
            return Some(index);
        }
        search = index + 1;
    }

    None
}

pub(super) fn is_escaped(raw: &str, index: usize) -> bool {
    let mut slash_count = 0;
    for byte in raw[..index].bytes().rev() {
        if byte == b'\\' {
            slash_count += 1;
        } else {
            break;
        }
    }

    slash_count % 2 == 1
}

pub(super) fn render_desktop_math(source: &str, display: bool) -> DesktopMathRender {
    type MathCache = BoundedCache<(String, bool), DesktopMathRender>;
    static CACHE: OnceLock<Mutex<MathCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BoundedCache::new(MATH_RENDER_CACHE_ENTRIES)));
    let key = (source.to_string(), display);
    if let Some(render) = lock_render_cache(cache).get(&key) {
        return render;
    }

    let options = mathjax_svg_rs::Options {
        font_size: if display { 15.0 } else { 13.0 },
        horizontal_align: if display {
            mathjax_svg_rs::HorizontalAlign::Center
        } else {
            mathjax_svg_rs::HorizontalAlign::Left
        },
    };

    let render = match mathjax_svg_rs::render_tex(source, &options) {
        Ok(svg) => DesktopMathRender::Svg(desktop_math_svg(svg)),
        Err(error) => DesktopMathRender::Error(error),
    };

    lock_render_cache(cache).insert(key, render.clone());
    render
}

pub(super) fn desktop_math_svg(svg: String) -> String {
    let svg = svg.replace("currentColor", "#dfcfc3");
    svg.replacen("<svg ", "<svg style=\"color:#dfcfc3\" ", 1)
}

pub(super) fn render_desktop_mermaid(source: &str) -> DesktopMermaidRender {
    static CACHE: OnceLock<Mutex<BoundedCache<String, DesktopMermaidRender>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(BoundedCache::new(MERMAID_RENDER_CACHE_ENTRIES)));
    let key = source.to_string();
    if let Some(render) = lock_render_cache(cache).get(&key) {
        return render;
    }

    let options = mermaid_rs_renderer::RenderOptions {
        theme: desktop_mermaid_theme(),
        layout: mermaid_rs_renderer::LayoutConfig::default(),
    };

    let render = match mermaid_rs_renderer::render_with_options(source, options) {
        Ok(svg) => DesktopMermaidRender::Svg(svg),
        Err(error) => DesktopMermaidRender::Error(error.to_string()),
    };

    lock_render_cache(cache).insert(key, render.clone());
    render
}

/// Most distinct formulas / diagrams whose rendered SVG is kept.
pub(super) const MATH_RENDER_CACHE_ENTRIES: usize = 256;
pub(super) const MERMAID_RENDER_CACHE_ENTRIES: usize = 64;

pub(super) fn lock_render_cache<K, V>(
    cache: &Mutex<BoundedCache<K, V>>,
) -> std::sync::MutexGuard<'_, BoundedCache<K, V>> {
    // A panic mid-insert leaves at worst a missing entry; keep using it.
    cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn desktop_mermaid_theme() -> mermaid_rs_renderer::Theme {
    let mut mermaid_theme = mermaid_rs_renderer::Theme::modern();
    mermaid_theme.background = "#141210".to_string();
    mermaid_theme.primary_color = "#2f2922".to_string();
    mermaid_theme.primary_text_color = "#dfcfc3".to_string();
    mermaid_theme.primary_border_color = "#594a36".to_string();
    mermaid_theme.line_color = "#c3afa1".to_string();
    mermaid_theme.secondary_color = "#1f1a16".to_string();
    mermaid_theme.tertiary_color = "#141210".to_string();
    mermaid_theme.edge_label_background = "#141210".to_string();
    mermaid_theme.cluster_background = "#1f1a16".to_string();
    mermaid_theme.cluster_border = "#594a36".to_string();
    mermaid_theme.sequence_actor_fill = "#2f2922".to_string();
    mermaid_theme.sequence_actor_border = "#594a36".to_string();
    mermaid_theme.sequence_actor_line = "#c3afa1".to_string();
    mermaid_theme.sequence_note_fill = "#2f2922".to_string();
    mermaid_theme.sequence_note_border = "#594a36".to_string();
    mermaid_theme.sequence_activation_fill = "#1f1a16".to_string();
    mermaid_theme.sequence_activation_border = "#594a36".to_string();
    mermaid_theme.text_color = "#dfcfc3".to_string();
    mermaid_theme.font_family = "Monaspace Neon, ui-monospace, monospace".to_string();
    mermaid_theme.font_size = 12.0;
    mermaid_theme
}

pub(super) fn desktop_block_id(
    prefix: &'static str,
    message_index: usize,
    block_index: usize,
) -> ElementId {
    ElementId::from((
        ElementId::from((prefix, message_index)),
        block_index.to_string(),
    ))
}

impl ChatView {
    fn render_pending_svg(
        &self,
        message_index: usize,
        block_index: usize,
        kind: &'static str,
        source: &SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        // A source containing backticks must remain a single code block.
        let longest = source.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        let markdown = format!("{fence}\n{source}\n{fence}");
        div()
            .id(desktop_block_id(
                "desktop-pending-rich-block",
                message_index,
                block_index,
            ))
            .debug_selector(move || format!("desktop-pending-{kind}-{message_index}-{block_index}"))
            .min_w_0()
            .w_full()
            .child(
                div()
                    .id("render-status")
                    .role(Role::Status)
                    .text_size(px(11.0))
                    .text_color(theme::rambutan_500())
                    .child(format!("Rendering {kind}…")),
            )
            .child(
                TextView::markdown(
                    desktop_block_id("desktop-rich-source", message_index, block_index),
                    markdown,
                )
                .style(desktop_markdown_style(cx))
                .selectable(true),
            )
            .into_any_element()
    }

    pub(super) fn render_assistant_content_blocks(
        &self,
        message_index: usize,
        content: &str,
        streaming: bool,
        _window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Vec<AnyElement> {
        let blocks = self
            .content_blocks
            .blocks(message_index, content, streaming);
        blocks
            .iter()
            .map(|(block_index, block)| {
                let block_index = *block_index;
                match block {
                    PreparedBlock::Markdown(markdown) => {
                        self.render_native_prose(message_index, block_index, markdown, cx)
                    }
                    PreparedBlock::Math { display, source } => {
                        let render = self.content_blocks.svg.request(
                            super::svg_cache::SvgKind::Math { display: *display },
                            source,
                            cx,
                        );
                        self.render_math_block(
                            message_index,
                            block_index,
                            *display,
                            &render,
                            source,
                            cx,
                        )
                    }
                    PreparedBlock::Mermaid { source } => {
                        let render = self.content_blocks.svg.request(
                            super::svg_cache::SvgKind::Mermaid,
                            source,
                            cx,
                        );
                        self.render_mermaid_block(message_index, block_index, &render, source, cx)
                    }
                    PreparedBlock::Rich { block, identity } => {
                        self.render_rich_response(message_index, block_index, block, identity, cx)
                    }
                }
            })
            .collect()
    }

    pub(super) fn render_math_block(
        &self,
        message_index: usize,
        block_index: usize,
        display: bool,
        render: &PreparedSvg,
        source: &SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        match render {
            PreparedSvg::Pending => {
                self.render_pending_svg(message_index, block_index, "math", source, cx)
            }
            PreparedSvg::Image(image) => div()
                .id(desktop_block_id(
                    "desktop-math-block",
                    message_index,
                    block_index,
                ))
                .max_w(relative(CHAT_CONTENT_BLOCK_MAX_WIDTH))
                .min_w_0()
                .overflow_x_scroll()
                .when(display, |el| el.py(px(CHAT_MATH_DISPLAY_Y_PADDING_PX)))
                .child(
                    img(Arc::clone(image))
                        .id(desktop_block_id(
                            "desktop-math-svg",
                            message_index,
                            block_index,
                        ))
                        .max_w(relative(CHAT_CONTENT_BLOCK_MAX_WIDTH)),
                )
                .into_any_element(),
            PreparedSvg::Error(error) => div()
                .id(desktop_block_id(
                    "desktop-math-error",
                    message_index,
                    block_index,
                ))
                .max_w(relative(CHAT_CONTENT_BLOCK_MAX_WIDTH))
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .rounded(px(CHAT_DIAGRAM_ERROR_RADIUS_PX))
                .border_1()
                .border_color(theme::lychee_400().opacity(CHAT_DIAGRAM_ERROR_BORDER_OPACITY))
                .bg(theme::longan_950())
                .px(px(CHAT_DIAGRAM_ERROR_X_PADDING_PX))
                .py(px(CHAT_DIAGRAM_ERROR_Y_PADDING_PX))
                .text_size(px(CHAT_DIAGRAM_ERROR_TEXT_SIZE_PX))
                .line_height(relative(CHAT_DIAGRAM_ERROR_LINE_HEIGHT))
                .text_color(theme::rambutan_400())
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::lychee_400())
                        .child("Math failed to render"),
                )
                .child(
                    div()
                        .font_family(CHAT_DIAGRAM_ERROR_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(error.clone()),
                )
                .child(
                    div()
                        .font_family(CHAT_DIAGRAM_ERROR_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(source.clone()),
                )
                .into_any_element(),
        }
    }

    pub(super) fn render_mermaid_block(
        &self,
        message_index: usize,
        block_index: usize,
        render: &PreparedSvg,
        source: &SharedString,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        match render {
            PreparedSvg::Pending => {
                self.render_pending_svg(message_index, block_index, "diagram", source, cx)
            }
            PreparedSvg::Image(image) => div()
                .id(desktop_block_id(
                    "desktop-mermaid-block",
                    message_index,
                    block_index,
                ))
                .max_w(relative(CHAT_CONTENT_BLOCK_MAX_WIDTH))
                .min_w_0()
                .overflow_x_scroll()
                .rounded(px(CHAT_MERMAID_CARD_RADIUS_PX))
                .border_1()
                .border_color(theme::longan_800().opacity(CHAT_MERMAID_CARD_BORDER_OPACITY))
                .bg(theme::longan_950())
                .p(px(CHAT_MERMAID_CARD_PADDING_PX))
                .child(
                    img(Arc::clone(image))
                        .id(desktop_block_id(
                            "desktop-mermaid-diagram",
                            message_index,
                            block_index,
                        ))
                        .max_w(relative(CHAT_CONTENT_BLOCK_MAX_WIDTH))
                        .min_w(px(CHAT_MERMAID_MIN_WIDTH_PX)),
                )
                .into_any_element(),
            PreparedSvg::Error(error) => div()
                .id(desktop_block_id(
                    "desktop-mermaid-error",
                    message_index,
                    block_index,
                ))
                .debug_selector(move || {
                    format!("desktop-mermaid-error-{message_index}-{block_index}")
                })
                .max_w(relative(CHAT_CONTENT_BLOCK_MAX_WIDTH))
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .rounded(px(CHAT_DIAGRAM_ERROR_RADIUS_PX))
                .border_1()
                .border_color(theme::lychee_400().opacity(CHAT_DIAGRAM_ERROR_BORDER_OPACITY))
                .bg(theme::longan_950())
                .px(px(CHAT_DIAGRAM_ERROR_X_PADDING_PX))
                .py(px(CHAT_DIAGRAM_ERROR_Y_PADDING_PX))
                .text_size(px(CHAT_DIAGRAM_ERROR_TEXT_SIZE_PX))
                .line_height(relative(CHAT_DIAGRAM_ERROR_LINE_HEIGHT))
                .text_color(theme::rambutan_400())
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::lychee_400())
                        .child("Mermaid diagram failed to render"),
                )
                .child(
                    div()
                        .font_family(CHAT_DIAGRAM_ERROR_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(error.clone()),
                )
                .child(
                    div()
                        .font_family(CHAT_DIAGRAM_ERROR_FONT_FAMILY)
                        .text_color(theme::rambutan_500())
                        .child(source.clone()),
                )
                .into_any_element(),
        }
    }

    pub(super) fn render_message(
        &self,
        index: usize,
        msg: &Message,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        #[cfg(test)]
        self.rendered_message_rows
            .set(self.rendered_message_rows.get() + 1);
        self.render_transcript_message(index, msg, window, cx)
    }

    pub(super) fn toggle_fold(&mut self, message_index: usize, cx: &mut Context<'_, Self>) {
        if !self.expanded_folds.remove(&message_index) {
            self.expanded_folds.insert(message_index);
        }
        cx.notify();
    }

    pub(super) fn toggle_thinking_block(
        &mut self,
        message_index: usize,
        cx: &mut Context<'_, Self>,
    ) {
        if message_index < self.messages.len() {
            if !self.expanded_thinking.remove(&message_index) {
                self.expanded_thinking.insert(message_index);
            }
            cx.notify();
        }
    }
}
