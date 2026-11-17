//! Access gates: what a bruh may do at its access level, checked on every
//! tool call of an unattended run.

use serde_json::Value;
use sha2::{Digest as _, Sha256};

use super::{AccessLevel, BruhAccess};
use crate::agent::AgentProvider;

/// How long a run waits for the owner to answer an approval.
pub const APPROVAL_WAIT_MS: u64 = 15 * 60 * 1000;

/// What happens to a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Allow,
    Approve,
    Deny(&'static str),
}

/// The kinds of action the gate table covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatedAction {
    Read,
    Write,
    Delivery,
    SendAsOwner,
}

pub const READ_ONLY_DENIAL: &str = "This bruh is read only. Return a reviewable draft instead.";
pub const DELIVERY_DENIAL: &str =
    "Pushing, merging and opening pull requests need Full access with Can deliver on.";

/// Claude `--disallowedTools` rules that keep a run from delivering code.
pub const DELIVERY_DENY_RULES: [&str; 5] = [
    "Bash(git push:*)",
    "Bash(gh pr create:*)",
    "Bash(gh pr merge:*)",
    "Bash(glab mr create:*)",
    "Bash(glab mr merge:*)",
];

const SUMMARY_MAX_CHARS: usize = 200;

/// The level a run actually gets on `provider`. Codex can't ask, so Ask runs
/// read only; runtimes other than Claude and Codex always run read only.
#[must_use]
pub fn effective_level(level: AccessLevel, provider: AgentProvider) -> AccessLevel {
    match (provider, level) {
        (AgentProvider::Claude, level) => level,
        (AgentProvider::Codex, AccessLevel::Ask) => AccessLevel::ReadOnly,
        (AgentProvider::Codex, level) => level,
        _ => AccessLevel::ReadOnly,
    }
}

/// Why `level` can't be offered on `provider`, if it can't.
#[must_use]
pub fn level_unavailable(level: AccessLevel, provider: AgentProvider) -> Option<&'static str> {
    match (provider, level) {
        (AgentProvider::Claude, _) | (_, AccessLevel::ReadOnly) => None,
        (AgentProvider::Codex, AccessLevel::Ask) => {
            Some("Codex can't ask for approval, so a Codex bruh runs Read only or Full.")
        }
        (AgentProvider::Codex, AccessLevel::Full) => None,
        _ => Some("Only Claude and Codex bruhs can do more than read."),
    }
}

/// The gate table. `level` is the effective level for the run's provider.
#[must_use]
pub fn gate(access: &BruhAccess, level: AccessLevel, action: GatedAction) -> Gate {
    match (action, level) {
        (GatedAction::Read, _) => Gate::Allow,
        (GatedAction::Write | GatedAction::SendAsOwner, AccessLevel::ReadOnly) => {
            Gate::Deny(READ_ONLY_DENIAL)
        }
        (GatedAction::Write | GatedAction::SendAsOwner, AccessLevel::Ask) => Gate::Approve,
        (GatedAction::Write, AccessLevel::Full) => Gate::Allow,
        (GatedAction::SendAsOwner, AccessLevel::Full) => {
            if access.send_as_me {
                Gate::Allow
            } else {
                Gate::Approve
            }
        }
        (GatedAction::Delivery, AccessLevel::Full) if access.can_deliver => Gate::Allow,
        (GatedAction::Delivery, _) => Gate::Deny(DELIVERY_DENIAL),
    }
}

/// The gate for one tool call of a bruh run. `project_path` is the bruh's
/// working folder; `shell_confined` is true only when the run's shell is
/// sandboxed to that folder.
#[must_use]
pub fn permission_gate(
    access: &BruhAccess,
    provider: AgentProvider,
    tool: &str,
    input: &Value,
    project_path: &str,
    shell_confined: bool,
) -> Gate {
    let level = effective_level(access.level, provider);
    if tool.starts_with("mcp__") || level == AccessLevel::ReadOnly {
        return Gate::Deny(READ_ONLY_DENIAL);
    }
    match tool {
        "Bash" => {
            let command = input["command"].as_str().unwrap_or_default();
            if is_delivery_command(command) {
                gate(access, level, GatedAction::Delivery)
            } else if level == AccessLevel::Full && shell_confined {
                Gate::Allow
            } else {
                Gate::Approve
            }
        }
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = input["file_path"]
                .as_str()
                .or_else(|| input["notebook_path"].as_str())
                .unwrap_or_default();
            if inside_folder(path, project_path) {
                gate(access, level, GatedAction::Write)
            } else {
                Gate::Approve
            }
        }
        _ => Gate::Approve,
    }
}

/// Whether any command in a shell line pushes, merges or opens a pull
/// request. A lexical check: the sandbox and deny rules back it up.
#[must_use]
pub fn is_delivery_command(command: &str) -> bool {
    command
        .split(['&', '|', ';', '\n', '\r', '(', ')', '`'])
        .any(|segment| {
            let words: Vec<&str> = segment
                .split_whitespace()
                .skip_while(|word| is_env_assignment(word))
                .collect();
            match words.as_slice() {
                ["git", rest @ ..] => git_subcommand(rest) == Some("push"),
                ["gh", "pr", "create" | "merge", ..] | ["glab", "mr", "create" | "merge", ..] => {
                    true
                }
                _ => false,
            }
        })
}

fn is_env_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// The git subcommand after global options such as `-C dir` or `-c k=v`.
fn git_subcommand<'a>(words: &[&'a str]) -> Option<&'a str> {
    let mut words = words.iter();
    while let Some(&word) = words.next() {
        match word {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                words.next();
            }
            _ if word.starts_with('-') => {}
            _ => return Some(word),
        }
    }
    None
}

/// Claude deny rules for a run: delivery stays off unless the gate allows it.
#[must_use]
pub fn claude_deny_rules(access: &BruhAccess, level: AccessLevel) -> Vec<&'static str> {
    if gate(access, level, GatedAction::Delivery) == Gate::Allow {
        Vec::new()
    } else {
        DELIVERY_DENY_RULES.to_vec()
    }
}

/// Absolute path components with `.` and empty parts dropped; `None` for a
/// relative path or one with `..`.
fn absolute_components(path: &str) -> Option<Vec<&str>> {
    if !path.starts_with('/') {
        return None;
    }
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    (!parts.contains(&"..")).then_some(parts)
}

/// Whether `path` is `folder` or inside it, compared lexically. Relative
/// paths, `..` components and an empty or root folder never count.
#[must_use]
pub fn inside_folder(path: &str, folder: &str) -> bool {
    match (absolute_components(path), absolute_components(folder)) {
        (Some(path), Some(folder)) => !folder.is_empty() && path.starts_with(&folder),
        _ => false,
    }
}

/// A one line summary of a tool call for an approval card.
#[must_use]
pub fn approval_summary(tool: &str, input: &Value) -> String {
    let path = || {
        input["file_path"]
            .as_str()
            .or_else(|| input["notebook_path"].as_str())
    };
    let summary = match tool {
        "Edit" | "MultiEdit" | "NotebookEdit" => path().map(|p| format!("Edit {p}")),
        "Write" => path().map(|p| format!("Write {p}")),
        "Bash" => input["command"]
            .as_str()
            .map(|c| format!("Run {}", c.lines().next().unwrap_or_default())),
        _ => None,
    }
    .unwrap_or_else(|| tool.to_string());
    if summary.chars().count() <= SUMMARY_MAX_CHARS {
        summary
    } else {
        let mut cut: String = summary.chars().take(SUMMARY_MAX_CHARS - 1).collect();
        cut.push('…');
        cut
    }
}

/// Identity of a tool call: lowercase hex SHA-256 of the tool, a newline
/// and the compact JSON input.
#[must_use]
pub fn approval_fingerprint(tool: &str, input: &Value) -> String {
    format!("{:x}", Sha256::digest(format!("{tool}\n{input}")))
}

/// The opening lines of every bruh run: its name and what it may do.
#[must_use]
pub fn run_preamble(name: &str, access: &BruhAccess, level: AccessLevel) -> String {
    let access_line = match level {
        AccessLevel::ReadOnly => "Access: read only. You can read files in your working folder but change nothing. Produce a reviewable answer or draft.",
        AccessLevel::Ask => "Access: ask first. You may edit files and run commands in your working folder. Each change waits for the owner's approval; if one is denied, continue without it and say so.",
        AccessLevel::Full => "Access: full. You may edit files and run commands in your working folder without asking.",
    };
    let mut lines = vec![format!("Your name is {name}."), access_line.to_string()];
    if gate(access, level, GatedAction::Delivery) != Gate::Allow {
        lines.push("Never push, merge or open pull requests.".to_string());
    }
    lines.push("Never send messages in the owner's name.".to_string());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full(can_deliver: bool) -> BruhAccess {
        BruhAccess {
            level: AccessLevel::Full,
            can_deliver,
            ..Default::default()
        }
    }

    #[test]
    fn the_gate_table_follows_the_spec() {
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        assert_eq!(
            gate(&ask, AccessLevel::Ask, GatedAction::Write),
            Gate::Approve
        );
        assert_eq!(
            gate(&full(false), AccessLevel::Full, GatedAction::Write),
            Gate::Allow
        );
        assert_eq!(
            gate(
                &BruhAccess::default(),
                AccessLevel::ReadOnly,
                GatedAction::Write
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            gate(&full(false), AccessLevel::Full, GatedAction::Delivery),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            gate(&full(true), AccessLevel::Full, GatedAction::Delivery),
            Gate::Allow
        );
        let ask_deliver = BruhAccess {
            can_deliver: true,
            ..ask
        };
        assert_eq!(
            gate(&ask_deliver, AccessLevel::Ask, GatedAction::Delivery),
            Gate::Deny(DELIVERY_DENIAL)
        );
        assert_eq!(
            gate(&full(false), AccessLevel::Full, GatedAction::SendAsOwner),
            Gate::Approve
        );
        let send = BruhAccess {
            send_as_me: true,
            ..full(false)
        };
        assert_eq!(
            gate(&send, AccessLevel::Full, GatedAction::SendAsOwner),
            Gate::Allow
        );
        assert_eq!(
            gate(
                &BruhAccess {
                    send_as_me: true,
                    ..ask
                },
                AccessLevel::Ask,
                GatedAction::SendAsOwner
            ),
            Gate::Approve
        );
        assert_eq!(
            gate(
                &BruhAccess::default(),
                AccessLevel::ReadOnly,
                GatedAction::SendAsOwner
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            gate(
                &BruhAccess::default(),
                AccessLevel::ReadOnly,
                GatedAction::Read
            ),
            Gate::Allow
        );
        // The effective level, not the stored one, decides.
        assert_eq!(
            gate(&full(true), AccessLevel::ReadOnly, GatedAction::Write),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            gate(&full(true), AccessLevel::ReadOnly, GatedAction::Delivery),
            Gate::Deny(DELIVERY_DENIAL)
        );
    }

    #[test]
    fn codex_runs_an_ask_bruh_read_only() {
        assert_eq!(
            effective_level(AccessLevel::Ask, AgentProvider::Codex),
            AccessLevel::ReadOnly
        );
        assert_eq!(
            effective_level(AccessLevel::Full, AgentProvider::Codex),
            AccessLevel::Full
        );
        assert_eq!(
            effective_level(AccessLevel::Full, AgentProvider::Grok),
            AccessLevel::ReadOnly
        );
        assert_eq!(
            effective_level(AccessLevel::Ask, AgentProvider::Claude),
            AccessLevel::Ask
        );
        assert!(level_unavailable(AccessLevel::Ask, AgentProvider::Codex).is_some());
        assert_eq!(
            level_unavailable(AccessLevel::Ask, AgentProvider::Claude),
            None
        );
        assert_eq!(
            level_unavailable(AccessLevel::Full, AgentProvider::Codex),
            None
        );
        assert_eq!(
            level_unavailable(AccessLevel::Full, AgentProvider::OpenCode),
            Some("Only Claude and Codex bruhs can do more than read.")
        );
        assert_eq!(
            level_unavailable(AccessLevel::ReadOnly, AgentProvider::Cursor),
            None
        );
        let input = serde_json::json!({"file_path": "/w/x"});
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        assert_eq!(
            permission_gate(&ask, AgentProvider::Codex, "Edit", &input, "/w", true),
            Gate::Deny(READ_ONLY_DENIAL)
        );
    }

    #[test]
    fn writes_outside_the_working_folder_always_ask() {
        let input = serde_json::json!({"file_path": "/work/bruh/notes.md"});
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "Edit",
                &input,
                "/work/bruh",
                false
            ),
            Gate::Allow
        );
        for path in [
            "/etc/hosts",
            "/work/bruh/../other/x",
            "/work/bruh-evil/x",
            "notes.md",
        ] {
            let input = serde_json::json!({"file_path": path});
            assert_eq!(
                permission_gate(
                    &full(false),
                    AgentProvider::Claude,
                    "Write",
                    &input,
                    "/work/bruh",
                    true
                ),
                Gate::Approve,
                "{path}"
            );
        }
        let notebook = serde_json::json!({"notebook_path": "/work/bruh/./a.ipynb"});
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "NotebookEdit",
                &notebook,
                "/work/bruh/",
                false
            ),
            Gate::Allow
        );
        assert_eq!(
            permission_gate(
                &full(true),
                AgentProvider::Claude,
                "mcp__x__y",
                &serde_json::json!({}),
                "/work/bruh",
                true
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            permission_gate(
                &BruhAccess::default(),
                AgentProvider::Claude,
                "Edit",
                &input,
                "/work/bruh",
                true
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
        assert_eq!(
            permission_gate(
                &full(true),
                AgentProvider::Claude,
                "WebFetch",
                &serde_json::json!({}),
                "/work/bruh",
                true
            ),
            Gate::Approve
        );
    }

    #[test]
    fn inside_folder_is_lexical_and_strict() {
        assert!(inside_folder("/a/b/c", "/a/b"));
        assert!(inside_folder("/a//b/./c", "/a/b/"));
        assert!(inside_folder("/a/b", "/a/b"));
        assert!(!inside_folder("/a/bc", "/a/b"));
        assert!(!inside_folder("/a/b/../c", "/a/b"));
        assert!(!inside_folder("/a/b/c", "/a/../a/b"));
        assert!(!inside_folder("b/c", "/a"));
        assert!(!inside_folder("/a/b", "a"));
        assert!(!inside_folder("/a/b", ""));
        assert!(!inside_folder("/a/b", "/"));
    }

    #[test]
    fn full_bash_asks_unless_the_shell_is_confined() {
        let input = serde_json::json!({"command": "cargo test"});
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "Bash",
                &input,
                "/w",
                true
            ),
            Gate::Allow
        );
        assert_eq!(
            permission_gate(
                &full(false),
                AgentProvider::Claude,
                "Bash",
                &input,
                "/w",
                false
            ),
            Gate::Approve
        );
        let ask = BruhAccess {
            level: AccessLevel::Ask,
            ..Default::default()
        };
        assert_eq!(
            permission_gate(&ask, AgentProvider::Claude, "Bash", &input, "/w", true),
            Gate::Approve
        );
        assert_eq!(
            permission_gate(
                &BruhAccess::default(),
                AgentProvider::Claude,
                "Bash",
                &input,
                "/w",
                true
            ),
            Gate::Deny(READ_ONLY_DENIAL)
        );
    }

    #[test]
    fn chained_delivery_commands_are_still_delivery() {
        for command in [
            "git push origin main",
            "cd repo && git push",
            "FOO=1 gh pr merge 3",
            "make; glab mr create",
            "cargo test\ngit push",
            "make & git push",
            "echo $(git push)",
            "git -C repo push",
            "  gh pr create --fill",
        ] {
            assert!(is_delivery_command(command), "{command}");
            let input = serde_json::json!({"command": command});
            for confined in [true, false] {
                assert_eq!(
                    permission_gate(
                        &full(false),
                        AgentProvider::Claude,
                        "Bash",
                        &input,
                        "/w",
                        confined
                    ),
                    Gate::Deny(DELIVERY_DENIAL)
                );
            }
        }
        for command in [
            "git status && cargo test",
            "echo git push",
            "gh pr view 3",
            "git pushd",
            "",
        ] {
            assert!(!is_delivery_command(command), "{command}");
        }
        let push = serde_json::json!({"command": "git push"});
        assert_eq!(
            permission_gate(
                &full(true),
                AgentProvider::Claude,
                "Bash",
                &push,
                "/w",
                false
            ),
            Gate::Allow
        );
        assert_eq!(
            claude_deny_rules(&full(true), AccessLevel::Full),
            Vec::<&str>::new()
        );
        assert_eq!(
            claude_deny_rules(&full(false), AccessLevel::Full),
            DELIVERY_DENY_RULES.to_vec()
        );
    }

    #[test]
    fn approvals_have_short_summaries_and_stable_fingerprints() {
        let input = serde_json::json!({"command": "cargo test\nsecond line"});
        assert_eq!(approval_summary("Bash", &input), "Run cargo test");
        assert_eq!(
            approval_summary("Edit", &serde_json::json!({"file_path": "/w/a.rs"})),
            "Edit /w/a.rs"
        );
        assert_eq!(
            approval_summary("Write", &serde_json::json!({"file_path": "/w/a.rs"})),
            "Write /w/a.rs"
        );
        assert_eq!(
            approval_summary("WebFetch", &serde_json::json!({})),
            "WebFetch"
        );
        assert_eq!(
            approval_fingerprint("Bash", &input),
            approval_fingerprint("Bash", &input.clone())
        );
        assert_ne!(
            approval_fingerprint("Bash", &input),
            approval_fingerprint("Bash", &serde_json::json!({"command": "ls"}))
        );
        assert_ne!(
            approval_fingerprint("Bash", &input),
            approval_fingerprint("Write", &input)
        );
        let fingerprint = approval_fingerprint("Bash", &input);
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        let long = approval_summary("Edit", &serde_json::json!({"file_path": "x".repeat(400)}));
        assert_eq!(long.chars().count(), 200);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn the_preamble_names_the_bruh_and_its_access() {
        let text = run_preamble("Penny", &full(false), AccessLevel::Full);
        assert!(text.starts_with("Your name is Penny."));
        assert!(
            text.contains("without asking")
                && text.contains("Never push, merge or open pull requests.")
        );
        assert!(!run_preamble("Penny", &full(true), AccessLevel::Full).contains("Never push"));
        assert_eq!(
            run_preamble("Penny", &BruhAccess::default(), AccessLevel::ReadOnly),
            "Your name is Penny.\n\
             Access: read only. You can read files in your working folder but change nothing. Produce a reviewable answer or draft.\n\
             Never push, merge or open pull requests.\n\
             Never send messages in the owner's name."
        );
        let ask = run_preamble(
            "Penny",
            &BruhAccess {
                level: AccessLevel::Ask,
                can_deliver: true,
                ..Default::default()
            },
            AccessLevel::Ask,
        );
        assert!(ask.contains("Access: ask first.") && ask.contains("Never push"));
        assert!(run_preamble("Penny", &full(true), AccessLevel::Full)
            .ends_with("Never send messages in the owner's name."));
    }
}
