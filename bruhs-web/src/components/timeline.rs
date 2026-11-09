//! Pure timeline model: turns a thread's ordered messages into the flat list
//! of rows the chat renders (user bubbles, assistant prose, grouped tool
//! activity, working/thinking indicators, changed-files cards, errors and
//! per-turn metadata). No DOM here so it is unit-testable on the host.

use crate::format::format_duration_ms;
use bruhs_models::agent::Role;
use bruhs_models::logic::diff::{lcs_diff, stats};
use bruhs_models::logic::tools::{
    self, relative_path, tool_kind, tool_label, tool_state, ToolCallView,
};
pub use bruhs_models::logic::tools::{ToolKind, ToolState};

use crate::state::{AgentMessage, MessagePart, ToolCallInfo, UsageInfo};

#[derive(Clone, PartialEq, Debug)]
pub struct ToolView {
    pub call: ToolCallInfo,
    pub kind: ToolKind,
    pub state: ToolState,
    pub label: String,
}

/// One file touched by Edit/Write calls in a turn, with aggregate stats.
#[derive(Clone, PartialEq, Debug)]
pub struct ChangedFile {
    pub path: String,
    pub added: bool,
    pub additions: usize,
    pub deletions: usize,
    /// `(old, new)` snippets in call order; Write contributes `("", content)`.
    pub edits: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Debug)]
pub enum TimelineRow {
    User {
        key: String,
        message_id: String,
        text: String,
        at_ms: Option<f64>,
        files: Vec<bruhs_models::logic::attachments::UploadedFile>,
        /// Worktree tree at send time, when this thread has one.
        checkpoint: Option<String>,
    },
    AssistantText {
        key: String,
        message_id: String,
        markdown: String,
        streaming: bool,
    },
    Thinking {
        key: String,
        message_id: String,
        text: String,
        live: bool,
    },
    Work {
        key: String,
        message_id: String,
        tools: Vec<ToolView>,
        live: bool,
        collapsed_by_default: bool,
    },
    Working {
        key: String,
        message_id: String,
        started_at_ms: Option<f64>,
        thinking_only: bool,
    },
    /// A settled turn's activity folded behind one "Worked for …" row.
    /// `rows` are the hidden thinking / tool / narration rows.
    Fold {
        key: String,
        message_id: String,
        label: String,
        rows: Vec<TimelineRow>,
    },
    ChangedFiles {
        key: String,
        message_id: String,
        user_message_id: String,
        files: Vec<ChangedFile>,
        /// True for the most recent assistant message — rendered expanded.
        latest: bool,
    },
    Error {
        key: String,
        message_id: String,
        message: String,
        retry_prompt: Option<String>,
    },
    AssistantMeta {
        key: String,
        message_id: String,
        model: Option<String>,
        duration_ms: Option<f64>,
        usage: Option<UsageInfo>,
        at_ms: Option<f64>,
        text: String,
    },
}

impl TimelineRow {
    pub fn key(&self) -> &str {
        match self {
            TimelineRow::User { key, .. }
            | TimelineRow::AssistantText { key, .. }
            | TimelineRow::Thinking { key, .. }
            | TimelineRow::Work { key, .. }
            | TimelineRow::Working { key, .. }
            | TimelineRow::Fold { key, .. }
            | TimelineRow::ChangedFiles { key, .. }
            | TimelineRow::Error { key, .. }
            | TimelineRow::AssistantMeta { key, .. } => key,
        }
    }

    /// Id of the message this row belongs to.
    #[cfg(test)]
    pub fn message_id(&self) -> &str {
        match self {
            TimelineRow::User { message_id, .. }
            | TimelineRow::AssistantText { message_id, .. }
            | TimelineRow::Thinking { message_id, .. }
            | TimelineRow::Work { message_id, .. }
            | TimelineRow::Working { message_id, .. }
            | TimelineRow::Fold { message_id, .. }
            | TimelineRow::ChangedFiles { message_id, .. }
            | TimelineRow::Error { message_id, .. }
            | TimelineRow::AssistantMeta { message_id, .. } => message_id,
        }
    }
}

/// One rendered turn: a message id plus its rows, in order.
#[derive(Clone, PartialEq, Debug)]
pub struct Turn {
    pub message_id: String,
    pub is_user: bool,
    pub rows: Vec<TimelineRow>,
}

impl ToolCallView for ToolCallInfo {
    fn tool_name(&self) -> &str {
        &self.tool
    }

    fn input(&self) -> &serde_json::Value {
        &self.input
    }

    fn has_output(&self) -> bool {
        self.output.is_some()
    }

    fn is_error(&self) -> bool {
        self.is_error
    }
}

/// One-line summary of a collapsed tool group, e.g.
/// `Read 2 files, edited 1 file and ran 1 command`.
pub fn work_summary(tools: &[ToolView]) -> String {
    let kinds: Vec<ToolKind> = tools.iter().map(|t| t.kind).collect();
    tools::work_summary(&kinds)
}

fn count_lines(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}

/// Files touched by successful Edit/Write calls in `message`, first-seen order.
pub fn changed_files(message: &AgentMessage, root: Option<&str>) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut push = |path: String, added: bool, old: String, new: String| {
        let (adds, dels) = if old.is_empty() {
            (count_lines(&new), 0)
        } else if new.is_empty() {
            (0, count_lines(&old))
        } else {
            let st = stats(&lcs_diff(&old, &new));
            (st.additions, st.deletions)
        };
        if let Some(f) = files.iter_mut().find(|f| f.path == path) {
            f.additions += adds;
            f.deletions += dels;
            f.added |= added;
            f.edits.push((old, new));
        } else {
            files.push(ChangedFile {
                path,
                added,
                additions: adds,
                deletions: dels,
                edits: vec![(old, new)],
            });
        }
    };
    for call in message.tool_calls_iter() {
        if call.output.is_none() || call.is_error {
            continue;
        }
        let field = |k: &str| {
            call.input
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        let raw_path = if field("file_path").is_empty() {
            field("notebook_path")
        } else {
            field("file_path")
        };
        if raw_path.is_empty() {
            continue;
        }
        let path = relative_path(&raw_path, root);
        match tool_kind(&call.tool) {
            ToolKind::Write => {
                let content = field("content");
                push(path, true, String::new(), content);
            }
            ToolKind::Edit => {
                if let Some(edits) = call.input.get("edits").and_then(|v| v.as_array()) {
                    for e in edits {
                        let old = e.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                        let new = e.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                        push(path.clone(), false, old.to_string(), new.to_string());
                    }
                } else if !field("old_string").is_empty() || !field("new_string").is_empty() {
                    push(path, false, field("old_string"), field("new_string"));
                } else if !field("new_source").is_empty() {
                    push(path, false, String::new(), field("new_source"));
                }
            }
            _ => {}
        }
    }
    files
}

/// Fold a settled turn: everything before the terminal
/// assistant message hides behind one "Worked for …" row, a single ordinary
/// activity after it joins the fold, and failed turns stay fully visible.
pub fn fold_settled_turn(
    part_rows: Vec<TimelineRow>,
    msg: &AgentMessage,
    is_latest: bool,
) -> Vec<TimelineRow> {
    if msg.error.is_some() {
        return part_rows;
    }
    let n = part_rows.len();
    let terminal = part_rows
        .iter()
        .rposition(|r| matches!(r, TimelineRow::AssistantText { .. }));
    let terminal_idx = terminal.unwrap_or(n);
    let mut hidden = vec![false; n];
    for (i, r) in part_rows.iter().enumerate() {
        if Some(i) == terminal {
            continue;
        }
        let single_trailing = n == terminal_idx + 2
            && matches!(r, TimelineRow::Work { tools, .. }
                if tools.len() == 1 && !tools.iter().any(|t| t.state == ToolState::Error));
        if i > terminal_idx && !single_trailing {
            continue;
        }
        hidden[i] = true;
    }
    if !hidden.iter().any(|h| *h) {
        return part_rows;
    }

    let interrupted = is_latest && msg.tool_calls_iter().any(|c| c.output.is_none());
    let duration = match (msg.started_at_ms, msg.status.finished_at_ms()) {
        (Some(s), Some(e)) if e >= s => Some(e - s),
        _ => None,
    };
    let label = match (interrupted, duration) {
        (true, Some(d)) => format!("You stopped after {}", format_duration_ms(d)),
        (true, None) => "You stopped this response".to_string(),
        (false, Some(d)) => format!("Worked for {}", format_duration_ms(d)),
        (false, None) => "Worked".to_string(),
    };

    let mut out: Vec<TimelineRow> = Vec::with_capacity(n);
    let mut folded: Vec<TimelineRow> = Vec::new();
    let mut insert_at: Option<usize> = None;
    for (i, r) in part_rows.into_iter().enumerate() {
        if hidden[i] {
            if insert_at.is_none() {
                insert_at = Some(out.len());
            }
            folded.push(r);
        } else {
            out.push(r);
        }
    }
    out.insert(
        insert_at.unwrap_or(0),
        TimelineRow::Fold {
            key: format!("{}:fold", msg.id),
            message_id: msg.id.clone(),
            label,
            rows: folded,
        },
    );
    out
}

/// Builds the rows of one message at a time. A message's rows depend on the
/// messages before it only through the latest prompt (for changed-files
/// cards and retries), and on which assistant message is the latest, so a
/// settled transcript can be built once and the running turn's message on
/// its own, per token.
pub struct RowBuilder<'a> {
    root: Option<&'a str>,
    /// The thread's latest assistant message (it may be the running one).
    latest_assistant: Option<&'a str>,
    /// `(id, text)` of the latest user message seen so far.
    last_user: Option<(String, String)>,
}

impl<'a> RowBuilder<'a> {
    pub fn new(root: Option<&'a str>, latest_assistant: Option<&'a str>) -> Self {
        Self {
            root,
            latest_assistant,
            last_user: None,
        }
    }

    /// A builder that continues after `earlier` (only its latest prompt
    /// matters), for the running turn's message.
    pub fn after(
        earlier: &[AgentMessage],
        root: Option<&'a str>,
        latest_assistant: Option<&'a str>,
    ) -> Self {
        Self {
            last_user: earlier
                .iter()
                .rev()
                .find(|m| m.role == Role::User)
                .map(|m| (m.id.clone(), m.text())),
            ..Self::new(root, latest_assistant)
        }
    }

    /// The rows of `msg`, in order (always at least one).
    pub fn rows(&mut self, msg: &AgentMessage) -> Vec<TimelineRow> {
        let mut rows: Vec<TimelineRow> = Vec::new();
        let id = &msg.id;
        if msg.role == Role::User {
            let text = msg.text();
            self.last_user = Some((id.clone(), text.clone()));
            rows.push(TimelineRow::User {
                key: format!("{id}:user"),
                message_id: id.clone(),
                text,
                at_ms: msg.started_at_ms,
                files: msg.files.clone(),
                checkpoint: msg.checkpoint.clone().filter(|tree| !tree.is_empty()),
            });
            return rows;
        }

        let streaming = msg.is_streaming();
        let parts = &msg.parts;
        let last_idx = parts.len().saturating_sub(1);
        let mut part_rows: Vec<TimelineRow> = Vec::new();
        let mut i = 0;
        while i < parts.len() {
            match &parts[i] {
                MessagePart::Text(t) => {
                    part_rows.push(TimelineRow::AssistantText {
                        key: format!("{id}:text:{i}"),
                        message_id: id.clone(),
                        markdown: t.clone(),
                        streaming: streaming && i == last_idx,
                    });
                    i += 1;
                }
                MessagePart::Thinking(t) => {
                    part_rows.push(TimelineRow::Thinking {
                        key: format!("{id}:think:{i}"),
                        message_id: id.clone(),
                        text: t.clone(),
                        live: streaming && i == last_idx,
                    });
                    i += 1;
                }
                MessagePart::ToolCall(_) => {
                    let start = i;
                    let mut tools: Vec<ToolView> = Vec::new();
                    while let Some(MessagePart::ToolCall(call)) = parts.get(i) {
                        let state = tool_state(call, streaming);
                        tools.push(ToolView {
                            kind: tool_kind(&call.tool),
                            state,
                            label: tool_label(call, state, self.root),
                            call: call.clone(),
                        });
                        i += 1;
                    }
                    let live = streaming && i == parts.len();
                    part_rows.push(TimelineRow::Work {
                        key: format!("{id}:work:{start}"),
                        message_id: id.clone(),
                        live,
                        collapsed_by_default: !live && tools.len() >= 2,
                        tools,
                    });
                }
            }
        }

        if streaming {
            rows.extend(part_rows);
            rows.push(TimelineRow::Working {
                key: format!("{id}:working"),
                message_id: id.clone(),
                started_at_ms: msg.started_at_ms,
                thinking_only: parts.is_empty(),
            });
            return rows;
        }

        let is_latest = self.latest_assistant == Some(id.as_str());
        rows.extend(fold_settled_turn(part_rows, msg, is_latest));

        if parts.is_empty() && msg.error.is_none() {
            rows.push(TimelineRow::AssistantText {
                key: format!("{id}:text:empty"),
                message_id: id.clone(),
                markdown: String::new(),
                streaming: false,
            });
        }

        let files = changed_files(msg, self.root);
        if !files.is_empty() {
            rows.push(TimelineRow::ChangedFiles {
                key: format!("{id}:files"),
                message_id: id.clone(),
                user_message_id: self
                    .last_user
                    .as_ref()
                    .map(|(uid, _)| uid.clone())
                    .unwrap_or_default(),
                files,
                latest: is_latest,
            });
        }

        if let Some(err) = &msg.error {
            rows.push(TimelineRow::Error {
                key: format!("{id}:error"),
                message_id: id.clone(),
                message: err.clone(),
                retry_prompt: self
                    .last_user
                    .as_ref()
                    .map(|(_, text)| text.clone())
                    .filter(|t| !t.trim().is_empty()),
            });
        } else {
            let finished_at = msg.status.finished_at_ms();
            let duration_ms = match (msg.started_at_ms, finished_at) {
                (Some(s), Some(e)) if e >= s => Some(e - s),
                _ => None,
            };
            rows.push(TimelineRow::AssistantMeta {
                key: format!("{id}:meta"),
                message_id: id.clone(),
                model: msg.model.clone(),
                duration_ms,
                usage: msg.usage.clone(),
                at_ms: finished_at.or(msg.started_at_ms),
                text: msg.text(),
            });
        }
        rows
    }

    /// `msg` as one turn.
    pub fn turn(&mut self, msg: &AgentMessage) -> Turn {
        Turn {
            message_id: msg.id.clone(),
            is_user: msg.role == Role::User,
            rows: self.rows(msg),
        }
    }
}

/// Id of the latest assistant message in `messages`.
pub fn latest_assistant_id(messages: &[AgentMessage]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.id.as_str())
}

/// The turns of a settled transcript. `latest_assistant` overrides which
/// assistant message counts as the latest: while a turn runs, its message
/// is, even though it is not in `messages`.
pub fn build_turns(
    messages: &[AgentMessage],
    project_root: Option<&str>,
    latest_assistant: Option<&str>,
) -> Vec<Turn> {
    let mut builder = RowBuilder::new(project_root, latest_assistant);
    messages.iter().map(|m| builder.turn(m)).collect()
}

/// Flatten `messages` into timeline rows.
#[cfg(test)]
pub fn build_rows(messages: &[AgentMessage], project_root: Option<&str>) -> Vec<TimelineRow> {
    build_turns(messages, project_root, latest_assistant_id(messages))
        .into_iter()
        .flat_map(|t| t.rows)
        .collect()
}

#[cfg(test)]
mod bench;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{AgentMessage, MessagePart, MessageStatus, ToolCallInfo};
    use serde_json::json;

    fn kind_of(r: &TimelineRow) -> &'static str {
        match r {
            TimelineRow::User { .. } => "user",
            TimelineRow::Thinking { .. } => "think",
            TimelineRow::Work { .. } => "work",
            TimelineRow::AssistantText { .. } => "text",
            TimelineRow::ChangedFiles { .. } => "files",
            TimelineRow::AssistantMeta { .. } => "meta",
            TimelineRow::Working { .. } => "working",
            TimelineRow::Fold { .. } => "fold",
            TimelineRow::Error { .. } => "error",
        }
    }

    fn call(
        id: &str,
        tool: &str,
        input: serde_json::Value,
        output: Option<&str>,
        err: bool,
    ) -> ToolCallInfo {
        ToolCallInfo {
            id: id.into(),
            tool: tool.into(),
            input,
            output: output.map(Into::into),
            is_error: err,
            started_at_ms: Some(0.0),
            ended_at_ms: output.map(|_| 1000.0),
        }
    }

    fn view(c: ToolCallInfo) -> ToolView {
        ToolView {
            kind: tool_kind(&c.tool),
            state: tool_state(&c, false),
            label: String::new(),
            call: c,
        }
    }

    #[test]
    fn work_summary_counts_by_verb() {
        let tools = vec![
            view(call("1", "Read", json!({}), Some(""), false)),
            view(call("2", "Read", json!({}), Some(""), false)),
            view(call("3", "Edit", json!({}), Some(""), false)),
            view(call("4", "Bash", json!({}), Some(""), false)),
        ];
        assert_eq!(
            work_summary(&tools),
            "Read 2 files, edited 1 file and ran 1 command"
        );
        assert_eq!(work_summary(&tools[..1]), "Read 1 file");
        let mixed = vec![
            view(call("1", "Write", json!({}), Some(""), false)),
            view(call("2", "Edit", json!({}), Some(""), false)),
            view(call("3", "WebFetch", json!({}), Some(""), false)),
        ];
        assert_eq!(work_summary(&mixed), "Edited 2 files and searched 1 time");
        assert_eq!(work_summary(&[]), "");
    }

    #[test]
    fn rows_group_consecutive_tools_and_add_meta() {
        let user = AgentMessage::user("do it".into(), 1.0);
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.status = MessageStatus::Finished {
            at_ms: Some(5000.0),
        };
        a.parts = vec![
            MessagePart::Thinking("hm".into()),
            MessagePart::ToolCall(call(
                "1",
                "Read",
                json!({"file_path":"a"}),
                Some("x"),
                false,
            )),
            MessagePart::ToolCall(call(
                "2",
                "Edit",
                json!({"file_path":"a","old_string":"1","new_string":"2"}),
                Some("ok"),
                false,
            )),
            MessagePart::Text("done".into()),
        ];
        let rows = build_rows(&[user.clone(), a], None);
        let kinds: Vec<&str> = rows.iter().map(kind_of).collect();
        // Settled turn: thinking + tools fold behind "Worked for …", the
        // terminal text, changed files and meta stay visible.
        assert_eq!(kinds, ["user", "fold", "text", "files", "meta"]);
        if let TimelineRow::Fold {
            label, rows: inner, ..
        } = &rows[1]
        {
            assert_eq!(label, "Worked for 5.0s");
            assert_eq!(
                inner.iter().map(kind_of).collect::<Vec<_>>(),
                ["think", "work"]
            );
            if let TimelineRow::Work {
                tools,
                collapsed_by_default,
                live,
                ..
            } = &inner[1]
            {
                assert_eq!(tools.len(), 2);
                assert!(*collapsed_by_default);
                assert!(!live);
                assert_eq!(tools[1].label, "Edited a");
            } else {
                panic!("expected work row inside the fold");
            }
        } else {
            panic!("expected fold row");
        }
        if let TimelineRow::ChangedFiles {
            files,
            latest,
            user_message_id,
            ..
        } = &rows[3]
        {
            assert_eq!(files[0].path, "a");
            assert_eq!((files[0].additions, files[0].deletions), (1, 1));
            assert!(*latest);
            assert_eq!(user_message_id, &user.id);
        } else {
            panic!("expected changed-files row");
        }
        if let TimelineRow::AssistantMeta { duration_ms, .. } = &rows[4] {
            assert_eq!(*duration_ms, Some(4998.0));
        } else {
            panic!("expected meta row");
        }
        let keys: std::collections::HashSet<&str> = rows.iter().map(|r| r.key()).collect();
        assert_eq!(keys.len(), rows.len(), "row keys must be unique");
    }

    #[test]
    fn streaming_message_gets_live_group_and_working_row() {
        let user = AgentMessage::user("go".into(), 1.0);
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.parts = vec![MessagePart::ToolCall(call(
            "1",
            "Bash",
            json!({"command":"ls"}),
            None,
            false,
        ))];
        let rows = build_rows(&[user, a], None);
        assert!(matches!(
            &rows[1],
            TimelineRow::Work { live: true, collapsed_by_default: false, tools, .. }
                if tools[0].state == ToolState::Running && tools[0].label == "Running ls"
        ));
        assert!(matches!(
            &rows[2],
            TimelineRow::Working {
                thinking_only: false,
                ..
            }
        ));
        let empty = build_rows(
            &[
                AgentMessage::user("go".into(), 1.0),
                AgentMessage::assistant_placeholder("m".into(), 2.0),
            ],
            None,
        );
        assert!(matches!(
            &empty[1],
            TimelineRow::Working {
                thinking_only: true,
                ..
            }
        ));
        assert_eq!(empty.len(), 2);
    }

    #[test]
    fn interrupted_tools_are_not_live_after_stop() {
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.status = MessageStatus::Finished { at_ms: None };
        a.parts = vec![MessagePart::ToolCall(call(
            "1",
            "Bash",
            json!({"command":"sleep 10"}),
            None,
            false,
        ))];
        let rows = build_rows(&[a], None);
        // Stopped latest turn: folded with a "You stopped" label, the
        // interrupted tool inside is not live.
        match &rows[0] {
            TimelineRow::Fold {
                label, rows: inner, ..
            } => {
                assert!(label.starts_with("You stopped"), "{label}");
                assert!(matches!(
                    &inner[0],
                    TimelineRow::Work { live: false, tools, .. }
                        if tools[0].state == ToolState::Interrupted
                ));
            }
            other => panic!("expected fold, got {other:?}"),
        }
    }

    #[test]
    fn fold_rules_follow_t3() {
        // Intro text + tools + terminal text: intro folds too, terminal stays.
        let mut a = AgentMessage::assistant_placeholder("m".into(), 0.0);
        a.status = MessageStatus::Finished {
            at_ms: Some(90_000.0),
        };
        a.parts = vec![
            MessagePart::Text("Let me look.".into()),
            MessagePart::ToolCall(call(
                "1",
                "Read",
                json!({"file_path":"a"}),
                Some("x"),
                false,
            )),
            MessagePart::Text("Done.".into()),
        ];
        let rows = build_rows(&[a.clone()], None);
        assert_eq!(
            rows.iter().map(kind_of).collect::<Vec<_>>(),
            ["fold", "text", "meta"]
        );
        assert!(
            matches!(&rows[0], TimelineRow::Fold { label, rows: inner, .. }
            if label == "Worked for 1m 30s" && inner.len() == 2)
        );

        // One ordinary trailing activity after the terminal text joins the fold.
        let mut b = a.clone();
        b.parts.push(MessagePart::ToolCall(call(
            "2",
            "Bash",
            json!({"command":"ls"}),
            Some(""),
            false,
        )));
        let rows = build_rows(&[b], None);
        assert_eq!(
            rows.iter().map(kind_of).collect::<Vec<_>>(),
            ["fold", "text", "meta"]
        );
        assert!(matches!(&rows[0], TimelineRow::Fold { rows: inner, .. } if inner.len() == 3));

        // Two trailing activities stay visible after the terminal text.
        let mut c = a.clone();
        c.parts.push(MessagePart::ToolCall(call(
            "2",
            "Bash",
            json!({"command":"ls"}),
            Some(""),
            false,
        )));
        c.parts.push(MessagePart::ToolCall(call(
            "3",
            "Bash",
            json!({"command":"pwd"}),
            Some(""),
            false,
        )));
        let rows = build_rows(&[c], None);
        assert_eq!(
            rows.iter().map(kind_of).collect::<Vec<_>>(),
            ["fold", "text", "work", "meta"]
        );

        // Failed turns never fold.
        let mut d = a;
        d.error = Some("boom".into());
        let rows = build_rows(&[d], None);
        assert_eq!(
            rows.iter().map(kind_of).collect::<Vec<_>>(),
            ["text", "work", "text", "error"]
        );

        // Text-only turns have nothing to fold.
        let mut e = AgentMessage::assistant_placeholder("m".into(), 0.0);
        e.status = MessageStatus::Finished { at_ms: None };
        e.parts = vec![MessagePart::Text("hi".into())];
        let rows = build_rows(&[e], None);
        assert_eq!(
            rows.iter().map(kind_of).collect::<Vec<_>>(),
            ["text", "meta"]
        );
    }

    #[test]
    fn error_row_carries_retry_prompt() {
        let user = AgentMessage::user("retry me".into(), 1.0);
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.status = MessageStatus::Finished { at_ms: None };
        a.error = Some("boom".into());
        let rows = build_rows(&[user, a], None);
        assert!(rows.iter().any(|r| matches!(
            r,
            TimelineRow::Error { message, retry_prompt: Some(p), .. }
                if message == "boom" && p == "retry me"
        )));
        assert!(!rows
            .iter()
            .any(|r| matches!(r, TimelineRow::AssistantMeta { .. })));
    }

    #[test]
    fn write_counts_as_added_file() {
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.parts = vec![MessagePart::ToolCall(call(
            "1",
            "Write",
            json!({"file_path":"/r/new.rs","content":"a\nb\nc"}),
            Some("ok"),
            false,
        ))];
        let files = changed_files(&a, Some("/r"));
        assert!(files[0].added);
        assert_eq!(files[0].path, "new.rs");
        assert_eq!(files[0].additions, 3);
        assert_eq!(files[0].deletions, 0);
    }

    #[test]
    fn changed_files_merge_and_skip_failures() {
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.parts = vec![
            MessagePart::ToolCall(call(
                "1",
                "Edit",
                json!({"file_path":"/r/x.rs","old_string":"a","new_string":"b"}),
                Some("ok"),
                false,
            )),
            MessagePart::ToolCall(call(
                "2",
                "MultiEdit",
                json!({"file_path":"/r/x.rs","edits":[{"old_string":"c","new_string":"d\ne"}]}),
                Some("ok"),
                false,
            )),
            MessagePart::ToolCall(call(
                "3",
                "Edit",
                json!({"file_path":"/r/y.rs","old_string":"1","new_string":"2"}),
                Some("nope"),
                true,
            )),
            MessagePart::ToolCall(call(
                "4",
                "Edit",
                json!({"file_path":"/r/z.rs","old_string":"1","new_string":"2"}),
                None,
                false,
            )),
        ];
        let files = changed_files(&a, Some("/r"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "x.rs");
        assert_eq!(files[0].edits.len(), 2);
        assert_eq!((files[0].additions, files[0].deletions), (3, 2));
        assert!(!files[0].added);
    }

    #[test]
    fn legacy_user_message_without_parts_still_renders() {
        let user: AgentMessage = serde_json::from_value(json!({
            "id": "u", "role": "user", "content": "hello", "tool_calls": [], "is_streaming": false
        }))
        .unwrap();
        let rows = build_rows(&[user], None);
        assert!(matches!(&rows[0], TimelineRow::User { text, .. } if text == "hello"));
    }

    #[test]
    fn turns_hold_each_message_rows_in_order() {
        let user = AgentMessage::user("go".into(), 1.0);
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.status = MessageStatus::Finished { at_ms: None };
        a.parts = vec![
            MessagePart::ToolCall(call(
                "1",
                "Read",
                json!({"file_path":"a"}),
                Some("x"),
                false,
            )),
            MessagePart::Text("ok".into()),
        ];
        let msgs = [user.clone(), a.clone()];
        let ts = build_turns(&msgs, None, latest_assistant_id(&msgs));
        assert_eq!(ts.len(), 2);
        assert!(ts[0].is_user);
        assert_eq!(ts[0].message_id, user.id);
        assert_eq!(ts[1].rows.len(), 3);
        assert!(ts[1].rows.iter().all(|r| r.message_id() == a.id));
    }

    /// The settled transcript built once plus the running message built on
    /// its own give exactly the rows of building everything together, for
    /// every stage of a turn (so streaming can rebuild only the live turn).
    #[test]
    fn a_live_turn_built_alone_matches_the_whole_transcript() {
        let mut history = Vec::new();
        for t in 0..3 {
            history.push(AgentMessage::user(format!("prompt {t}"), f64::from(t)));
            let mut a = AgentMessage::assistant_placeholder("m".into(), f64::from(t));
            a.status = MessageStatus::Finished {
                at_ms: Some(f64::from(t) + 10.0),
            };
            a.parts = vec![
                MessagePart::Thinking("hm".into()),
                MessagePart::ToolCall(call(
                    &format!("e{t}"),
                    "Edit",
                    json!({"file_path":"/r/a.rs","old_string":"1","new_string":"2"}),
                    Some("ok"),
                    false,
                )),
                MessagePart::Text(format!("done {t}")),
            ];
            history.push(a);
        }
        history.push(AgentMessage::user("live prompt".into(), 9.0));
        let stages = {
            let mut live = AgentMessage::assistant_placeholder("m".into(), 9.0);
            let mut stages = vec![live.clone()];
            live.parts.push(MessagePart::Thinking("plan".into()));
            stages.push(live.clone());
            live.parts.push(MessagePart::ToolCall(call(
                "w",
                "Write",
                json!({"file_path":"/r/b.rs","content":"x"}),
                None,
                false,
            )));
            stages.push(live.clone());
            live.parts.push(MessagePart::Text("partial".into()));
            stages.push(live.clone());
            live.error = Some("boom".into());
            live.status = MessageStatus::Finished { at_ms: Some(12.0) };
            stages.push(live);
            stages
        };
        for live in stages {
            let mut all = history.clone();
            all.push(live.clone());
            let whole = build_rows(&all, Some("/r"));

            let mut parts: Vec<TimelineRow> = build_turns(&history, Some("/r"), Some(&live.id))
                .into_iter()
                .flat_map(|t| t.rows)
                .collect();
            parts.extend(RowBuilder::after(&history, Some("/r"), Some(&live.id)).rows(&live));
            assert_eq!(parts, whole);
        }
    }

    #[test]
    fn empty_finished_turn_gets_placeholder_text_row() {
        let mut a = AgentMessage::assistant_placeholder("m".into(), 2.0);
        a.status = MessageStatus::Finished { at_ms: None };
        let rows = build_rows(&[a], None);
        assert!(matches!(
            &rows[0],
            TimelineRow::AssistantText { markdown, streaming: false, .. } if markdown.is_empty()
        ));
        assert!(matches!(&rows[1], TimelineRow::AssistantMeta { .. }));
    }
}
