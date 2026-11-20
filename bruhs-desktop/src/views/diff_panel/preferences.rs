use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DiffPreferences {
    pub blue_orange: bool,
    pub hide_whitespace: bool,
    pub collapsed: bool,
    pub split: bool,
    pub proactive_panels: bool,
    pub open_pull_links_in_app: bool,
}

impl Default for DiffPreferences {
    fn default() -> Self {
        Self {
            blue_orange: false,
            hide_whitespace: false,
            collapsed: false,
            split: false,
            proactive_panels: true,
            open_pull_links_in_app: true,
        }
    }
}

impl DiffPanelView {
    pub(crate) fn set_default_wrap(&mut self, wrap: Option<bool>, cx: &mut Context<'_, Self>) {
        self.wrap = wrap.unwrap_or(true);
        cx.notify();
    }

    pub(crate) fn set_preferences(&mut self, prefs: DiffPreferences, cx: &mut Context<'_, Self>) {
        if prefs.collapsed != self.prefs.collapsed {
            self.fold_exceptions.clear();
        }
        self.prefs = prefs;
        cx.notify();
    }

    pub(super) fn is_collapsed(&self, path: &str) -> bool {
        self.prefs.collapsed ^ self.fold_exceptions.contains(path)
    }

    pub(super) fn fold_file(&mut self, path: &str, collapsed: bool) {
        if collapsed != self.prefs.collapsed {
            self.fold_exceptions.insert(path.to_owned());
        } else {
            self.fold_exceptions.remove(path);
        }
    }

    pub(super) fn added_color(&self) -> Hsla {
        if self.prefs.blue_orange {
            theme::blueberry_400()
        } else {
            theme::pandan_400()
        }
    }
    pub(super) fn removed_color(&self) -> Hsla {
        if self.prefs.blue_orange {
            theme::persimmon_400()
        } else {
            theme::lychee_400()
        }
    }

    pub(super) fn render_split_lines(
        &self,
        lines: &[DiffLine],
        start: usize,
        highlight: Option<Option<usize>>,
    ) -> Vec<AnyElement> {
        split_rows(lines)
            .into_iter()
            .map(|(old, new)| {
                let half = |index: Option<usize>, right| {
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_x_hidden()
                        .when(right, |el| {
                            el.border_l_1().border_color(theme::longan_800())
                        })
                        .when_some(index, |el, i| {
                            el.child(self.render_row(
                                &lines[i],
                                highlight == Some(Some(start + i)),
                                Some(right),
                            ))
                        })
                };
                div()
                    .flex()
                    .w_full()
                    .child(half(old, false))
                    .child(half(new, true))
                    .into_any_element()
            })
            .collect()
    }
}

/// Pair removals with additions within each change block. Both sides share
/// context lines. Indices refer to the unified hunk so search still finds them.
fn split_rows(lines: &[DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut rows = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        if lines[at].kind == LineKind::Context {
            rows.push((Some(at), Some(at)));
            at += 1;
            continue;
        }
        let mut old = Vec::new();
        let mut new = Vec::new();
        while at < lines.len() && lines[at].kind != LineKind::Context {
            if lines[at].kind == LineKind::Remove {
                old.push(at);
            } else {
                new.push(at);
            }
            at += 1;
        }
        rows.extend(
            (0..old.len().max(new.len())).map(|i| (old.get(i).copied(), new.get(i).copied())),
        );
    }
    rows
}

/// Recompare the two sides of an agent edit using whitespace-free keys. Keep
/// the original display text and line numbers of every substantive change.
pub(super) fn ignore_turn_whitespace(turns: &mut [DesktopTurnFiles]) {
    for turn in turns {
        for file in &mut turn.files {
            if file.added || file.deleted || file.is_binary {
                continue;
            }
            file.hunks.retain_mut(|hunk| {
                let old: Vec<_> = hunk.lines.iter().filter(|l| l.old_no.is_some()).collect();
                let new: Vec<_> = hunk.lines.iter().filter(|l| l.new_no.is_some()).collect();
                let keys = |rows: &[&DiffLine]| {
                    // A terminating newline distinguishes an empty line from no lines.
                    rows.iter().fold(String::new(), |mut text, row| {
                        text.extend(row.text.chars().filter(|c| !c.is_whitespace()));
                        text.push('\n');
                        text
                    })
                };
                let mut lines = lcs_diff(&keys(&old), &keys(&new));
                for line in &mut lines {
                    if let Some(original) = line
                        .new_no
                        .map(|n| new[n - 1])
                        .or_else(|| line.old_no.map(|n| old[n - 1]))
                    {
                        line.text.clone_from(&original.text);
                    }
                }
                hunk.lines = lines;
                hunk.lines.iter().any(|l| l.kind != LineKind::Context)
            });
        }
        turn.files
            .retain(|f| f.added || f.deleted || f.is_binary || !f.hunks.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;
    #[test]
    fn hiding_turn_whitespace_keeps_original_spelling_and_hides_empty_edits() {
        let mut turns = vec![DesktopTurnFiles {
            prompt: String::new(),
            label: String::new(),
            files: vec![
                DesktopDiffFile {
                    path: "src/lib.rs".into(),
                    is_binary: false,
                    added: false,
                    deleted: false,
                    hunks: vec![Hunk {
                        header: "@@ edit 1 @@".into(),
                        lines: lcs_diff("  one\nold", "one\n  new"),
                    }],
                },
                DesktopDiffFile {
                    path: "only-spaces".into(),
                    is_binary: false,
                    added: false,
                    deleted: false,
                    hunks: vec![Hunk {
                        header: String::new(),
                        lines: lcs_diff("one", " one "),
                    }],
                },
            ],
        }];
        ignore_turn_whitespace(&mut turns);
        assert_eq!(turns[0].files.len(), 1);
        let lines = &turns[0].files[0].hunks[0].lines;
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.kind == LineKind::Add)
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            ["  new"]
        );
        assert_eq!(
            lines
                .iter()
                .filter(|l| l.kind == LineKind::Remove)
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>(),
            ["old"]
        );
    }

    #[test]
    fn split_diff_pairs_replacements_and_preserves_extra_lines() {
        let lines = lcs_diff("context\nold\nlast", "context\nnew\nextra\nlast");
        let rows = split_rows(&lines);
        assert_eq!(rows.len(), 4);
        let text = |index: Option<usize>| index.map(|i| lines[i].text.as_str());
        assert_eq!(
            rows.iter()
                .map(|(old, new)| (text(*old), text(*new)))
                .collect::<Vec<_>>(),
            [
                (Some("context"), Some("context")),
                (Some("old"), Some("new")),
                (None, Some("extra")),
                (Some("last"), Some("last"))
            ]
        );
    }
}
