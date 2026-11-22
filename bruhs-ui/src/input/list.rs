//! Structural edits preserve the source spelling of list markers and the caret's
//! byte offset within the item, including when nearby ordered items change width.
use super::markdown::{block_newline, MarkdownEdit};
use std::ops::Range;

#[derive(Clone, Copy)]
struct Item<'a> {
    indent: &'a str,
    digits: &'a str,
    delimiter: u8,
    content_column: usize,
    task: bool,
}

fn columns(s: &str, mut column: usize) -> usize {
    for byte in s.bytes() {
        column += if byte == b'\t' { 4 - column % 4 } else { 1 };
    }
    column
}

fn whitespace(s: &str) -> &str {
    &s[..s.len() - s.trim_start_matches([' ', '\t']).len()]
}

impl<'a> Item<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let indent = whitespace(line);
        let marker = &line[indent.len()..];
        let count = marker.bytes().take_while(u8::is_ascii_digit).count();
        let digits = &marker[..count];
        let delimiter = *marker.as_bytes().get(count)?;
        match (count, delimiter) {
            (0, b'*' | b'+' | b'-') | (1.., b')' | b'.') => {}
            _ => return None,
        }
        let rest = &marker[count + 1..];
        let gap = whitespace(rest);
        if gap.is_empty() && !rest.is_empty() {
            return None;
        }
        let marker_end = columns(indent, 0) + count + 1;
        let gap_width = columns(gap, marker_end) - marker_end;
        let content_column = marker_end
            + if (1..=4).contains(&gap_width) {
                gap_width
            } else {
                1
            };
        Some(Self {
            indent,
            digits,
            delimiter,
            content_column,
            task: count == 0
                && ["[x]", "[X]", "[ ]"].iter().any(|token| {
                    rest[gap.len()..]
                        .strip_prefix(token)
                        .is_some_and(|body| body.is_empty() || body.starts_with([' ', '\t']))
                }),
        })
    }

    fn next_number(self) -> Option<usize> {
        self.digits.parse::<usize>().ok()?.checked_add(1)
    }

    fn formatted(self, number: usize) -> String {
        if self.digits.starts_with('0') {
            format!("{number:0width$}", width = self.digits.len())
        } else {
            number.to_string()
        }
    }

    fn same_order(self, other: Self) -> bool {
        !self.digits.is_empty() && self.delimiter == other.delimiter
    }
}

fn line_bounds(source: &str, at: usize) -> Range<usize> {
    let start = source[..at].rfind('\n').map_or(0, |i| i + 1);
    let end = source[at..].find('\n').map_or(source.len(), |i| at + i);
    start..end
}

/// Update subsequent siblings in the same edit. Descendants keep their own
/// sequence. A paragraph break or a new list ends the sequence.
fn renumber(
    source: &str,
    line_end: usize,
    depth: usize,
    delimiter: u8,
    first: usize,
    edit: &mut MarkdownEdit,
) {
    let mut number = first;
    let mut offset = line_end;
    while offset < source.len() {
        let start = offset + 1;
        let line = source[start..].split('\n').next().unwrap_or_default();
        offset = start + line.len();
        let indent = columns(whitespace(line), 0);
        if line.trim().is_empty() || indent < depth {
            break;
        }
        if indent > depth {
            continue;
        }
        let Some(item) =
            Item::parse(line).filter(|item| item.delimiter == delimiter && !item.digits.is_empty())
        else {
            break;
        };
        let replacement = item.formatted(number);
        if item.digits != replacement {
            let at = start + item.indent.len();
            edit.text.push_str(&source[edit.range.end..at]);
            edit.text.push_str(&replacement);
            edit.range.end = at + item.digits.len();
        }
        let Some(next) = number.checked_add(1) else {
            break;
        };
        number = next;
    }
}

pub(super) fn indent(source: &str, selection: Range<usize>, outdent: bool) -> Option<MarkdownEdit> {
    let cursor = selection.start;
    let line = line_bounds(source, cursor);
    let item = Item::parse(&source[line.clone()])?;
    if !selection.is_empty() {
        // A selection in a rich list keeps its text and selection on Tab.
        return Some(MarkdownEdit {
            range: cursor..cursor,
            text: String::new(),
            selection,
        });
    }
    let depth = columns(item.indent, 0);
    let previous = source[..line.start]
        .strip_suffix('\n')
        .into_iter()
        .flat_map(|lines| lines.rsplit('\n'))
        .map_while(Item::parse);
    let mut digits = String::new();
    let mut following = None;
    let indent = if outdent {
        if depth == 0 {
            return None;
        }
        match previous
            .into_iter()
            .find(|prev| columns(prev.indent, 0) < depth)
        {
            Some(parent) => {
                if parent.same_order(item) {
                    let next = parent.next_number()?;
                    digits = parent.formatted(next);
                    following = next.checked_add(1).map(|number| (parent, number));
                }
                parent.indent.to_owned()
            }
            None => item
                .indent
                .strip_prefix("  ")
                .unwrap_or(&item.indent[1..])
                .to_owned(),
        }
    } else {
        let mut child: Option<Item<'_>> = None;
        let mut previous = previous.take_while(|prev| columns(prev.indent, 0) >= depth);
        if let Some(parent) = previous.find(|prev| {
            let prev_depth = columns(prev.indent, 0);
            if prev_depth == depth {
                return true;
            }
            if child.is_none_or(|c| prev_depth < columns(c.indent, 0)) {
                child = Some(*prev);
            }
            false
        }) {
            if !item.digits.is_empty() {
                digits = match child.filter(|c| c.same_order(item)) {
                    Some(c) => c.formatted(c.next_number()?),
                    None => "1".into(),
                };
            }
            child.map_or_else(
                || {
                    format!(
                        "{}{}",
                        parent.indent,
                        " ".repeat(parent.content_column - depth)
                    )
                },
                |c| c.indent.to_owned(),
            )
        } else {
            return Some(MarkdownEdit::insert(
                line.start..line.start,
                "  ".into(),
                cursor + 2,
            ));
        }
    };
    let end = line.start
        + item.indent.len()
        + if digits.is_empty() {
            0
        } else {
            item.digits.len()
        };
    let mut prefix = indent;
    let marker_start = line.start + prefix.len();
    prefix.push_str(&digits);
    let caret = if cursor < end {
        marker_start
    } else {
        line.start + prefix.len() + (cursor - end)
    };
    let mut edit = MarkdownEdit::insert(line.start..end, prefix, caret);
    if let Some((level, first)) = following {
        renumber(
            source,
            line.end,
            columns(level.indent, 0),
            level.delimiter,
            first,
            &mut edit,
        );
    }
    Some(edit)
}

pub(super) fn newline(source: &str, selection: Range<usize>, shift: bool) -> Option<MarkdownEdit> {
    let line = line_bounds(source, selection.start);
    let (range, mut text) = block_newline(source, selection)?;
    let item = Item::parse(&source[line.clone()]);
    if range.end > line.end && text.starts_with('\n') {
        if let Some(item) = item {
            // Across paragraphs Shift+Enter joins the surviving text with a
            // hard break. Enter splits the joined item, retaining its marker.
            if shift {
                text.truncate(1);
            } else if !item.digits.is_empty() {
                let prefix = &text[1..];
                let continued = Item::parse(prefix)?;
                let at = 1 + continued.indent.len();
                text.replace_range(at..at + continued.digits.len(), item.digits);
            }
        }
    }
    if range == line {
        if let (Some(item), Some(parent)) = (item, Item::parse(&text)) {
            if item.task != parent.task {
                // A task and an ordinary item use different node types.
                // Shift+Enter inserts a hard break. Enter can leave the
                // nested list once the item is its final child.
                if shift {
                    return Some(MarkdownEdit::insert(
                        line.end..line.end,
                        "\n".into(),
                        line.end + 1,
                    ));
                }
                let next = source[line.end..]
                    .strip_prefix('\n')
                    .and_then(|tail| tail.lines().next());
                if next
                    .and_then(Item::parse)
                    .is_some_and(|next| columns(next.indent, 0) > columns(parent.indent, 0))
                {
                    return Some(MarkdownEdit::insert(
                        line.end..line.end,
                        String::new(),
                        line.end,
                    ));
                }
                text.clear();
            }
        }
    }
    let sequence = if range.end > line.end {
        None
    } else if text.starts_with('\n') {
        item.and_then(|i| {
            Some((
                columns(i.indent, 0),
                i.delimiter,
                i.next_number()?.checked_add(1)?,
            ))
        })
    } else {
        // An empty nested item was promoted. Continue its new sequence.
        Item::parse(&text).and_then(|i| Some((columns(i.indent, 0), i.delimiter, i.next_number()?)))
    };
    let caret = range.start + text.len();
    let mut edit = MarkdownEdit::insert(range, text, caret);
    if let Some((depth, delimiter, first)) = sequence {
        renumber(source, line.end, depth, delimiter, first, &mut edit);
    }
    Some(edit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(before: &str, expected: &str, key: Option<bool>) {
        let at = before.find('|').unwrap();
        let mut source = before.replace('|', "");
        let edit = match key {
            None => newline(&source, at..at, true),
            Some(outdent) => indent(&source, at..at, outdent),
        }
        .expect(before);
        source.replace_range(edit.range, &edit.text);
        assert_eq!(edit.selection.start, edit.selection.end);
        source.insert(edit.selection.start, '|');
        assert_eq!(source, expected);
    }

    #[test]
    fn nest_items_at_the_existing_child_or_parent_content_column() {
        for (before, after) in [
            ("10. Parent\n11. 猫|猫", "10. Parent\n    1. 猫|猫"),
            (
                "1. Parent\n   05) Leaf\n2) |Item",
                "1. Parent\n   05) Leaf\n   06) |Item",
            ),
            (
                "- Parent\n   * Leaf\n- |Item\n  + Child",
                "- Parent\n   * Leaf\n   - |Item\n  + Child",
            ),
            ("1. Parent\n\n9. |Item", "1. Parent\n\n  9. |Item"),
            ("- [x] Parent\n- [ ] |Task", "- [x] Parent\n  - [ ] |Task"),
            ("-\tParent\n- |Item", "-\tParent\n    - |Item"),
        ] {
            check(before, after, Some(false));
        }
    }

    #[test]
    fn outdent_renumbers_only_matching_ordered_siblings() {
        check(
            "09. Root\n    1. A|b\n    2. Leaf\n10. Another\n\n22. Final",
            "09. Root\n10. A|b\n    2. Leaf\n11. Another\n\n22. Final",
            Some(true),
        );
        check(
            "- Root\n  * A|b\n- Last",
            "- Root\n* A|b\n- Last",
            Some(true),
        );
        check("   - A|b", " - A|b", Some(true));
    }

    #[test]
    fn empty_task_nested_under_an_ordinary_item_respects_the_chord() {
        let source = "- Parent\n  - [ ] ";
        let at = source.len();
        for (shift, expected) in [(true, "- Parent\n  - [ ] \n"), (false, "- Parent\n")] {
            let edit = newline(source, at..at, shift).unwrap();
            let mut result = source.to_owned();
            result.replace_range(edit.range, &edit.text);
            assert_eq!(result, expected);
            assert_eq!(edit.selection.start, result.len());
        }
        let siblings = format!("{source}\n  - [x] Done");
        let edit = newline(&siblings, at..at, false).unwrap();
        assert!(edit.range.is_empty() && edit.text.is_empty());
    }

    #[test]
    fn newline_across_two_items_honors_the_chord() {
        let source = "1. Parent\n   1. Zebra\n   2. Last\n2. End";
        let selection = source.find("bra").unwrap()..source.find("st\n").unwrap();
        for (shift, expected) in [
            (true, "1. Parent\n   1. Ze\nst\n2. End"),
            (false, "1. Parent\n   1. Ze\n   1. st\n2. End"),
        ] {
            let edit = newline(source, selection.clone(), shift).unwrap();
            let mut result = source.to_owned();
            result.replace_range(edit.range, &edit.text);
            assert_eq!(result, expected);
            assert_eq!(edit.selection.start, expected.find("st\n").unwrap());
        }
    }

    #[test]
    fn split_and_lift_items_in_place() {
        for (before, after) in [
            (
                "1. Parent\n   1. 猫|猫\n   2. More\n2. Last",
                "1. Parent\n   1. 猫\n   2. |猫\n   3. More\n2. Last",
            ),
            ("01) Z|ebra\n05) Other", "01) Z\n02) |ebra\n03) Other"),
            (
                "- Parent\n  - [x] Ze|bra\n- End",
                "- Parent\n  - [x] Ze\n  - [ ] |bra\n- End",
            ),
            (
                "1. Parent\n   - |\n   - More\n2. Last",
                "1. Parent\n2. |\n   - More\n3. Last",
            ),
            (
                "- Before\n- |\n  - Child\n- Last",
                "- Before\n- \n- |\n  - Child\n- Last",
            ),
            ("1. Parent\n2. |\n3. End", "1. Parent\n|\n3. End"),
        ] {
            check(before, after, None);
        }
    }
}
