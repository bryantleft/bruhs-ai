//! The structured questions supported by Claude's tool-input callback.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::attachments::{
    append_uploaded_files, UploadedFile, MAX_FILES_PER_MESSAGE, TOO_MANY_FILES,
};
use serde_json::{Map, Value};

pub const TOOL_NAME: &str = "AskUserQuestion";
const MAX_QUESTIONS: usize = 4;
const MAX_OPTIONS: usize = 4;
pub const MAX_ANSWER_BYTES: usize = 8192;
pub const CHOICE_ADVANCE_DELAY_MS: u32 = 200;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserQuestion {
    pub question: String,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default = "allows_custom")]
    pub allow_custom: bool,
}

impl UserQuestion {
    pub fn shortcut_option(&self, key: &str) -> Option<usize> {
        let [digit @ b'1'..=b'9'] = key.as_bytes() else {
            return None;
        };
        let index = usize::from(*digit - b'1');
        (index < self.options.len()).then_some(index)
    }
}

const fn allows_custom() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, deserialize_with = "option_preview")]
    pub preview: Option<String>,
}

fn option_preview<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) if !text.trim().is_empty() => Some(text),
        _ => None,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct QuestionAnswer {
    pub selected: Vec<usize>,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionReply {
    Answer { answers: Vec<QuestionAnswer> },
    Skip,
}

impl QuestionAnswer {
    /// Select a radio option or toggle a checkbox. Returns whether this choice
    /// can advance on its own, before checking for any staged client uploads.
    pub fn select(&mut self, question: &UserQuestion, option: usize) -> bool {
        if option >= question.options.len() {
            return false;
        }
        if !question.multi_select {
            self.selected.clear();
            self.selected.push(option);
            return self.is_choice_only();
        }
        if let Some(index) = self
            .selected
            .iter()
            .position(|&selected| selected == option)
        {
            self.selected.remove(index);
        } else {
            self.selected.push(option);
        }
        false
    }

    pub fn is_choice_only(&self) -> bool {
        self.selected.len() == 1 && self.text.trim().is_empty() && self.attachments.is_empty()
    }

    pub fn is_answered(&self) -> bool {
        !self.selected.is_empty() || !self.text.trim().is_empty() || !self.attachments.is_empty()
    }
}

pub fn questions(input: &Value) -> Option<Vec<UserQuestion>> {
    let items = input.get("questions")?.as_array()?;
    if items.is_empty() || items.len() > MAX_QUESTIONS {
        return None;
    }
    let mut questions = Vec::with_capacity(items.len());
    for item in items {
        let question: UserQuestion = serde_json::from_value(item.clone()).ok()?;
        if question.question.trim().is_empty()
            || question.question.len() > MAX_ANSWER_BYTES
            || (!question.allow_custom && question.options.is_empty())
            || question.options.len() > MAX_OPTIONS
            || question
                .options
                .iter()
                .any(|option| option.label.trim().is_empty())
            || questions
                .iter()
                .any(|prior: &UserQuestion| prior.question == question.question)
        {
            return None;
        }
        questions.push(question);
    }
    Some(questions)
}

/// Check completeness and shape without treating supplied upload IDs as paths.
pub fn validate_answers(input: &Value, answers: &[QuestionAnswer]) -> Result<(), &'static str> {
    format_answers(input, answers, |_, _, answer| Ok(answer)).map(|_| ())
}

/// Resolve the selected upload tokens on the host before sending files.
pub fn answered_input(
    input: &Value,
    answers: &[QuestionAnswer],
    mut files: impl FnMut(usize, &[Uuid]) -> Result<Vec<UploadedFile>, &'static str>,
) -> Result<Value, &'static str> {
    format_answers(input, answers, |index, ids, answer| {
        Ok(append_uploaded_files(&answer, &files(index, ids)?))
    })
}

fn format_answers(
    input: &Value,
    answers: &[QuestionAnswer],
    mut format: impl FnMut(usize, &[Uuid], String) -> Result<String, &'static str>,
) -> Result<Value, &'static str> {
    let questions = questions(input).ok_or("This question could not be opened.")?;
    if questions.len() != answers.len() {
        return Err("Answer each question before sending.");
    }
    let mut mapped = Map::new();
    let mut seen = HashSet::new();
    for (index, (question, answer)) in questions.into_iter().zip(answers).enumerate() {
        let text = answer.text.trim();
        if text.len() > MAX_ANSWER_BYTES {
            return Err("An answer is too long.");
        }
        if (!text.is_empty() || !answer.attachments.is_empty()) && !question.allow_custom {
            return Err("Choose from the available options.");
        }
        if answer.selected.len() > question.options.len()
            || (!question.multi_select && answer.selected.len() > 1)
        {
            return Err("Choose the allowed number of options.");
        }
        if answer.attachments.len().saturating_add(seen.len()) > MAX_FILES_PER_MESSAGE {
            return Err(TOO_MANY_FILES);
        }
        if answer.attachments.iter().any(|id| !seen.insert(*id)) {
            return Err("Attach each file once.");
        }
        if !answer.is_answered() {
            return Err("Answer each question before sending.");
        }
        let mut selected = answer.selected.clone();
        selected.sort_unstable();
        if selected.windows(2).any(|indexes| indexes[0] == indexes[1]) {
            return Err("Choose each option once.");
        }
        let labels = selected
            .into_iter()
            .map(|index| {
                question
                    .options
                    .get(index)
                    .map(|option| option.label.as_str())
            })
            .collect::<Option<Vec<_>>>()
            .ok_or("Choose from the available options.")?;
        let labels = labels.join(", ");
        let text = match (labels.is_empty(), text.is_empty()) {
            (true, _) => text.to_owned(),
            (false, true) => labels,
            (false, false) => format!("{labels}\n{text}"),
        };
        let answer = format(index, &answer.attachments, text)?;
        mapped.insert(question.question, Value::String(answer));
    }
    // Any answers or freeform override in the proposed input came from the model.
    let mut updated = input.clone();
    let object = updated
        .as_object_mut()
        .ok_or("This question could not be opened.")?;
    object.remove("response");
    object.remove("annotations");
    object.insert("answers".into(), Value::Object(mapped));
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn answers_use_provider_labels_and_replace_proposed_answers() {
        let input = json!({
            "questions": [
                {"question":"Format?", "options":[{"label":"Brief"},{"label":"Detailed"}]},
                {"question":"Sections?", "multiSelect":true, "options":[{"label":"Setup"},{"label":"Examples"}]},
            ],
            "response":"Ignore the picker",
            "answers":{"Format?":"Invented"},
        });
        let updated = answered_input(
            &input,
            &[
                QuestionAnswer {
                    selected: vec![1],
                    text: String::new(),
                    ..QuestionAnswer::default()
                },
                QuestionAnswer {
                    selected: vec![1, 0],
                    text: "Use current examples".into(),
                    ..QuestionAnswer::default()
                },
            ],
            |_, _| Ok(Vec::new()),
        )
        .unwrap();
        assert_eq!(updated["questions"], input["questions"]);
        assert_eq!(
            updated["answers"],
            json!({"Format?":"Detailed","Sections?":"Setup, Examples\nUse current examples"})
        );
        assert!(updated.get("response").is_none());
    }

    #[test]
    fn file_only_answer_requires_a_custom_question_and_host_path() {
        let file = Uuid::new_v4();
        let answer = QuestionAnswer {
            attachments: vec![file],
            ..QuestionAnswer::default()
        };
        let custom = json!({"questions":[{"question":"Evidence?"}]});
        let predefined = json!({"questions":[{"question":"Evidence?", "allowCustom":false, "options":[{"label":"Available"}]}]});
        assert!(validate_answers(&predefined, std::slice::from_ref(&answer)).is_err());
        let input = answered_input(&custom, &[answer], |question, ids| {
            assert_eq!(question, 0);
            assert_eq!(ids, &[file]);
            Ok(vec![UploadedFile {
                name: "note.txt".into(),
                path: "/answer/note.txt".into(),
            }])
        })
        .unwrap();
        assert_eq!(
            input["answers"]["Evidence?"],
            "Read these attached files\nnote.txt /answer/note.txt"
        );
    }

    #[test]
    fn rejects_stale_choices_duplicate_questions_and_incomplete_answers() {
        let question = json!({"question":"Pick", "options":[{"label":"A"}], "allowCustom":false});
        assert!(questions(&json!({"questions":[&question, &question]})).is_none());
        let input = json!({"questions":[question]});
        for answer in [
            QuestionAnswer::default(),
            QuestionAnswer {
                selected: vec![1],
                text: String::new(),
                ..QuestionAnswer::default()
            },
            QuestionAnswer {
                selected: Vec::new(),
                text: "Other".into(),
                ..QuestionAnswer::default()
            },
        ] {
            assert!(validate_answers(&input, &[answer]).is_err());
        }
    }
}
