//! Select the original request and recent conversation within the title model's input budget.
use bruhs_models::{agent::Role, logic::attachments::UploadedFile};
use std::borrow::Cow;

const OMITTED: &str = "[Earlier content truncated]\n\n";
const TRUNCATED: &str = "\n[Content truncated]\n";
const PER_MESSAGE: usize = 2_000;

pub struct TitleTurn<'a> {
    pub role: Role,
    pub text: &'a str,
    pub files: &'a [UploadedFile],
}

#[derive(Default)]
pub struct Conversation {
    pub text: String,
    pub files: Vec<UploadedFile>,
}

fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

fn prefix(s: &str, budget: usize) -> &str {
    let mut used = 0;
    for (offset, ch) in s.char_indices() {
        used += ch.len_utf16();
        if used > budget {
            return &s[..offset];
        }
    }
    s
}

pub(super) fn clipped(s: &str, budget: usize) -> String {
    if units(s) <= budget {
        return s.to_owned();
    }
    if budget <= TRUNCATED.len() {
        return String::new();
    }
    let available = budget - TRUNCATED.len();
    let head = prefix(s, available.div_ceil(2));
    let mut tail = s.len();
    let mut left = available / 2;
    for (offset, ch) in s.char_indices().rev() {
        if ch.len_utf16() > left {
            break;
        }
        left -= ch.len_utf16();
        tail = offset;
    }
    format!("{head}{TRUNCATED}{}", &s[tail..])
}

impl TitleTurn<'_> {
    fn header(&self) -> &'static str {
        match self.role {
            Role::User => "USER:\n",
            Role::Assistant => "ASSISTANT:\n",
            Role::Tool => "",
        }
    }
    pub fn text_with_attachment_names(&self) -> Cow<'_, str> {
        let text = self.text.trim();
        if self.files.is_empty() {
            return Cow::Borrowed(text);
        }
        let names = self
            .files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let separator = if text.is_empty() { "" } else { "\n" };
        Cow::Owned(format!("{text}{separator}[Attachments: {names}]"))
    }
}

pub fn conversation<'a>(turns: impl IntoIterator<Item = TitleTurn<'a>>) -> Conversation {
    let turns: Vec<_> = turns
        .into_iter()
        .filter(|turn| {
            turn.role != Role::Tool && (!turn.text.trim().is_empty() || !turn.files.is_empty())
        })
        .collect();
    let mut selected = vec![None; turns.len()];
    let mut remaining = 8_000 - OMITTED.len();
    let first = turns.iter().position(|turn| turn.role == Role::User);
    let mut add = |index: usize, reserve: usize| {
        if selected[index].is_some() {
            return;
        }
        let turn = &turns[index];
        let room = remaining
            .saturating_sub(reserve)
            .min(PER_MESSAGE)
            .saturating_sub(turn.header().len() + 2);
        if room <= TRUNCATED.len() {
            return;
        }
        let value = clipped(&turn.text_with_attachment_names(), room);
        remaining -= turn.header().len() + units(&value) + 2;
        selected[index] = Some(value);
    };
    if let Some(index) = first {
        add(index, 0);
    }
    for (role, reserve) in [(Role::User, PER_MESSAGE), (Role::Assistant, 0)] {
        for (index, turn) in turns.iter().enumerate().rev() {
            if turn.role == role {
                add(index, reserve);
            }
        }
    }
    // Give unused space back to the newest selected user requests first.
    for role in [Role::User, Role::Assistant] {
        for (turn, slot) in turns.iter().zip(&mut selected).rev() {
            if turn.role != role {
                continue;
            }
            if let Some(text) = slot {
                let available = units(text) + remaining;
                *text = clipped(&turn.text_with_attachment_names(), available);
                remaining = available - units(text);
            }
        }
    }
    let mut truncated = false;
    let mut sections = Vec::new();
    let mut files = std::collections::VecDeque::new();
    let first_file = first.and_then(|index| turns[index].files.first());
    let recent_limit = if first_file.is_some() { 3 } else { 4 };
    for (turn, slot) in turns.iter().zip(selected) {
        let Some(text) = slot else {
            truncated = true;
            continue;
        };
        truncated |= text != turn.text_with_attachment_names();
        sections.push(format!("{}{text}", turn.header()));
        for file in turn.files {
            if first_file.is_some_and(|first| first.path == file.path) {
                continue;
            }
            if files.len() == recent_limit {
                files.pop_front();
            }
            files.push_back(file);
        }
    }
    Conversation {
        text: format!(
            "{}{}",
            if truncated { OMITTED } else { "" },
            sections.join("\n\n")
        ),
        files: first_file.into_iter().chain(files).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_requests_and_assistant_context_share_space_with_the_first_request() {
        let huge = format!("Initial 🦀 {} original end", "界".repeat(8_000));
        let mut turns = vec![TitleTurn {
            role: Role::User,
            text: &huge,
            files: &[],
        }];
        let history: Vec<_> = (0..30)
            .map(|i| format!("Request {i} {} finish {i}", "内容".repeat(1000)))
            .collect();
        turns.extend(history.iter().map(|text| TitleTurn {
            role: Role::User,
            text,
            files: &[],
        }));
        turns.push(TitleTurn {
            role: Role::Tool,
            text: "SECRET TOOL OUTPUT",
            files: &[],
        });
        turns.push(TitleTurn {
            role: Role::Assistant,
            text: "The affected component is the compositor.",
            files: &[],
        });
        let result = conversation(turns);
        assert!(units(&result.text) <= 8_000);
        for value in [
            OMITTED,
            "Initial 🦀",
            "original end",
            "Request 29",
            "finish 29",
            "ASSISTANT:\nThe affected component",
        ] {
            assert!(result.text.contains(value), "{value}");
        }
        assert!(!result.text.contains("Request 2 ") && !result.text.contains("SECRET TOOL"));
    }
    #[test]
    fn first_image_and_three_recent_attachments_keep_their_original_order() {
        let files: Vec<_> = (0..8)
            .map(|i| UploadedFile {
                name: format!("{i}.png"),
                path: i.to_string(),
            })
            .collect();
        let result = conversation(files.iter().map(|file| TitleTurn {
            role: Role::User,
            text: "",
            files: std::slice::from_ref(file),
        }));
        assert_eq!(
            result
                .files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>(),
            ["0", "5", "6", "7"]
        );
        assert!(result.text.ends_with("USER:\n[Attachments: 7.png]"));
    }
}
