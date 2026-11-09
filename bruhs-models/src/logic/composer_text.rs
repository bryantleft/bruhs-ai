//! Composer Markdown grammar, the styled overlay, and large-paste attachments.
//!
//! The textarea keeps the characters the agent receives. The overlay only
//! changes color and background, so a heavier weight or a larger heading
//! cannot pull the caret off the glyphs. Syntax colors stay on the transcript.

use serde::{Deserialize, Serialize};

use super::context_chip::{chip_detail, ContextChip};

/// A typed message longer than this stays in the composer.
pub const MESSAGE_CHAR_LIMIT: usize = 120_000;

/// A paste this large, in bytes, becomes a text attachment.
pub const LARGE_PASTE_BYTES: usize = 32 * 1024;

/// A larger paste stays in the composer. The character limit then blocks send.
pub const MAX_TEXT_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

pub const MESSAGE_LIMIT_HINT: &str = "This message is longer than 120,000 characters.";

/// Languages a fence corner can write onto the opening line.
pub const FENCE_LANGUAGES: &[&str] = &[
    "text",
    "rust",
    "typescript",
    "javascript",
    "python",
    "go",
    "json",
    "markdown",
    "bash",
    "toml",
    "css",
    "html",
];

/// Pasted text held beside the composer. The body is not the user message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAttachment {
    pub name: String,
    pub text: String,
}

/// Where a paste goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteOutcome {
    /// The browser (or the desktop input) inserts the text.
    Insert,
    /// The paste becomes a text attachment. It does not enter the draft.
    Attach,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerKey {
    Enter,
    ShiftEnter,
    Tab,
    ShiftTab,
    Backspace,
}

/// A composer key either leaves the field alone, sends, or replaces a range.
/// `start` and `end` are UTF-8 byte offsets into the current text. `caret`
/// is the byte offset in the text after that replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposerEdit {
    Pass,
    Submit,
    Apply {
        text: String,
        start: usize,
        end: usize,
        caret: usize,
    },
}

fn apply_at(text: String, start: usize, end: usize, caret: usize) -> ComposerEdit {
    ComposerEdit::Apply {
        text,
        start,
        end,
        caret,
    }
}

fn apply_after(text: String, start: usize, end: usize) -> ComposerEdit {
    let caret = start + text.len();
    apply_at(text, start, end, caret)
}

/// One opening fence. `info_start..info_end` is the language token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FenceListing {
    pub line_index: usize,
    pub info: String,
    pub open_line_byte: usize,
    pub info_start: usize,
    pub info_end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineRole {
    Plain,
    FenceOpen,
    FenceClose,
    InFence,
}

struct LineSpan {
    start: usize,
    end: usize,
    next: usize,
    has_newline: bool,
}

/// `true` when the typed draft is too long to send. Attachments do not count.
pub fn exceeds_message_limit(draft: &str) -> bool {
    exceeds_message_limit_with(draft, 0)
}

/// Typed characters plus citation and terminal text that the label leaves out.
pub fn exceeds_message_limit_with(draft: &str, extra_chars: usize) -> bool {
    draft.chars().count().saturating_add(extra_chars) > MESSAGE_CHAR_LIMIT
}

/// Empty text inserts. `force_inline` inserts even when the paste is large.
/// A paste bigger than [`MAX_TEXT_ATTACHMENT_BYTES`] also inserts.
pub fn paste_outcome(draft: &str, paste: &str, force_inline: bool) -> PasteOutcome {
    if paste.is_empty() || force_inline || paste.len() > MAX_TEXT_ATTACHMENT_BYTES {
        return PasteOutcome::Insert;
    }
    if paste.len() >= LARGE_PASTE_BYTES {
        return PasteOutcome::Attach;
    }
    let chars = draft.chars().count().saturating_add(paste.chars().count());
    if chars > MESSAGE_CHAR_LIMIT {
        PasteOutcome::Attach
    } else {
        PasteOutcome::Insert
    }
}

/// `paste.txt`, then `paste-2.txt`, skipping names already in use.
pub fn paste_name<'a>(existing: impl IntoIterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = existing.into_iter().collect();
    if !taken.contains(&"paste.txt") {
        return "paste.txt".to_string();
    }
    let mut n = 2u32;
    loop {
        let candidate = format!("paste-{n}.txt");
        if !taken.iter().any(|name| *name == candidate) {
            return candidate;
        }
        n = n.saturating_add(1);
        if n == u32::MAX {
            return candidate;
        }
    }
}

/// A file name that cannot leave the paste directory. Anything else is
/// `paste.txt`.
pub fn safe_paste_filename(name: &str) -> String {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    if ok {
        name.to_string()
    } else {
        "paste.txt".to_string()
    }
}

/// Disk name under the paste directory. `unique` is a caller-chosen id.
pub fn paste_disk_name(unique: &str, name: &str) -> String {
    format!("{unique}-{}", safe_paste_filename(name))
}

/// The user message. With no attachments this is `typed` unchanged. With
/// attachments the typed text is trimmed and each file is named underneath.
pub fn visible_prompt(typed: &str, attachments: &[TextAttachment]) -> String {
    if attachments.is_empty() {
        return typed.to_string();
    }
    let typed = typed.trim();
    let mut out = String::new();
    if !typed.is_empty() {
        out.push_str(typed);
        out.push_str("\n\n");
    }
    for (index, attachment) in attachments.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str("Attached: ");
        out.push_str(&attachment.name);
    }
    out
}

/// The provider prompt. The pasted bodies stay in the files.
pub fn agent_prompt(visible: &str, files: &[(String, String)]) -> String {
    if files.is_empty() {
        return visible.to_string();
    }
    let mut out = String::from(visible);
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("Read these text files if you need the pasted text");
    for (name, path) in files {
        out.push('\n');
        out.push_str(name);
        out.push(' ');
        out.push_str(path);
    }
    out
}

/// Opening fences, in order.
pub fn fences(text: &str) -> Vec<FenceListing> {
    let lines = line_spans(text);
    scan(text, &lines).1
}

/// Rewrite the info string of the opening fence that starts at `open_line_start`.
/// `language` is empty or one of [`FENCE_LANGUAGES`].
pub fn set_fence_language(text: &str, open_line_start: usize, language: &str) -> Option<String> {
    if !language.is_empty() && !FENCE_LANGUAGES.contains(&language) {
        return None;
    }
    if language.contains(['`', '\n', '\r']) {
        return None;
    }
    let lines = line_spans(text);
    let (roles, _) = scan(text, &lines);
    let line = lines.iter().find(|line| line.start == open_line_start)?;
    if roles.get(line_number(&lines, line.start))? != &LineRole::FenceOpen {
        return None;
    }
    let content = &text[line.start..line.end];
    let raw = content.trim_end();
    let indent_len = raw.len() - raw.trim_start().len();
    let indent = &raw[..indent_len];
    let new_content = if language.is_empty() {
        format!("{indent}```")
    } else {
        format!("{indent}```{language}")
    };
    let mut out = String::with_capacity(text.len() + new_content.len());
    out.push_str(&text[..line.start]);
    out.push_str(&new_content);
    out.push_str(&text[line.end..]);
    Some(out)
}

/// Escaped HTML for the composer overlay. Empty text produces an empty string
/// so the textarea placeholder stays visible.
pub fn overlay_html(text: &str, caret_byte: usize) -> String {
    paint_overlay(text, caret_byte, &[])
}

/// [`overlay_html`] plus chip ranges. A chip is the same characters as its
/// label, with a background and no padding, so the caret stays aligned.
pub fn overlay_html_with_chips(text: &str, caret_byte: usize, chips: &[ContextChip]) -> String {
    paint_overlay(text, caret_byte, chips)
}

fn paint_overlay(text: &str, caret_byte: usize, chips: &[ContextChip]) -> String {
    if text.is_empty() {
        return String::new();
    }
    let caret = floor_boundary(text, caret_byte);
    let lines = line_spans(text);
    let (roles, _) = scan(text, &lines);
    let mut out = String::new();
    for (line, role) in lines.iter().zip(roles.iter()) {
        paint_line(&mut out, text, line, *role, caret, chips);
        if line.has_newline {
            out.push('\n');
        }
    }
    out
}

fn paint_line(
    out: &mut String,
    text: &str,
    line: &LineSpan,
    role: LineRole,
    caret: usize,
    chips: &[ContextChip],
) {
    let mut ranges: Vec<(usize, usize, usize, usize, String)> = chips
        .iter()
        .filter_map(|chip| {
            if chip.start >= line.end || chip.end <= line.start || chip.start >= chip.end {
                return None;
            }
            if chip.end > text.len()
                || !text.is_char_boundary(chip.start)
                || !text.is_char_boundary(chip.end)
            {
                return None;
            }
            let from = chip.start.max(line.start);
            let to = chip.end.min(line.end);
            if from >= to {
                return None;
            }
            Some((from, to, chip.start, chip.end, chip_detail(&chip.kind)))
        })
        .collect();
    ranges.sort_by_key(|range| range.0);
    if ranges.is_empty() {
        paint_role(out, text, line.start, line.end, line.start, role, caret);
        return;
    }
    let mut cursor = line.start;
    for (from, to, chip_start, chip_end, title) in ranges {
        if to <= cursor {
            continue;
        }
        let from = from.max(cursor);
        if from > cursor {
            paint_role(out, text, cursor, from, line.start, role, caret);
        }
        push_chip_span(out, &text[from..to], chip_start, chip_end, &title);
        cursor = to;
    }
    if cursor < line.end {
        paint_role(out, text, cursor, line.end, line.start, role, caret);
    }
}

fn paint_role(
    out: &mut String,
    text: &str,
    start: usize,
    end: usize,
    line_start: usize,
    role: LineRole,
    caret: usize,
) {
    if start >= end {
        return;
    }
    let content = &text[start..end];
    let caret_on_line = caret >= line_start && caret <= end && start == line_start;
    if start != line_start {
        let class = match role {
            LineRole::FenceOpen | LineRole::FenceClose | LineRole::InFence => {
                "bg-longan-800 text-rambutan-100"
            }
            LineRole::Plain => "text-rambutan-100",
        };
        push_span(out, class, content);
        return;
    }
    match role {
        LineRole::FenceOpen | LineRole::FenceClose | LineRole::InFence => {
            push_span(out, "bg-longan-800 text-rambutan-100", content);
        }
        LineRole::Plain => paint_plain(out, content, line_start, caret, caret_on_line),
    }
}

pub fn composer_edit(
    text: &str,
    selection_start: usize,
    selection_end: usize,
    key: ComposerKey,
) -> ComposerEdit {
    let mut start = floor_boundary(text, selection_start);
    let mut end = floor_boundary(text, selection_end);
    if start > end {
        std::mem::swap(&mut start, &mut end);
    }
    let lines = line_spans(text);
    let (roles, listings) = scan(text, &lines);
    match key {
        ComposerKey::Enter => edit_enter(text, &lines, &roles, start, end),
        ComposerKey::ShiftEnter => edit_shift_enter(text, &lines, &roles, start, end),
        ComposerKey::Tab => edit_tab(text, &lines, &roles, start, end, false),
        ComposerKey::ShiftTab => edit_tab(text, &lines, &roles, start, end, true),
        ComposerKey::Backspace => edit_backspace(&lines, &listings, start, end),
    }
}

fn edit_enter(
    text: &str,
    lines: &[LineSpan],
    roles: &[LineRole],
    start: usize,
    end: usize,
) -> ComposerEdit {
    if start != end {
        let last = end.saturating_sub(1);
        if role_at(lines, roles, start) == LineRole::InFence
            && role_at(lines, roles, last) == LineRole::InFence
        {
            let indent = leading_ws(
                &text[lines[line_index_at(lines, start)].start
                    ..lines[line_index_at(lines, start)].end],
            );
            return apply_after(format!("\n{indent}"), start, end);
        }
        return ComposerEdit::Submit;
    }
    let index = line_index_at(lines, start);
    match roles.get(index).copied().unwrap_or(LineRole::Plain) {
        LineRole::FenceOpen | LineRole::FenceClose => {
            apply_after("\n".to_string(), lines[index].end, lines[index].end)
        }
        LineRole::InFence => {
            if let Some(edit) = close_after_two_blanks(text, lines, roles, index) {
                return edit;
            }
            let line = &lines[index];
            let indent = leading_ws(&text[line.start..line.end]);
            apply_after(format!("\n{indent}"), start, end)
        }
        LineRole::Plain => ComposerEdit::Submit,
    }
}

fn close_after_two_blanks(
    text: &str,
    lines: &[LineSpan],
    roles: &[LineRole],
    index: usize,
) -> Option<ComposerEdit> {
    if index == 0 || index + 1 != lines.len() {
        return None;
    }
    let current = &lines[index];
    let previous = &lines[index - 1];
    let current_blank = text[current.start..current.end].trim().is_empty();
    let previous_blank = text[previous.start..previous.end].trim().is_empty();
    if current_blank
        && previous_blank
        && roles.get(index) == Some(&LineRole::InFence)
        && roles.get(index - 1) == Some(&LineRole::InFence)
    {
        Some(apply_after(
            "```\n".to_string(),
            previous.start,
            current.next,
        ))
    } else {
        None
    }
}

fn edit_shift_enter(
    text: &str,
    lines: &[LineSpan],
    roles: &[LineRole],
    start: usize,
    end: usize,
) -> ComposerEdit {
    if start != end {
        return ComposerEdit::Pass;
    }
    let index = line_index_at(lines, start);
    if roles.get(index) == Some(&LineRole::InFence) {
        return ComposerEdit::Pass;
    }
    let line = &lines[index];
    let content = &text[line.start..line.end];
    let Some(item) = list_item(content) else {
        return ComposerEdit::Pass;
    };
    if item.content.is_empty() {
        return apply_after(String::new(), line.start, line.end);
    }
    let marker = continued_marker(item.marker);
    apply_after(format!("\n{}{marker}", item.indent), start, end)
}

fn edit_tab(
    text: &str,
    lines: &[LineSpan],
    roles: &[LineRole],
    start: usize,
    end: usize,
    outdent: bool,
) -> ComposerEdit {
    let indexes = intersecting(lines, start, end);
    if indexes.is_empty() {
        return ComposerEdit::Pass;
    }
    let qualifies = indexes.iter().all(|&index| {
        let content = &text[lines[index].start..lines[index].end];
        matches!(roles.get(index), Some(LineRole::InFence))
            || list_item(content).is_some()
            || content.trim().is_empty()
    });
    let meaningful = indexes.iter().any(|&index| {
        let content = &text[lines[index].start..lines[index].end];
        matches!(roles.get(index), Some(LineRole::InFence)) || list_item(content).is_some()
    });
    if !qualifies || !meaningful {
        return ComposerEdit::Pass;
    }
    let first = indexes[0];
    let last = indexes[indexes.len() - 1];
    let mut changed = false;
    let mut next = String::new();
    for (offset, index) in indexes.iter().copied().enumerate() {
        if offset > 0 {
            next.push('\n');
        }
        let content = &text[lines[index].start..lines[index].end];
        let mapped = if outdent {
            dedent(content)
        } else {
            format!("  {content}")
        };
        if mapped != content {
            changed = true;
        }
        next.push_str(&mapped);
    }
    if outdent && !changed {
        return ComposerEdit::Pass;
    }
    let origin = lines[first].start;
    let caret = if start == end {
        let content = &text[lines[first].start..lines[first].end];
        let removed = if outdent {
            content.len() - dedent(content).len()
        } else {
            0
        };
        let added = if outdent { 0 } else { 2 };
        let at = start.saturating_sub(origin);
        origin + at - removed.min(at) + added
    } else {
        origin + next.len()
    };
    apply_at(next, origin, lines[last].end, caret)
}

fn edit_backspace(
    lines: &[LineSpan],
    listings: &[FenceListing],
    start: usize,
    end: usize,
) -> ComposerEdit {
    if start != end {
        return ComposerEdit::Pass;
    }
    for fence in listings {
        let Some(open) = lines.iter().find(|line| line.start == fence.open_line_byte) else {
            continue;
        };
        let content_start = if open.has_newline {
            open.next
        } else {
            open.end
        };
        if start == open.start || start == content_start {
            return apply_after(String::new(), open.start, content_start);
        }
    }
    ComposerEdit::Pass
}

struct Item<'a> {
    indent: &'a str,
    marker: &'a str,
    content: &'a str,
}

fn list_item(line: &str) -> Option<Item<'_>> {
    let indent_len = line.len() - line.trim_start().len();
    let indent = &line[..indent_len];
    let rest = &line[indent_len..];
    let marker_len = task_marker(rest)
        .or_else(|| bullet_marker(rest))
        .or_else(|| ordered_marker(rest))
        .or_else(|| quote_marker(rest))?;
    Some(Item {
        indent,
        marker: &rest[..marker_len],
        content: &rest[marker_len..],
    })
}

fn task_marker(rest: &str) -> Option<usize> {
    ["- [ ] ", "- [x] ", "- [X] "]
        .into_iter()
        .find(|prefix| rest.starts_with(prefix))
        .map(|prefix| prefix.len())
}

fn bullet_marker(rest: &str) -> Option<usize> {
    rest.starts_with("- ").then_some(2)
}

fn ordered_marker(rest: &str) -> Option<usize> {
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    rest[digits..].starts_with(". ").then_some(digits + 2)
}

fn quote_marker(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() && bytes[index] == b'>' && bytes[index + 1] == b' ' {
        index += 2;
    }
    (index > 0).then_some(index)
}

fn continued_marker(marker: &str) -> String {
    if task_marker(marker).is_some() {
        return "- [ ] ".to_string();
    }
    if let Some(digits) = marker.strip_suffix(". ") {
        if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            let next = digits.parse::<u64>().map_or(digits.to_string(), |value| {
                value.saturating_add(1).to_string()
            });
            return format!("{next}. ");
        }
    }
    marker.to_string()
}

fn dedent(line: &str) -> String {
    if let Some(rest) = line.strip_prefix('\t') {
        return rest.to_string();
    }
    let spaces = line
        .bytes()
        .take_while(|byte| *byte == b' ')
        .take(2)
        .count();
    line[spaces..].to_string()
}

fn intersecting(lines: &[LineSpan], start: usize, end: usize) -> Vec<usize> {
    if start == end {
        return vec![line_index_at(lines, start)];
    }
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let bound = if line.has_newline {
                line.next
            } else {
                line.end
            };
            line.start < end && start < bound
        })
        .map(|(index, _)| index)
        .collect()
}

fn role_at(lines: &[LineSpan], roles: &[LineRole], pos: usize) -> LineRole {
    roles
        .get(line_index_at(lines, pos))
        .copied()
        .unwrap_or(LineRole::Plain)
}

fn paint_plain(
    out: &mut String,
    content: &str,
    line_start: usize,
    caret: usize,
    caret_on_line: bool,
) {
    if let Some(mark_len) = heading_marker_len(content) {
        let hash_class = if caret_on_line {
            "text-rambutan-400"
        } else {
            "text-rambutan-500"
        };
        push_span(out, hash_class, &content[..mark_len]);
        push_span(out, "text-persimmon-300", &content[mark_len..]);
        return;
    }
    if is_rule(content) {
        push_span(out, "text-rambutan-600", content);
        return;
    }
    if quote_marker(content.trim_start()).is_some() {
        push_span(out, "text-rambutan-300", content);
        return;
    }
    inline_into(out, content, line_start, caret);
}

fn inline_into(out: &mut String, content: &str, line_start: usize, caret: usize) {
    let mut plain = String::new();
    let mut index = 0;
    while index < content.len() {
        if content[index..].starts_with("```") {
            let ticks = content[index..]
                .bytes()
                .take_while(|byte| *byte == b'`')
                .count();
            plain.push_str(&content[index..index + ticks]);
            index += ticks;
            continue;
        }
        if content[index..].starts_with("**") {
            if let Some(close) = find_token(&content[index + 2..], "**") {
                flush_plain(out, &mut plain);
                let inner_start = index + 2;
                let inner_end = inner_start + close;
                let marker_end = inner_end + 2;
                push_marker(
                    out,
                    &content[index..inner_start],
                    line_start + index,
                    line_start + inner_start,
                    caret,
                );
                push_span(out, "text-rambutan-100", &content[inner_start..inner_end]);
                push_marker(
                    out,
                    &content[inner_end..marker_end],
                    line_start + inner_end,
                    line_start + marker_end,
                    caret,
                );
                index = marker_end;
                continue;
            }
        }
        if content.as_bytes()[index] == b'`' {
            if let Some(close) = find_token(&content[index + 1..], "`") {
                flush_plain(out, &mut plain);
                let inner_start = index + 1;
                let inner_end = inner_start + close;
                let marker_end = inner_end + 1;
                push_marker(
                    out,
                    "`",
                    line_start + index,
                    line_start + inner_start,
                    caret,
                );
                push_span(
                    out,
                    "bg-longan-800 text-rambutan-100",
                    &content[inner_start..inner_end],
                );
                push_marker(
                    out,
                    "`",
                    line_start + inner_end,
                    line_start + marker_end,
                    caret,
                );
                index = marker_end;
                continue;
            }
        }
        let ch = content[index..].chars().next().unwrap_or('\0');
        plain.push(ch);
        index += ch.len_utf8();
    }
    flush_plain(out, &mut plain);
}

fn find_token(haystack: &str, token: &str) -> Option<usize> {
    haystack
        .find(token)
        .filter(|at| haystack.is_char_boundary(*at))
}

fn push_marker(out: &mut String, text: &str, start: usize, end: usize, caret: usize) {
    let class = if caret >= start && caret <= end {
        "text-rambutan-400"
    } else {
        "text-transparent"
    };
    push_span(out, class, text);
}

fn flush_plain(out: &mut String, plain: &mut String) {
    if plain.is_empty() {
        return;
    }
    push_span(out, "text-rambutan-100", plain);
    plain.clear();
}

fn push_span(out: &mut String, class: &str, text: &str) {
    if text.is_empty() {
        return;
    }
    out.push_str("<span class=\"");
    out.push_str(class);
    out.push_str("\">");
    push_escaped(out, text);
    out.push_str("</span>");
}

fn push_chip_span(out: &mut String, text: &str, start: usize, end: usize, title: &str) {
    if text.is_empty() {
        return;
    }
    out.push_str(
        "<span class=\"pointer-events-auto bg-longan-800 text-rambutan-100\" data-chip-start=\"",
    );
    out.push_str(&start.to_string());
    out.push_str("\" data-chip-end=\"");
    out.push_str(&end.to_string());
    out.push_str("\" title=\"");
    push_escaped(out, title);
    out.push_str("\">");
    push_escaped(out, text);
    out.push_str("</span>");
}

fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

fn heading_marker_len(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ') {
        Some(hashes + 1)
    } else {
        None
    }
}

fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3 && trimmed.bytes().all(|byte| byte == b'-')
}

fn leading_ws(line: &str) -> &str {
    let end = line.len() - line.trim_start().len();
    &line[..end]
}

fn line_spans(text: &str) -> Vec<LineSpan> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            lines.push(LineSpan {
                start,
                end: index,
                next: index + 1,
                has_newline: true,
            });
            start = index + 1;
        }
    }
    lines.push(LineSpan {
        start,
        end: text.len(),
        next: text.len(),
        has_newline: false,
    });
    lines
}

fn line_index_at(lines: &[LineSpan], pos: usize) -> usize {
    lines
        .iter()
        .position(|line| pos <= line.end)
        .unwrap_or(lines.len().saturating_sub(1))
}

fn line_number(lines: &[LineSpan], start: usize) -> usize {
    lines
        .iter()
        .position(|line| line.start == start)
        .unwrap_or(0)
}

fn scan(text: &str, lines: &[LineSpan]) -> (Vec<LineRole>, Vec<FenceListing>) {
    let mut roles = Vec::with_capacity(lines.len());
    let mut listings = Vec::new();
    let mut open = false;
    for (index, line) in lines.iter().enumerate() {
        let content = &text[line.start..line.end];
        if let Some((info, info_start, info_end)) = fence_parts(content, line.start) {
            if open {
                roles.push(LineRole::FenceClose);
                open = false;
            } else {
                roles.push(LineRole::FenceOpen);
                open = true;
                listings.push(FenceListing {
                    line_index: index,
                    info,
                    open_line_byte: line.start,
                    info_start,
                    info_end,
                });
            }
        } else if open {
            roles.push(LineRole::InFence);
        } else {
            roles.push(LineRole::Plain);
        }
    }
    (roles, listings)
}

fn fence_parts(line: &str, line_start: usize) -> Option<(String, usize, usize)> {
    let raw = line.trim_end();
    let indent_len = raw.len() - raw.trim_start().len();
    let body = raw.trim_start();
    let after = body.strip_prefix("```")?;
    if after.contains('`') {
        return None;
    }
    let trimmed = after.trim();
    let lead = after.len() - after.trim_start().len();
    let info_start = line_start + indent_len + 3 + lead;
    let info_end = info_start + trimmed.len();
    Some((trimmed.to_string(), info_start, info_end))
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    if index > text.len() {
        index = text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(text: &str, start: usize, end: usize, key: ComposerKey) -> (String, usize) {
        match composer_edit(text, start, end, key) {
            ComposerEdit::Apply {
                text: insert,
                start,
                end,
                caret,
            } => {
                let mut next = String::new();
                next.push_str(&text[..start]);
                next.push_str(&insert);
                next.push_str(&text[end..]);
                (next, caret)
            }
            other => panic!("expected apply, got {other:?}"),
        }
    }

    #[test]
    fn paste_thresholds() {
        assert_eq!(paste_outcome("hi", "", false), PasteOutcome::Insert);
        assert_eq!(paste_outcome("hi", "there", false), PasteOutcome::Insert);
        let large = "x".repeat(LARGE_PASTE_BYTES);
        assert_eq!(paste_outcome("", &large, false), PasteOutcome::Attach);
        assert_eq!(
            paste_outcome("", &large, true),
            PasteOutcome::Insert,
            "cmd-shift-v stays inline"
        );
        let almost = "x".repeat(LARGE_PASTE_BYTES - 1);
        assert_eq!(paste_outcome("", &almost, false), PasteOutcome::Insert);
        let room = "a".repeat(MESSAGE_CHAR_LIMIT - 2);
        assert_eq!(paste_outcome(&room, "abcd", false), PasteOutcome::Attach);
        assert!(exceeds_message_limit(&"a".repeat(MESSAGE_CHAR_LIMIT + 1)));
        assert!(!exceeds_message_limit(&"a".repeat(MESSAGE_CHAR_LIMIT)));
    }

    #[test]
    fn paste_names_and_prompts() {
        assert_eq!(paste_name(["note.txt"]), "paste.txt");
        assert_eq!(paste_name(["paste.txt"]), "paste-2.txt");
        assert_eq!(paste_name(["paste.txt", "paste-2.txt"]), "paste-3.txt");
        assert_eq!(safe_paste_filename("notes.txt"), "notes.txt");
        assert_eq!(safe_paste_filename("../etc/passwd"), "paste.txt");
        assert_eq!(safe_paste_filename(".."), "paste.txt");
        assert_eq!(paste_disk_name("abc", "notes.txt"), "abc-notes.txt");
        let files = [TextAttachment {
            name: "paste.txt".into(),
            text: "body".into(),
        }];
        assert_eq!(
            visible_prompt("  hello  ", &files),
            "hello\n\nAttached: paste.txt"
        );
        assert_eq!(visible_prompt("   ", &files), "Attached: paste.txt");
        assert_eq!(visible_prompt("  hello  ", &[]), "  hello  ");
        assert_eq!(
            agent_prompt("hello\n\nAttached: paste.txt", &[("paste.txt".into(), "/tmp/p".into())]),
            "hello\n\nAttached: paste.txt\n\nRead these text files if you need the pasted text\npaste.txt /tmp/p"
        );
        assert_eq!(agent_prompt("hello", &[]), "hello");
    }

    #[test]
    fn enter_opens_and_closes_fences_without_sending() {
        assert!(matches!(
            composer_edit("hello", 5, 5, ComposerKey::Enter),
            ComposerEdit::Submit
        ));
        let (text, caret) = applied("```rust", 3, 3, ComposerKey::Enter);
        assert_eq!(text, "```rust\n");
        assert_eq!(caret, 8);
        let (text, caret) = applied("```rust\nlet x = 1;\n```", 20, 20, ComposerKey::Enter);
        assert_eq!(text, "```rust\nlet x = 1;\n```\n");
        assert_eq!(caret, text.len());
        let (text, caret) = applied("```rust\nlet x = 1;\n\n", 20, 20, ComposerKey::Enter);
        assert_eq!(text, "```rust\nlet x = 1;\n```\n");
        assert_eq!(caret, text.len());
        let inside = applied("```rust\n  let x = 1;", 20, 20, ComposerKey::Enter);
        assert_eq!(inside.0, "```rust\n  let x = 1;\n  ");
        assert_eq!(inside.1, inside.0.len());
        assert!(matches!(
            composer_edit("```\ncode\n```\nstill", 4, 8, ComposerKey::Enter),
            ComposerEdit::Apply { .. }
        ));
        assert!(matches!(
            composer_edit("```\ncode\n```\nstill", 13, 18, ComposerKey::Enter),
            ComposerEdit::Submit
        ));
    }

    #[test]
    fn shift_enter_continues_lists_and_leaves_empty_ones() {
        let (text, caret) = applied("- alpha", 7, 7, ComposerKey::ShiftEnter);
        assert_eq!(text, "- alpha\n- ");
        assert_eq!(caret, text.len());
        let (text, _) = applied("- alpha", 3, 3, ComposerKey::ShiftEnter);
        assert_eq!(text, "- a\n- lpha");
        let (text, caret) = applied("12. item", 8, 8, ComposerKey::ShiftEnter);
        assert_eq!(text, "12. item\n13. ");
        assert_eq!(caret, text.len());
        let (text, _) = applied("- [x] done", 11, 11, ComposerKey::ShiftEnter);
        assert_eq!(text, "- [x] done\n- [ ] ");
        let (text, _) = applied("> > quote", 9, 9, ComposerKey::ShiftEnter);
        assert_eq!(text, "> > quote\n> > ");
        let (text, caret) = applied("  - ", 4, 4, ComposerKey::ShiftEnter);
        assert_eq!(text, "");
        assert_eq!(caret, 0);
        let (text, caret) = applied("- a\n- \n- b", 5, 5, ComposerKey::ShiftEnter);
        assert_eq!(text, "- a\n\n- b");
        assert_eq!(caret, 4);
        assert!(matches!(
            composer_edit("# heading", 10, 10, ComposerKey::ShiftEnter),
            ComposerEdit::Pass
        ));
        assert!(matches!(
            composer_edit("---", 3, 3, ComposerKey::ShiftEnter),
            ComposerEdit::Pass
        ));
        assert!(matches!(
            composer_edit("- alpha", 0, 7, ComposerKey::ShiftEnter),
            ComposerEdit::Pass
        ));
        assert!(matches!(
            composer_edit("- alpha", 7, 7, ComposerKey::Enter),
            ComposerEdit::Submit
        ));
    }

    #[test]
    fn tab_indents_lists_and_code_and_backspace_opens_a_fence() {
        let (text, _) = applied("- a\n- b", 0, 6, ComposerKey::Tab);
        assert_eq!(text, "  - a\n  - b");
        let (text, _) = applied("  - a\n  - b", 0, 10, ComposerKey::ShiftTab);
        assert_eq!(text, "- a\n- b");
        assert!(matches!(
            composer_edit("- a", 2, 2, ComposerKey::ShiftTab),
            ComposerEdit::Pass
        ));
        assert!(matches!(
            composer_edit("plain", 0, 5, ComposerKey::Tab),
            ComposerEdit::Pass
        ));
        let (text, _) = applied("```\ncode", 4, 4, ComposerKey::Tab);
        assert_eq!(text, "```\n  code");
        assert!(matches!(
            composer_edit("```rust", 0, 0, ComposerKey::Tab),
            ComposerEdit::Pass
        ));
        let (text, caret) = applied("```rust\nhello", 0, 0, ComposerKey::Backspace);
        assert_eq!(text, "hello");
        assert_eq!(caret, 0);
        let (text, _) = applied("```rust\nhello", 8, 8, ComposerKey::Backspace);
        assert_eq!(text, "hello");
        assert!(matches!(
            composer_edit("```rust\nhello", 4, 4, ComposerKey::Backspace),
            ComposerEdit::Pass
        ));
    }

    #[test]
    fn overlay_colors_markers_without_weight_or_size() {
        let away = overlay_html("**hi** `<b>`", 3);
        assert!(away.contains("text-transparent"));
        assert!(away.contains("text-rambutan-100"));
        assert!(away.contains("&lt;b&gt;"));
        assert!(!away.contains("<b>"));
        assert!(!away.contains("font-"));
        let on_marker = overlay_html("**hi**", 1);
        assert!(on_marker.contains("text-rambutan-400"));
        let heading = overlay_html("note\n# Title", 0);
        assert!(heading.contains("text-rambutan-500"));
        assert!(heading.contains("text-persimmon-300"));
        let editing = overlay_html("# Title", 2);
        assert!(editing.contains("text-rambutan-400"));
        assert!(!overlay_html("#1234", 0).contains("text-persimmon-300"));
        assert!(overlay_html("> quoted", 0).contains("text-rambutan-300"));
        assert!(overlay_html("---", 0).contains("text-rambutan-600"));
        assert!(overlay_html("```rust\nlet x = 1;", 0).contains("bg-longan-800"));
        assert_eq!(overlay_html("", 0), "");
        let chip = crate::logic::context_chip::ContextChip {
            id: "p".into(),
            start: 4,
            end: 8,
            kind: crate::logic::context_chip::ChipKind::PullRequest {
                number: 128,
                title: "Open the gate".into(),
                url: "https://example/128".into(),
                state: "open".into(),
            },
        };
        let painted = overlay_html_with_chips("see #128 next", 0, &[chip]);
        assert!(painted.contains("data-chip-start=\"4\""));
        assert!(painted.contains(">#128</span>"));
        assert!(!painted.contains("font-"));
        assert!(!painted.contains("px-"));
        assert_eq!(
            overlay_html_with_chips("**hi**", 0, &[]),
            overlay_html("**hi**", 0)
        );
    }

    #[test]
    fn fence_language_rewrites_the_opening_line() {
        let found = fences("```rust\nlet x = 1;\n```");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].info, "rust");
        assert_eq!(found[0].line_index, 0);
        assert_eq!(
            set_fence_language("```rust\nlet x = 1;", 0, "python").as_deref(),
            Some("```python\nlet x = 1;")
        );
        assert_eq!(
            set_fence_language("```\nlet x = 1;", 0, "text").as_deref(),
            Some("```text\nlet x = 1;")
        );
        assert!(set_fence_language("```rust\n```", 8, "python").is_none());
        assert!(set_fence_language("```rust", 0, "nope").is_none());
    }

    #[test]
    fn hash_without_a_space_is_not_a_heading_and_fences_ignore_extra_ticks() {
        assert!(matches!(
            composer_edit("#1234", 5, 5, ComposerKey::Enter),
            ComposerEdit::Submit
        ));
        assert!(fences("````rust").is_empty());
        assert_eq!(fences("``` rust").len(), 1);
        assert_eq!(fences("``` rust")[0].info, "rust");
    }
}
