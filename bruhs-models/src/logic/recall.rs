//! Sent prompts the composer can bring back, and the text Edit from here
//! returns. History is the user messages loaded in the thread. Nothing here
//! is stored on its own.

/// One prompt the composer can recall. Consecutive identical texts collapse
/// into the later id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentPrompt {
    pub id: String,
    pub text: String,
}

/// Where a recall walk is standing. `placed` is the text last put in the
/// composer. Editing that text leaves the walk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecallCursor {
    pub index: usize,
    pub placed: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecallDirection {
    Older,
    Newer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecallAction {
    /// Replace the composer with this text.
    Place(String),
    /// The walk moved past the newest prompt.
    Clear,
    /// The arrow belongs to the caret.
    Pass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecallOutcome {
    pub action: RecallAction,
    pub cursor: Option<RecallCursor>,
}

/// User messages in order. Blank text is skipped. Consecutive identical
/// texts collapse, keeping the later id.
pub fn sent_prompts<I, S, T>(items: I) -> Vec<SentPrompt>
where
    I: IntoIterator<Item = (S, bool, T)>,
    S: AsRef<str>,
    T: AsRef<str>,
{
    let mut prompts: Vec<SentPrompt> = Vec::new();
    for (id, is_user, text) in items {
        if !is_user {
            continue;
        }
        let text = text.as_ref();
        if text.trim().is_empty() {
            continue;
        }
        if let Some(last) = prompts.last_mut() {
            if last.text == text {
                last.id = id.as_ref().to_string();
                continue;
            }
        }
        prompts.push(SentPrompt {
            id: id.as_ref().to_string(),
            text: text.to_string(),
        });
    }
    prompts
}

/// `(on the first logical line, on the last logical line)` for a collapsed
/// caret. `None` when the selection covers more than a caret. Offsets are
/// bytes and stop on a character boundary.
pub fn logical_line_edges(text: &str, start: usize, end: usize) -> Option<(bool, bool)> {
    if start != end {
        return None;
    }
    let start = floor_char_boundary(text, start);
    let on_first = !text[..start].contains('\n');
    let on_last = !text[start..].contains('\n');
    Some((on_first, on_last))
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// Arrow through sent prompts.
///
/// `holds_extra` means the composer is holding an attachment or a picked
/// element, so it is not empty even when `composer` is. `caret_on_edge` is
/// the first visual line for [`RecallDirection::Older`] and the last visual
/// line for [`RecallDirection::Newer`].
pub fn recall_step(
    prompts: &[SentPrompt],
    cursor: Option<&RecallCursor>,
    composer: &str,
    holds_extra: bool,
    direction: RecallDirection,
    caret_on_edge: bool,
) -> RecallOutcome {
    let active = active_index(prompts, cursor, composer);
    match direction {
        RecallDirection::Older => {
            if let Some(index) = active {
                if !caret_on_edge {
                    return stay(prompts, index);
                }
                let index = index.saturating_sub(1);
                return place(prompts, index);
            }
            if composer.is_empty() && !holds_extra {
                if let Some(index) = prompts.len().checked_sub(1) {
                    return place(prompts, index);
                }
            }
            RecallOutcome {
                action: RecallAction::Pass,
                cursor: None,
            }
        }
        RecallDirection::Newer => {
            let Some(index) = active else {
                return RecallOutcome {
                    action: RecallAction::Pass,
                    cursor: None,
                };
            };
            if !caret_on_edge {
                return stay(prompts, index);
            }
            let next = index + 1;
            if next >= prompts.len() {
                return RecallOutcome {
                    action: RecallAction::Clear,
                    cursor: None,
                };
            }
            place(prompts, next)
        }
    }
}

fn active_index(
    prompts: &[SentPrompt],
    cursor: Option<&RecallCursor>,
    composer: &str,
) -> Option<usize> {
    let cursor = cursor?;
    if composer != cursor.placed {
        return None;
    }
    if prompts
        .get(cursor.index)
        .is_some_and(|prompt| prompt.text == cursor.placed)
    {
        return Some(cursor.index);
    }
    prompts
        .iter()
        .rposition(|prompt| prompt.text == cursor.placed)
}

fn place(prompts: &[SentPrompt], index: usize) -> RecallOutcome {
    let text = prompts[index].text.clone();
    RecallOutcome {
        action: RecallAction::Place(text.clone()),
        cursor: Some(RecallCursor {
            index,
            placed: text,
        }),
    }
}

fn stay(prompts: &[SentPrompt], index: usize) -> RecallOutcome {
    RecallOutcome {
        action: RecallAction::Pass,
        cursor: Some(RecallCursor {
            index,
            placed: prompts[index].text.clone(),
        }),
    }
}

/// The unsent draft stays above the prompt that returns to the composer.
pub fn compose_after_rewind(draft: &str, prompt: &str) -> String {
    if draft.is_empty() {
        return prompt.to_string();
    }
    if prompt.is_empty() {
        return draft.to_string();
    }
    let mut out = String::with_capacity(draft.len() + prompt.len() + 2);
    out.push_str(draft);
    if !draft.ends_with("\n\n") {
        if draft.ends_with('\n') {
            out.push('\n');
        } else {
            out.push_str("\n\n");
        }
    }
    out.push_str(prompt);
    out
}

/// Index of the user message `message_id`. Messages from there on leave the
/// thread. `None` when the id is missing or is not a user message.
pub fn rewind_keep_end(messages: &[(bool, &str)], message_id: &str) -> Option<usize> {
    let index = messages.iter().position(|(_, id)| *id == message_id)?;
    messages[index].0.then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompts() -> Vec<SentPrompt> {
        sent_prompts([
            ("1", true, "older"),
            ("2", false, "reply"),
            ("3", true, "  "),
            ("4", true, "newer"),
            ("5", true, "newer"),
            ("6", true, "older"),
        ])
    }

    #[test]
    fn history_skips_blanks_and_collapses_consecutive_duplicates() {
        let prompts = prompts();
        assert_eq!(
            prompts
                .iter()
                .map(|prompt| (prompt.id.as_str(), prompt.text.as_str()))
                .collect::<Vec<_>>(),
            [("1", "older"), ("5", "newer"), ("6", "older")]
        );
    }

    #[test]
    fn arrow_up_in_an_empty_composer_walks_backward() {
        let prompts = prompts();
        let first = recall_step(&prompts, None, "", false, RecallDirection::Older, true);
        assert_eq!(first.action, RecallAction::Place("older".into()));
        assert_eq!(first.cursor.as_ref().map(|cursor| cursor.index), Some(2));

        let second = recall_step(
            &prompts,
            first.cursor.as_ref(),
            "older",
            false,
            RecallDirection::Older,
            true,
        );
        assert_eq!(second.action, RecallAction::Place("newer".into()));

        let third = recall_step(
            &prompts,
            second.cursor.as_ref(),
            "newer",
            false,
            RecallDirection::Older,
            true,
        );
        assert_eq!(third.action, RecallAction::Place("older".into()));
        let stayed = recall_step(
            &prompts,
            third.cursor.as_ref(),
            "older",
            false,
            RecallDirection::Older,
            true,
        );
        assert_eq!(stayed.action, RecallAction::Place("older".into()));
        assert_eq!(stayed.cursor.as_ref().map(|cursor| cursor.index), Some(0));
    }

    #[test]
    fn arrow_down_walks_forward_and_clears_past_the_newest() {
        let prompts = prompts();
        let oldest = RecallCursor {
            index: 0,
            placed: "older".into(),
        };
        let mid = recall_step(
            &prompts,
            Some(&oldest),
            "older",
            false,
            RecallDirection::Newer,
            true,
        );
        assert_eq!(mid.action, RecallAction::Place("newer".into()));
        let newest = recall_step(
            &prompts,
            mid.cursor.as_ref(),
            "newer",
            false,
            RecallDirection::Newer,
            true,
        );
        assert_eq!(newest.action, RecallAction::Place("older".into()));
        let cleared = recall_step(
            &prompts,
            newest.cursor.as_ref(),
            "older",
            false,
            RecallDirection::Newer,
            true,
        );
        assert_eq!(
            cleared,
            RecallOutcome {
                action: RecallAction::Clear,
                cursor: None,
            }
        );
    }

    #[test]
    fn arrows_move_the_caret_until_the_recalled_line_edge() {
        let prompts = prompts();
        let cursor = RecallCursor {
            index: 1,
            placed: "newer".into(),
        };
        let up = recall_step(
            &prompts,
            Some(&cursor),
            "newer",
            false,
            RecallDirection::Older,
            false,
        );
        assert_eq!(up.action, RecallAction::Pass);
        assert_eq!(up.cursor.as_ref().map(|cursor| cursor.index), Some(1));
        let down = recall_step(
            &prompts,
            Some(&cursor),
            "newer",
            false,
            RecallDirection::Newer,
            false,
        );
        assert_eq!(down.action, RecallAction::Pass);
    }

    #[test]
    fn an_edited_recall_is_a_normal_draft() {
        let prompts = prompts();
        let cursor = RecallCursor {
            index: 2,
            placed: "older".into(),
        };
        let edited = recall_step(
            &prompts,
            Some(&cursor),
            "older!",
            false,
            RecallDirection::Older,
            true,
        );
        assert_eq!(
            edited,
            RecallOutcome {
                action: RecallAction::Pass,
                cursor: None,
            }
        );
    }

    #[test]
    fn an_attachment_blocks_recall_from_an_empty_composer() {
        let prompts = prompts();
        let blocked = recall_step(&prompts, None, "", true, RecallDirection::Older, true);
        assert_eq!(blocked.action, RecallAction::Pass);
    }

    #[test]
    fn arrow_down_in_a_normal_composer_passes() {
        let prompts = prompts();
        let down = recall_step(&prompts, None, "", false, RecallDirection::Newer, true);
        assert_eq!(down.action, RecallAction::Pass);
        let typed = recall_step(&prompts, None, "draft", false, RecallDirection::Older, true);
        assert_eq!(typed.action, RecallAction::Pass);
    }

    #[test]
    fn logical_lines_follow_newlines_and_ignore_a_selection() {
        assert_eq!(logical_line_edges("one", 3, 3), Some((true, true)));
        assert_eq!(logical_line_edges("one\ntwo", 0, 0), Some((true, false)));
        assert_eq!(logical_line_edges("one\ntwo", 4, 4), Some((false, true)));
        assert_eq!(logical_line_edges("one\ntwo", 2, 2), Some((true, false)));
        assert_eq!(logical_line_edges("one", 0, 2), None);
        assert_eq!(logical_line_edges("é", 1, 1), Some((true, true)));
    }

    #[test]
    fn rewind_keeps_the_draft_above_the_prompt() {
        assert_eq!(compose_after_rewind("", "fix the parser"), "fix the parser");
        assert_eq!(compose_after_rewind("draft", ""), "draft");
        assert_eq!(
            compose_after_rewind("draft", "fix the parser"),
            "draft\n\nfix the parser"
        );
        assert_eq!(
            compose_after_rewind("draft\n", "fix the parser"),
            "draft\n\nfix the parser"
        );
        assert_eq!(
            compose_after_rewind("draft\n\n", "fix the parser"),
            "draft\n\nfix the parser"
        );
    }

    #[test]
    fn rewind_drops_the_selected_user_message_and_everything_after_it() {
        let messages = [(true, "a"), (false, "b"), (true, "c"), (false, "d")];
        assert_eq!(rewind_keep_end(&messages, "c"), Some(2));
        assert_eq!(rewind_keep_end(&messages, "b"), None);
        assert_eq!(rewind_keep_end(&messages, "missing"), None);
    }
}
