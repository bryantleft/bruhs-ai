//! Inline composer context: a citation, file, pull request, thread, or
//! terminal excerpt that occupies a byte range of the draft.
//!
//! The draft text is the visible label. The chip record keeps what the label
//! leaves out (the quote, the pull-request title, the thread id). An edit
//! that touches a chip removes that chip. An edit outside it shifts the range.

use serde::{Deserialize, Serialize};

/// What an inline chip points at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChipKind {
    File {
        path: String,
    },
    Citation {
        message_id: String,
        quote: String,
        comment: String,
    },
    PullRequest {
        number: u64,
        title: String,
        url: String,
        state: String,
    },
    Thread {
        thread_id: String,
        title: String,
    },
    Terminal {
        label: String,
        excerpt: String,
    },
}

/// One chip. `start` and `end` are UTF-8 byte offsets into the current draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextChip {
    pub id: String,
    pub start: usize,
    pub end: usize,
    pub kind: ChipKind,
}

/// The characters the draft shows for `kind`.
pub fn chip_label(kind: &ChipKind) -> String {
    match kind {
        ChipKind::File { path } => {
            if path.starts_with('@') {
                path.clone()
            } else {
                format!("@{path}")
            }
        }
        ChipKind::Citation { quote, comment, .. } => {
            let source = if comment.trim().is_empty() {
                quote.as_str()
            } else {
                comment.as_str()
            };
            let preview: String = source.chars().take(48).collect();
            if source.chars().count() > 48 {
                format!("“{preview}…”")
            } else {
                format!("“{preview}”")
            }
        }
        ChipKind::PullRequest { number, .. } => format!("#{number}"),
        ChipKind::Thread { title, .. } => format!("@{title}"),
        ChipKind::Terminal { label, .. } => label.clone(),
    }
}

/// Insert `kind` at `caret`. A trailing space follows the label when the
/// draft does not already continue with whitespace. Existing chips shift.
pub fn insert_chip(
    text: &str,
    chips: &[ContextChip],
    caret: usize,
    id: impl Into<String>,
    kind: ChipKind,
) -> (String, usize, Vec<ContextChip>) {
    let caret = floor_boundary(text, caret);
    let label = chip_label(&kind);
    let after = &text[caret..];
    let needs_space = !after.starts_with(|ch: char| ch.is_whitespace());
    let mut inserted = label;
    if needs_space {
        inserted.push(' ');
    }
    let end = caret + inserted.trim_end().len();
    let chip = ContextChip {
        id: id.into(),
        start: caret,
        end,
        kind,
    };
    let (next, mut kept) = replace_range(text, chips, caret, caret, &inserted);
    kept.push(chip);
    kept.sort_by_key(|item| item.start);
    (next, caret + inserted.len(), kept)
}

/// Backspace removes the whole chip when the caret is inside it or on the
/// byte just after it.
pub fn backspace_chip(
    text: &str,
    chips: &[ContextChip],
    caret: usize,
) -> Option<(String, usize, Vec<ContextChip>)> {
    let caret = floor_boundary(text, caret);
    let chip = chips
        .iter()
        .find(|chip| caret > chip.start && caret <= chip.end)?;
    let start = chip.start;
    let (next, kept) = replace_range(text, chips, chip.start, chip.end, "");
    Some((next, start, kept))
}

/// Splice `inserted` over `[start, end)` and drop or shift chips to match.
pub fn replace_range(
    text: &str,
    chips: &[ContextChip],
    start: usize,
    end: usize,
    inserted: &str,
) -> (String, Vec<ContextChip>) {
    let start = floor_boundary(text, start.min(text.len()));
    let end = floor_boundary(text, end.clamp(start, text.len()));
    let mut next = String::with_capacity(text.len() - (end - start) + inserted.len());
    next.push_str(&text[..start]);
    next.push_str(inserted);
    next.push_str(&text[end..]);
    let delta = len_delta(end - start, inserted.len());
    let kept = chips
        .iter()
        .filter_map(|chip| retarget_one(chip, start, end, delta))
        .collect();
    (next, kept)
}

/// Rebase chips after `old` became `new`. A chip the edit touches is dropped.
pub fn retarget(chips: &[ContextChip], old: &str, new: &str) -> Vec<ContextChip> {
    if old == new {
        return chips.to_vec();
    }
    let prefix = common_prefix(old, new);
    let suffix = common_suffix(old, new, prefix);
    let old_end = old.len() - suffix;
    let new_mid = new.len() - suffix - prefix;
    let delta = len_delta(old_end - prefix, new_mid);
    chips
        .iter()
        .filter_map(|chip| retarget_one(chip, prefix, old_end, delta))
        .collect()
}

/// Words the agent should read. Chip labels in `text` are replaced from the
/// end so earlier ranges stay put.
pub fn expand_prompt(text: &str, chips: &[ContextChip]) -> String {
    let mut ordered: Vec<&ContextChip> = chips.iter().collect();
    ordered.sort_by_key(|chip| chip.start);
    let mut out = text.to_string();
    for chip in ordered.into_iter().rev() {
        if chip.end > out.len()
            || chip.start > chip.end
            || !out.is_char_boundary(chip.start)
            || !out.is_char_boundary(chip.end)
        {
            continue;
        }
        let body = agent_body(&chip.kind);
        out.replace_range(chip.start..chip.end, &body);
    }
    out
}

/// Markdown another app can read. Each chip becomes a link.
pub fn markdown_for_copy(text: &str, chips: &[ContextChip]) -> String {
    let mut ordered: Vec<&ContextChip> = chips.iter().collect();
    ordered.sort_by_key(|chip| chip.start);
    let mut out = text.to_string();
    for chip in ordered.into_iter().rev() {
        if chip.end > out.len()
            || chip.start > chip.end
            || !out.is_char_boundary(chip.start)
            || !out.is_char_boundary(chip.end)
        {
            continue;
        }
        let link = copy_link(&chip.kind);
        out.replace_range(chip.start..chip.end, &link);
    }
    out
}

/// Hover text for a chip. The draft still shows [`chip_label`].
pub fn chip_detail(kind: &ChipKind) -> String {
    match kind {
        ChipKind::File { path } => path.clone(),
        ChipKind::Citation { quote, comment, .. } => {
            if comment.trim().is_empty() {
                quote.clone()
            } else {
                format!("{comment}\n{quote}")
            }
        }
        ChipKind::PullRequest {
            number,
            title,
            state,
            ..
        } => format!("#{number} {title} ({state})"),
        ChipKind::Thread { title, .. } => title.clone(),
        ChipKind::Terminal { label, excerpt } => {
            let line = excerpt.lines().next().unwrap_or("");
            if line.is_empty() {
                label.clone()
            } else {
                format!("{label}: {line}")
            }
        }
    }
}

/// Drop leading and trailing whitespace and shift chips to match.
pub fn trim_with_chips(text: &str, chips: &[ContextChip]) -> (String, Vec<ContextChip>) {
    let lead = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    if lead >= end {
        return (String::new(), Vec::new());
    }
    if lead == 0 && end == text.len() {
        return (text.to_string(), chips.to_vec());
    }
    let (mid, kept) = replace_range(text, chips, end, text.len(), "");
    replace_range(&mid, &kept, 0, lead, "")
}

/// Append a stashed draft. `blank_line` inserts a blank line before it
/// (web). A single newline matches the desktop composer.
pub fn join_drafts(
    draft: &str,
    draft_chips: &[ContextChip],
    stashed: &str,
    stashed_chips: &[ContextChip],
    blank_line: bool,
) -> (String, Vec<ContextChip>) {
    if draft.trim().is_empty() {
        return (stashed.to_string(), stashed_chips.to_vec());
    }
    let end = draft.trim_end().len();
    let glue = if blank_line { "\n\n" } else { "\n" };
    let inserted = format!("{glue}{stashed}");
    let (next, mut kept) = replace_range(draft, draft_chips, end, draft.len(), &inserted);
    let base = end + glue.len();
    for chip in stashed_chips {
        if chip.start > chip.end || chip.end > stashed.len() {
            continue;
        }
        kept.push(ContextChip {
            start: base + chip.start,
            end: base + chip.end,
            ..chip.clone()
        });
    }
    kept.sort_by_key(|chip| chip.start);
    (next, kept)
}

/// Replace one chip's payload and the label it occupies.
pub fn replace_kind(
    text: &str,
    chips: &[ContextChip],
    id: &str,
    kind: ChipKind,
) -> Option<(String, Vec<ContextChip>)> {
    let chip = chips.iter().find(|chip| chip.id == id)?;
    let start = chip.start;
    let label = chip_label(&kind);
    let (next, mut kept) = replace_range(text, chips, chip.start, chip.end, &label);
    kept.push(ContextChip {
        id: id.to_string(),
        start,
        end: start + label.len(),
        kind,
    });
    kept.sort_by_key(|chip| chip.start);
    Some((next, kept))
}

/// Forward delete removes the whole chip when the caret is at its start or
/// inside it, and not on the byte just after it.
pub fn delete_forward_chip(
    text: &str,
    chips: &[ContextChip],
    caret: usize,
) -> Option<(String, usize, Vec<ContextChip>)> {
    let caret = floor_boundary(text, caret);
    let chip = chips
        .iter()
        .find(|chip| caret >= chip.start && caret < chip.end)?;
    let start = chip.start;
    let (next, kept) = replace_range(text, chips, chip.start, chip.end, "");
    Some((next, start, kept))
}

/// Clipboard payload for a paste that should restore chips. `text` is the
/// label slice. Other apps receive Markdown instead.
pub const CHIP_CLIPBOARD_MIME: &str = "application/x-bruhs-chips";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChipClipboard {
    pub text: String,
    pub chips: Vec<ContextChip>,
}

/// Quote, comment, and terminal excerpt characters that are not already in
/// the visible label. They count toward the 120,000-character message limit.
pub fn extra_limit_chars(chips: &[ContextChip]) -> usize {
    chips
        .iter()
        .map(|chip| match &chip.kind {
            ChipKind::Citation { quote, comment, .. } => {
                quote.chars().count() + comment.chars().count()
            }
            ChipKind::Terminal { excerpt, .. } => excerpt.chars().count(),
            _ => 0,
        })
        .sum()
}

fn agent_body(kind: &ChipKind) -> String {
    match kind {
        ChipKind::File { path } => format!("File: {path}"),
        ChipKind::Citation { quote, comment, .. } => {
            if comment.is_empty() {
                format!("Quoted from the assistant: {quote}")
            } else {
                format!("Quoted from the assistant: {quote}\nComment: {comment}")
            }
        }
        ChipKind::PullRequest {
            number,
            title,
            url,
            state,
        } => format!("Pull request #{number} ({state}): {title} {url}"),
        ChipKind::Thread { thread_id, title } => {
            format!("Thread \"{title}\" ({thread_id})")
        }
        ChipKind::Terminal { label, excerpt } => format!("{label}:\n{excerpt}"),
    }
}

fn copy_link(kind: &ChipKind) -> String {
    match kind {
        ChipKind::File { path } => format!("[{path}](file://{path})"),
        ChipKind::Citation { quote, comment, .. } => {
            if comment.is_empty() {
                format!("“{quote}”")
            } else {
                format!("“{quote}” ({comment})")
            }
        }
        ChipKind::PullRequest { number, url, .. } => format!("[#{number}]({url})"),
        ChipKind::Thread { title, thread_id } => format!("[{title}](bruhs-thread://{thread_id})"),
        ChipKind::Terminal { label, .. } => label.clone(),
    }
}

fn len_delta(removed: usize, inserted: usize) -> i64 {
    i64::try_from(inserted).unwrap_or(i64::MAX) - i64::try_from(removed).unwrap_or(i64::MAX)
}

fn retarget_one(
    chip: &ContextChip,
    edit_start: usize,
    edit_end: usize,
    delta: i64,
) -> Option<ContextChip> {
    if chip.end <= edit_start {
        return Some(chip.clone());
    }
    if chip.start >= edit_end {
        return Some(ContextChip {
            start: shift(chip.start, delta),
            end: shift(chip.end, delta),
            ..chip.clone()
        });
    }
    None
}

fn shift(offset: usize, delta: i64) -> usize {
    if delta >= 0 {
        offset.saturating_add(usize::try_from(delta).unwrap_or(usize::MAX))
    } else {
        offset.saturating_sub(usize::try_from(delta.unsigned_abs()).unwrap_or(usize::MAX))
    }
}

fn common_prefix(left: &str, right: &str) -> usize {
    let mut bytes = 0;
    for (a, b) in left.chars().zip(right.chars()) {
        if a != b {
            break;
        }
        bytes += a.len_utf8();
    }
    bytes
}

fn common_suffix(left: &str, right: &str, prefix: usize) -> usize {
    let left = &left[prefix..];
    let right = &right[prefix..];
    let mut bytes = 0;
    for (a, b) in left.chars().rev().zip(right.chars().rev()) {
        if a != b {
            break;
        }
        bytes += a.len_utf8();
    }
    bytes
}

fn floor_boundary(text: &str, index: usize) -> usize {
    let index = index.min(text.len());
    if text.is_char_boundary(index) {
        index
    } else {
        let mut at = index;
        while at > 0 && !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(start: usize, end: usize) -> ContextChip {
        ContextChip {
            id: "p".into(),
            start,
            end,
            kind: ChipKind::PullRequest {
                number: 128,
                title: "Open the gate".into(),
                url: "https://github.com/example/repo/pull/128".into(),
                state: "open".into(),
            },
        }
    }

    #[test]
    fn insert_shifts_a_later_chip_and_lands_at_the_caret() {
        let existing = pr(5, 9);
        let (text, caret, chips) = insert_chip(
            "keep #128",
            &[existing],
            0,
            "c",
            ChipKind::Thread {
                thread_id: "t1".into(),
                title: "Notes".into(),
            },
        );
        assert_eq!(text, "@Notes keep #128");
        assert_eq!(caret, 7);
        assert_eq!(&text[chips[0].start..chips[0].end], "@Notes");
        assert_eq!(&text[chips[1].start..chips[1].end], "#128");
    }

    #[test]
    fn backspace_removes_the_whole_chip() {
        let text = "see #128 next";
        let (next, caret, chips) = backspace_chip(text, &[pr(4, 8)], 8).unwrap();
        assert_eq!(next, "see  next");
        assert_eq!(caret, 4);
        assert!(chips.is_empty());
    }

    #[test]
    fn an_edit_inside_a_chip_drops_it_and_an_edit_before_it_shifts_it() {
        let chips = vec![pr(4, 8)];
        let shifted = retarget(&chips, "see #128", "see  #128");
        assert_eq!(shifted[0].start, 5);
        assert_eq!(shifted[0].end, 9);
        let dropped = retarget(&chips, "see #128", "see #129");
        assert!(dropped.is_empty());
    }

    #[test]
    fn expand_and_copy_replace_the_label() {
        let chips = vec![pr(0, 4)];
        let expanded = expand_prompt("#128", &chips);
        assert!(expanded.contains("Open the gate"));
        assert!(expanded.contains("https://github.com/example/repo/pull/128"));
        assert_eq!(
            markdown_for_copy("see #128", &[pr(4, 8)]),
            "see [#128](https://github.com/example/repo/pull/128)"
        );
    }

    #[test]
    fn citation_quote_counts_toward_the_limit() {
        let chips = vec![ContextChip {
            id: "c".into(),
            start: 0,
            end: 3,
            kind: ChipKind::Citation {
                message_id: "m".into(),
                quote: "hello there".into(),
                comment: "why".into(),
            },
        }];
        assert_eq!(
            extra_limit_chars(&chips),
            "hello there".chars().count() + "why".chars().count()
        );
        let body = expand_prompt("“h…", &chips);
        assert!(body.contains("hello there"));
        assert!(body.contains("Comment: why"));
        assert_eq!(chip_label(&chips[0].kind), "“why”");
    }

    #[test]
    fn trim_and_join_keep_chip_ranges_on_the_label() {
        let (text, chips) = trim_with_chips("  #128  ", &[pr(2, 6)]);
        assert_eq!(text, "#128");
        assert_eq!(&text[chips[0].start..chips[0].end], "#128");
        let (joined, chips) = join_drafts("keep", &[], &text, &chips, true);
        assert_eq!(joined, "keep\n\n#128");
        assert_eq!(&joined[chips[0].start..chips[0].end], "#128");
    }
}
