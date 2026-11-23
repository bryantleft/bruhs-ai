//! Parsed list gutters for the rich composer. A list shares the width of its
//! longest ordered marker. Nested lists start at the parent item's body.
use pulldown_cmark::{Event, Tag, TagEnd};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gutter {
    Bullet,
    Task,
    Ordered(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ListLine {
    pub start: usize,
    pub body: usize,
    pub gutters: Vec<Gutter>,
}

#[derive(Default)]
pub(super) struct ListLayout {
    groups: Vec<Gutter>,
    lists: Vec<usize>,
    items: Vec<usize>,
    lines: BTreeMap<usize, (usize, Vec<usize>)>,
}

impl ListLayout {
    pub(super) fn depth(&self) -> usize {
        self.lists.len()
    }

    pub(super) fn observe(&mut self, source: &str, event: &Event<'_>, range: &Range<usize>) {
        match event {
            Event::Start(Tag::List(number)) => {
                self.lists.push(self.groups.len());
                self.groups.push(if number.is_some() {
                    Gutter::Ordered(2)
                } else {
                    Gutter::Bullet
                });
            }
            Event::End(TagEnd::List(_)) => {
                self.lists.pop();
            }
            Event::Start(Tag::Item) => {
                let Some(&group) = self.lists.last() else {
                    return;
                };
                let marker = source[range.start..].split('\n').next().unwrap_or_default();
                let width = marker.bytes().take_while(u8::is_ascii_digit).count() + 1;
                let after = &marker[width..];
                let body = after.trim_start_matches([' ', '\t']);
                let at = range.start + marker.len() - body.len();
                if let Gutter::Ordered(longest) = &mut self.groups[group] {
                    *longest = (*longest).max(width);
                }
                // The parser emits the task marker next. Use its range there so
                // ordinary bracket text keeps the bullet and its source mapping.
                self.items.push(group);
                self.record(source, range.start, at);
            }
            Event::TaskListMarker(_) if !self.items.is_empty() => {
                let group = self.groups.len();
                self.groups.push(Gutter::Task);
                if let Some(last) = self.items.last_mut() {
                    *last = group;
                }
                self.record(source, range.start, skip_space(source, range.end));
            }
            Event::End(TagEnd::Item) => {
                self.items.pop();
            }
            Event::SoftBreak | Event::HardBreak if !self.items.is_empty() => {
                if let Some(newline) = source[range.clone()].find('\n') {
                    let start = range.start + newline + 1;
                    self.record(source, start, skip_space(source, start));
                }
            }
            Event::Start(Tag::Paragraph) if !self.items.is_empty() => {
                let start = line_start(source, range.start);
                if !self.lines.contains_key(&start) {
                    self.record(source, range.start, skip_space(source, start));
                }
            }
            _ => {}
        }
    }

    fn record(&mut self, source: &str, at: usize, body: usize) {
        self.lines
            .insert(line_start(source, at), (body, self.items.clone()));
    }

    pub(super) fn finish(self) -> Vec<ListLine> {
        self.lines
            .into_iter()
            .map(|(start, (body, groups))| ListLine {
                start,
                body,
                gutters: groups.into_iter().map(|i| self.groups[i]).collect(),
            })
            .collect()
    }
}
fn line_start(source: &str, at: usize) -> usize {
    source[..at].rfind('\n').map_or(0, |i| i + 1)
}
fn skip_space(source: &str, at: usize) -> usize {
    at + source[at..]
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::{Options, Parser};

    #[test]
    fn continuations_inherit_the_correct_item_and_fences_stay_literal() {
        let s="9. Parent\n   + child\n     continued\n10. Sibling\n\n    another paragraph\n\n- [x] café\n  remains done\n\n```md\n- untouched\n```\n- next";
        let mut lists = ListLayout::default();
        for (event, range) in Parser::new_ext(s, Options::ENABLE_TASKLISTS).into_offset_iter() {
            lists.observe(s, &event, &range);
        }
        let lines = lists.finish();
        let line = |word: &str| {
            lines
                .iter()
                .find(|line| s[line.body..].starts_with(word))
                .unwrap()
        };
        assert_eq!(line("Parent").gutters, [Gutter::Ordered(3)]);
        assert_eq!(line("child").gutters, [Gutter::Ordered(3), Gutter::Bullet]);
        assert_eq!(line("continued").gutters, line("child").gutters);
        assert_eq!(line("another paragraph").gutters, line("Sibling").gutters);
        assert_eq!(line("remains done").gutters, [Gutter::Task]);
        assert_eq!(line("next").gutters, [Gutter::Bullet]);
        assert!(!lines
            .iter()
            .any(|line| s[line.body..].starts_with("untouched")));
    }
}
