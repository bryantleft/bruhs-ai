//! How the chat timeline names tool calls: kinds, states, present/past-tense
//! labels and one-line group summaries. Moved from bruhs-web
//! `components/timeline.rs` (the reference) and bruhs-desktop
//! `views/chat/timeline_ui.rs` (identical logic over its own call type).
//!
//! Each surface keeps its own tool-call struct and implements
//! [`ToolCallView`] for it.

use std::collections::HashMap;

use serde_json::Value;

/// Read access to one tool call, whatever struct a surface stores it in.
pub trait ToolCallView {
    /// Tool name as the provider reported it (`Read`, `Bash`, `mcp__x__y`).
    fn tool_name(&self) -> &str;
    /// The call's JSON input.
    fn input(&self) -> &Value;
    /// Whether the call has produced output yet.
    fn has_output(&self) -> bool;
    /// Whether that output is an error.
    fn is_error(&self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Read,
    Edit,
    Write,
    Bash,
    Search,
    Web,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Done,
    Error,
    /// The turn ended (stop, crash, reload) before this call produced output.
    Interrupted,
}

pub fn tool_kind(tool: &str) -> ToolKind {
    match tool {
        "Read" | "NotebookRead" => ToolKind::Read,
        "Edit" | "MultiEdit" | "NotebookEdit" => ToolKind::Edit,
        "Write" => ToolKind::Write,
        "Bash" => ToolKind::Bash,
        "Glob" | "Grep" | "LS" => ToolKind::Search,
        "WebSearch" | "WebFetch" => ToolKind::Web,
        _ => ToolKind::Other,
    }
}

/// `Error`/`Done` once the call has output; otherwise `Running` while its
/// message streams and `Interrupted` after.
pub fn tool_state(call: &impl ToolCallView, message_streaming: bool) -> ToolState {
    match (call.has_output(), call.is_error(), message_streaming) {
        (true, true, _) => ToolState::Error,
        (true, false, _) => ToolState::Done,
        (false, _, true) => ToolState::Running,
        (false, _, false) => ToolState::Interrupted,
    }
}

/// `path` relative to `root` when it lives inside it; otherwise unchanged.
pub fn relative_path(path: &str, root: Option<&str>) -> String {
    if let Some(root) = root
        .map(|r| r.trim_end_matches('/'))
        .filter(|r| !r.is_empty())
    {
        if let Some(rest) = path.strip_prefix(root) {
            if rest.is_empty() {
                return ".".to_string();
            }
            if let Some(rel) = rest.strip_prefix('/') {
                return rel.to_string();
            }
        }
    }
    path.to_string()
}

/// Unwrap `sh -c "…"` / `/bin/zsh -lc '…'` wrappers around a command.
pub fn unwrap_shell(command: &str) -> &str {
    let mut it = command.split_whitespace();
    let (Some(prog), Some(flag)) = (it.next(), it.next()) else {
        return command;
    };
    let base = prog.rsplit('/').next().unwrap_or(prog);
    let is_shell = matches!(base, "sh" | "bash" | "zsh" | "dash" | "fish");
    let is_c_flag = flag.starts_with('-') && flag.contains('c') && flag.len() <= 4;
    if !(is_shell && is_c_flag) {
        return command;
    }
    // `prog` is the first whitespace-separated token, so it starts at the
    // first non-whitespace byte, not necessarily at 0.
    let prog_end = command.find(prog).map_or(prog.len(), |at| at + prog.len());
    let after_prog = &command[prog_end..];
    let Some(flag_at) = after_prog.find(flag) else {
        return command;
    };
    after_prog[flag_at + flag.len()..]
        .trim()
        .trim_matches(|c| c == '\'' || c == '"')
}

/// The program a shell command runs: strips shell wrappers, a leading
/// `cd x &&`, env assignments and `sudo`/`env`/`time` prefixes; returns the
/// first real token (basename for absolute paths).
pub fn bash_program(command: &str) -> String {
    let unwrapped = unwrap_shell(command.trim());
    let mut segment = unwrapped;
    for part in unwrapped.split("&&") {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        if p == "cd" || p.starts_with("cd ") {
            continue;
        }
        segment = p;
        break;
    }
    let token = segment
        .split_whitespace()
        .find(|t| {
            let is_env_assign = t.contains('=') && !t.starts_with('-');
            let is_prefix = matches!(*t, "env" | "sudo" | "time" | "nohup" | "exec" | "command");
            !is_env_assign && !is_prefix
        })
        .unwrap_or("");
    if token.starts_with('/') {
        token.rsplit('/').next().unwrap_or(token).to_string()
    } else {
        token.to_string()
    }
}

fn host_of(url: &str) -> String {
    let no_scheme = url.split("//").nth(1).unwrap_or(url);
    no_scheme.split('/').next().unwrap_or("").to_string()
}

/// Human label for a tool call — present tense while running, past once
/// done. Paths are shown relative to `root`.
pub fn tool_label(call: &impl ToolCallView, state: ToolState, root: Option<&str>) -> String {
    let running = state == ToolState::Running;
    let tool = call.tool_name();
    let s = |k: &str| -> String {
        call.input()
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let path = || {
        let raw = if s("file_path").is_empty() {
            if s("notebook_path").is_empty() {
                s("path")
            } else {
                s("notebook_path")
            }
        } else {
            s("file_path")
        };
        relative_path(&raw, root)
    };
    let tense = |now: &'static str, past: &'static str| -> &'static str {
        if running {
            now
        } else {
            past
        }
    };
    let label = match tool_kind(tool) {
        ToolKind::Read => format!("{} {}", tense("Reading", "Read"), path()),
        ToolKind::Edit => format!("{} {}", tense("Editing", "Edited"), path()),
        ToolKind::Write => format!("{} {}", tense("Writing", "Wrote"), path()),
        ToolKind::Bash => {
            let program = bash_program(&s("command"));
            if program.is_empty() {
                tense("Running a command", "Ran a command").to_string()
            } else {
                format!("{} {}", tense("Running", "Ran"), program)
            }
        }
        ToolKind::Search => {
            let needle = if s("pattern").is_empty() {
                if s("query").is_empty() {
                    s("path")
                } else {
                    s("query")
                }
            } else {
                s("pattern")
            };
            format!("{} {}", tense("Searching", "Searched"), needle)
        }
        ToolKind::Web => {
            if tool == "WebFetch" {
                format!("{} {}", tense("Fetching", "Fetched"), host_of(&s("url")))
            } else {
                let q = s("query");
                if q.is_empty() {
                    tense("Searching the web", "Searched the web").to_string()
                } else {
                    format!("{} the web for {}", tense("Searching", "Searched"), q)
                }
            }
        }
        ToolKind::Other => match tool {
            "Task" | "Agent" => {
                let d = s("description");
                if d.is_empty() {
                    tense("Delegating to an agent", "Delegated to an agent").to_string()
                } else {
                    format!("{} agent: {}", tense("Running", "Ran"), d)
                }
            }
            "TodoWrite" => tense("Updating todos", "Updated todos").to_string(),
            "AskUserQuestion" => tense("Asking a question", "Asked a question").to_string(),
            "mcp__bruhs_secrets__request_secret" => format!(
                "{} {}",
                tense("Requesting credential", "Requested credential"),
                s("name")
            ),
            "Skill" => {
                let name = s("skill");
                if name.is_empty() {
                    tense("Loading a skill", "Loaded a skill").to_string()
                } else {
                    format!("{} skill {}", tense("Loading", "Loaded"), name)
                }
            }
            other => {
                if let Some(rest) = other.strip_prefix("mcp__") {
                    let mut it = rest.splitn(2, "__");
                    let server = it.next().unwrap_or("");
                    let tool = it.next().unwrap_or("");
                    if tool.is_empty() {
                        format!("{} {}", tense("Using", "Used"), server)
                    } else {
                        format!("{} {} ({})", tense("Using", "Used"), tool, server)
                    }
                } else {
                    format!("{} {}", tense("Using", "Used"), other)
                }
            }
        },
    };
    label.trim_end().to_string()
}

/// One-line summary of a collapsed tool group, e.g.
/// `Read 2 files, edited 1 file and ran 1 command`. Writes count as edits
/// and web lookups as searches.
pub fn work_summary(kinds: &[ToolKind]) -> String {
    let bucket = |k: ToolKind| match k {
        ToolKind::Write => ToolKind::Edit,
        ToolKind::Web => ToolKind::Search,
        other => other,
    };
    let mut order: Vec<ToolKind> = Vec::new();
    let mut counts: HashMap<ToolKind, usize> = HashMap::new();
    for kind in kinds {
        let k = bucket(*kind);
        if !order.contains(&k) {
            order.push(k);
        }
        *counts.entry(k).or_insert(0) += 1;
    }
    let phrase = |k: ToolKind, n: usize, first: bool| -> String {
        let plural = |one: &str, many: &str| {
            if n == 1 {
                one.to_string()
            } else {
                many.to_string()
            }
        };
        let (verb, noun) = match k {
            ToolKind::Read => (if first { "Read" } else { "read" }, plural("file", "files")),
            ToolKind::Edit | ToolKind::Write => (
                if first { "Edited" } else { "edited" },
                plural("file", "files"),
            ),
            ToolKind::Bash => (
                if first { "Ran" } else { "ran" },
                plural("command", "commands"),
            ),
            ToolKind::Search | ToolKind::Web => (
                if first { "Searched" } else { "searched" },
                plural("time", "times"),
            ),
            ToolKind::Other => (if first { "Used" } else { "used" }, plural("tool", "tools")),
        };
        format!("{verb} {n} {noun}")
    };
    let parts: Vec<String> = order
        .iter()
        .enumerate()
        .map(|(i, k)| phrase(*k, counts.get(k).copied().unwrap_or(0), i == 0))
        .collect();
    match parts.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {}", init.join(", "), last),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Call {
        tool: &'static str,
        input: Value,
        output: Option<&'static str>,
        is_error: bool,
    }

    impl ToolCallView for Call {
        fn tool_name(&self) -> &str {
            self.tool
        }
        fn input(&self) -> &Value {
            &self.input
        }
        fn has_output(&self) -> bool {
            self.output.is_some()
        }
        fn is_error(&self) -> bool {
            self.is_error
        }
    }

    fn call(
        tool: &'static str,
        input: Value,
        output: Option<&'static str>,
        is_error: bool,
    ) -> Call {
        Call {
            tool,
            input,
            output,
            is_error,
        }
    }

    #[test]
    fn labels_use_tense_and_relative_paths() {
        let c = call(
            "Read",
            json!({"file_path":"/repo/src/main.rs"}),
            None,
            false,
        );
        assert_eq!(
            tool_label(&c, ToolState::Running, Some("/repo")),
            "Reading src/main.rs"
        );
        assert_eq!(
            tool_label(&c, ToolState::Done, Some("/repo")),
            "Read src/main.rs"
        );
        assert_eq!(
            tool_label(&c, ToolState::Done, Some("/other")),
            "Read /repo/src/main.rs"
        );
        let b = call(
            "Bash",
            json!({"command":"/bin/zsh -lc 'cargo test --all'"}),
            Some(""),
            false,
        );
        assert_eq!(tool_label(&b, ToolState::Done, None), "Ran cargo");
        let g = call("Grep", json!({"pattern":"TODO"}), Some(""), false);
        assert_eq!(tool_label(&g, ToolState::Done, None), "Searched TODO");
        let w = call(
            "WebFetch",
            json!({"url":"https://docs.rs/leptos/latest"}),
            None,
            false,
        );
        assert_eq!(tool_label(&w, ToolState::Running, None), "Fetching docs.rs");
        let m = call("mcp__linear__create_issue", json!({}), Some(""), false);
        assert_eq!(
            tool_label(&m, ToolState::Done, None),
            "Used create_issue (linear)"
        );
        let e = call("Bash", json!({"command":""}), Some(""), false);
        assert_eq!(tool_label(&e, ToolState::Done, None), "Ran a command");
    }

    #[test]
    fn other_tools_have_their_own_phrases() {
        let label =
            |tool, input| tool_label(&call(tool, input, None, false), ToolState::Done, None);
        assert_eq!(
            label("Task", json!({"description":"triage"})),
            "Ran agent: triage"
        );
        assert_eq!(label("Agent", json!({})), "Delegated to an agent");
        assert_eq!(label("TodoWrite", json!({})), "Updated todos");
        assert_eq!(label("AskUserQuestion", json!({})), "Asked a question");
        assert_eq!(label("Skill", json!({"skill":"pdf"})), "Loaded skill pdf");
        assert_eq!(
            label("WebSearch", json!({"query":"leptos"})),
            "Searched the web for leptos"
        );
        assert_eq!(label("WebSearch", json!({})), "Searched the web");
        assert_eq!(label("mcp__bruhs", json!({})), "Used bruhs");
        assert_eq!(label("Frobnicate", json!({})), "Used Frobnicate");
        assert_eq!(
            label("NotebookEdit", json!({"notebook_path":"/n.ipynb"})),
            "Edited /n.ipynb"
        );
        assert_eq!(label("Read", json!({})), "Read");
    }

    #[test]
    fn bash_program_strips_wrappers() {
        assert_eq!(bash_program("cd /x && FOO=1 npm test"), "npm");
        assert_eq!(bash_program("git status"), "git");
        assert_eq!(bash_program("sh -c \"ls -la\""), "ls");
        assert_eq!(bash_program("/bin/zsh -lc pwd"), "pwd");
        assert_eq!(bash_program("sudo /usr/bin/apt install x"), "apt");
        assert_eq!(bash_program("cd /x"), "cd");
        assert_eq!(bash_program("  bash -c 'make'"), "make");
        assert_eq!(bash_program(""), "");
    }

    #[test]
    fn unwrap_shell_leaves_non_wrappers_alone() {
        assert_eq!(unwrap_shell("sh -c 'ls'"), "ls");
        assert_eq!(unwrap_shell("python -c 'x'"), "python -c 'x'");
        assert_eq!(unwrap_shell("bash --login"), "bash --login");
        assert_eq!(unwrap_shell("ls"), "ls");
    }

    #[test]
    fn tool_state_reflects_output_and_streaming() {
        let done = call("Read", json!({}), Some("x"), false);
        assert_eq!(tool_state(&done, true), ToolState::Done);
        let err = call("Read", json!({}), Some("x"), true);
        assert_eq!(tool_state(&err, false), ToolState::Error);
        let pending = call("Read", json!({}), None, false);
        assert_eq!(tool_state(&pending, true), ToolState::Running);
        assert_eq!(tool_state(&pending, false), ToolState::Interrupted);
    }

    #[test]
    fn work_summary_counts_by_verb() {
        let kinds = [
            ToolKind::Read,
            ToolKind::Read,
            ToolKind::Edit,
            ToolKind::Bash,
        ];
        assert_eq!(
            work_summary(&kinds),
            "Read 2 files, edited 1 file and ran 1 command"
        );
        assert_eq!(work_summary(&kinds[..1]), "Read 1 file");
        assert_eq!(
            work_summary(&[ToolKind::Write, ToolKind::Edit, ToolKind::Web]),
            "Edited 2 files and searched 1 time"
        );
        assert_eq!(
            work_summary(&[ToolKind::Other, ToolKind::Other]),
            "Used 2 tools"
        );
        assert_eq!(work_summary(&[]), "");
    }

    #[test]
    fn relative_path_handles_root_edge_cases() {
        assert_eq!(relative_path("/r/a.rs", Some("/r/")), "a.rs");
        assert_eq!(relative_path("/r", Some("/r")), ".");
        assert_eq!(relative_path("/rx/a.rs", Some("/r")), "/rx/a.rs");
        assert_eq!(relative_path("a.rs", None), "a.rs");
        assert_eq!(relative_path("/a.rs", Some("/")), "/a.rs");
    }
}
