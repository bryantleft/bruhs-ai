//! Token numbers from Codex's saved session ("rollout") file.
//!
//! `codex exec --json` ends a turn with `turn.completed`, whose `usage` is
//! the thread's running total, and it says nothing about how full the
//! context window is. The rollout under `$CODEX_HOME/sessions/` records a
//! `token_count` event after every model request with both the running
//! total and the last request's usage, so a turn's own spend is the total
//! now minus the total before the turn, and the context is the last
//! request's input.

use crate::types::UsageInfo;
use std::path::{Path, PathBuf};

/// `$CODEX_HOME`, as the spawned CLI will see it, else `$HOME/.codex` with
/// the CLI's `HOME`.
pub(crate) fn codex_home(
    env: &[(String, String)],
    env_remove: &[String],
    inherit_env: bool,
) -> Option<PathBuf> {
    let seen = |key: &str| {
        child_var(key, env, env_remove, inherit_env, |name: &str| {
            std::env::var_os(name)
        })
    };
    seen("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| seen("HOME").map(|home| Path::new(&home).join(".codex")))
}

/// The value of `key` in the spawned CLI's environment: an explicit `env`
/// entry wins, then the inherited one unless removed or not inherited.
fn child_var(
    key: &str,
    env: &[(String, String)],
    env_remove: &[String],
    inherit_env: bool,
    inherited: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Option<std::ffi::OsString> {
    if let Some((_, value)) = env.iter().rev().find(|(name, _)| name == key) {
        return Some(value.into());
    }
    let kept = inherit_env && !env_remove.iter().any(|name| name == key);
    kept.then(|| inherited(key)).flatten()
}

/// The rollout of `thread_id`: `sessions/YYYY/MM/DD/rollout-…-<id>.jsonl`.
/// Newest days are searched first, since a resumed thread is usually recent.
pub(crate) fn find_rollout(codex_home: &Path, thread_id: &str) -> Option<PathBuf> {
    let suffix = format!("-{thread_id}.jsonl");
    let mut days = Vec::new();
    for year in sorted_dirs(&codex_home.join("sessions")) {
        for month in sorted_dirs(&year) {
            days.extend(sorted_dirs(&month));
        }
    }
    days.sort();
    days.iter().rev().find_map(|day| {
        std::fs::read_dir(day).ok()?.flatten().find_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            (name.starts_with("rollout-") && name.ends_with(&suffix)).then(|| entry.path())
        })
    })
}

fn sorted_dirs(path: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect();
    dirs.sort();
    dirs
}

/// One `token_count` event's numbers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TokenCount {
    total: UsageInfo,
    last: UsageInfo,
}

/// The usage of the turn whose rollout lines start at byte `offset` of
/// `contents`: spent = last total after `offset` minus last total before it,
/// with the context from the last request. `None` when the turn wrote no
/// `token_count`.
pub(crate) fn turn_usage(contents: &str, offset: usize) -> Option<UsageInfo> {
    let offset = floor_char_boundary(contents, offset);
    let (before, during) = contents.split_at(offset);
    let previous = last_token_count(before).map(|count| count.total);
    let current = last_token_count(during)?;
    let previous = previous.unwrap_or_default();
    Some(UsageInfo {
        input_tokens: current
            .total
            .input_tokens
            .saturating_sub(previous.input_tokens),
        cached_input_tokens: current
            .total
            .cached_input_tokens
            .saturating_sub(previous.cached_input_tokens),
        output_tokens: current
            .total
            .output_tokens
            .saturating_sub(previous.output_tokens),
        reasoning_output_tokens: current
            .total
            .reasoning_output_tokens
            .saturating_sub(previous.reasoning_output_tokens),
        context_input_tokens: Some(current.last.input_tokens),
    })
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn last_token_count(lines: &str) -> Option<TokenCount> {
    lines
        .lines()
        .rev()
        .filter(|line| line.contains("\"token_count\""))
        .find_map(parse_token_count)
}

fn parse_token_count(line: &str) -> Option<TokenCount> {
    let json: serde_json::Value = serde_json::from_str(line).ok()?;
    let payload = json.get("payload")?;
    if payload.get("type")?.as_str()? != "token_count" {
        return None;
    }
    let info = payload.get("info").filter(|info| !info.is_null())?;
    let usage =
        |key: &str| -> Option<UsageInfo> { serde_json::from_value(info.get(key)?.clone()).ok() };
    Some(TokenCount {
        total: usage("total_token_usage")?,
        last: usage("last_token_usage")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(total_in: u64, total_cached: u64, total_out: u64, last_in: u64) -> String {
        format!(
            r#"{{"type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":{total_in},"cached_input_tokens":{total_cached},"output_tokens":{total_out},"reasoning_output_tokens":0}},"last_token_usage":{{"input_tokens":{last_in},"cached_input_tokens":0,"output_tokens":1,"reasoning_output_tokens":0}}}}}}}}"#
        )
    }

    /// Real totals from a codex-cli 0.166 thread: 21,739 → 43,496 → 74,625.
    #[test]
    fn a_turn_spends_the_total_minus_the_previous_total() {
        let before = format!(
            "{}\n{}\n",
            count(21_739, 13_184, 5, 21_739),
            count(43_496, 34_688, 16, 21_757)
        );
        let during = format!(
            "{{\"type\":\"response_item\"}}\n{}\n{}\n",
            count(60_000, 45_000, 20, 16_504),
            count(74_625, 56_320, 37, 31_129)
        );
        let contents = format!("{before}{during}");
        let usage = turn_usage(&contents, before.len()).expect("turn usage");
        assert_eq!(usage.input_tokens, 74_625 - 43_496);
        assert_eq!(usage.cached_input_tokens, 56_320 - 34_688);
        assert_eq!(usage.output_tokens, 21);
        assert_eq!(usage.context_input_tokens, Some(31_129), "last request");
    }

    #[test]
    fn a_new_thread_starts_from_zero() {
        let contents = format!("{}\n", count(19_956, 13_184, 10, 19_956));
        let usage = turn_usage(&contents, 0).expect("turn usage");
        assert_eq!(usage.input_tokens, 19_956);
        assert_eq!(usage.context_input_tokens, Some(19_956));
    }

    #[test]
    fn no_token_count_this_turn_is_none_and_null_info_is_skipped() {
        let before = format!("{}\n", count(10, 0, 1, 10));
        let null = r#"{"type":"event_msg","payload":{"type":"token_count","info":null}}"#;
        let contents = format!("{before}{null}\n");
        assert_eq!(turn_usage(&contents, before.len()), None);
        assert_eq!(turn_usage("", 0), None);
    }

    #[test]
    fn rollouts_are_found_by_thread_id_newest_day_first() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join("sessions/2026/10/01");
        let new = home.path().join("sessions/2026/11/07");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("rollout-2026-10-01T00-00-00-other.jsonl"), "").unwrap();
        let wanted = new.join("rollout-2026-11-07T21-34-23-thread-9.jsonl");
        std::fs::write(&wanted, "").unwrap();
        assert_eq!(find_rollout(home.path(), "thread-9"), Some(wanted));
        assert_eq!(find_rollout(home.path(), "missing"), None);
        assert_eq!(find_rollout(&home.path().join("nope"), "thread-9"), None);
    }

    #[test]
    fn an_explicit_codex_home_wins() {
        let env = vec![("CODEX_HOME".to_string(), "/x".to_string())];
        assert_eq!(codex_home(&env, &[], true), Some(PathBuf::from("/x")));
    }

    /// The SDK must look where the CLI writes: the child's `HOME`, not ours.
    #[test]
    fn the_childs_environment_decides() {
        let ours = |key: &str| (key == "HOME").then(|| std::ffi::OsString::from("/ours"));
        let home = vec![("HOME".to_string(), "/x".to_string())];
        assert_eq!(child_var("HOME", &home, &[], true, ours), Some("/x".into()));
        assert_eq!(
            child_var("HOME", &[], &[], true, ours),
            Some("/ours".into())
        );
        assert_eq!(
            child_var("HOME", &[], &[], false, ours),
            None,
            "not inherited"
        );
        assert_eq!(
            child_var("HOME", &[], &["HOME".to_string()], true, ours),
            None,
            "removed"
        );
    }
}
