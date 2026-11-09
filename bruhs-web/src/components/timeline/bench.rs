//! Per-token cost of the chat transcript's reactive data work, measured
//! natively over a 500-message thread. Not a correctness test: run with
//! `cargo test -p bruhs-web --release -- --ignored --nocapture streaming_tick_cost`.

use bruhs_models::num::ToFloatLossy;
use std::time::Instant;

use serde_json::json;

use super::*;
use crate::state::{AgentMessage, MessagePart, MessageStatus, ToolCallInfo};

const MESSAGES: usize = 500;
const TOKENS: usize = 300;

fn call(id: String, tool: &str, input: serde_json::Value) -> ToolCallInfo {
    ToolCallInfo {
        id,
        tool: tool.into(),
        input,
        output: Some("ok".into()),
        is_error: false,
        started_at_ms: Some(0.0),
        ended_at_ms: Some(1.0),
    }
}

/// 250 settled turns (prompt + an answer with thinking, a read, an edit of a
/// 20-line block and ~1.5 KB of prose) plus the prompt of the live turn.
fn fixture() -> Vec<AgentMessage> {
    let old: String = (0..20).map(|i| format!("let a{i} = {i};\n")).collect();
    let new: String = (0..20)
        .map(|i| format!("let a{i} = {};\n", i * 2))
        .collect();
    let prose = "Some **markdown** prose with `code` and a [link](https://x.y). ".repeat(24);
    let mut out = Vec::with_capacity(MESSAGES + 2);
    for t in 0..(MESSAGES / 2) {
        out.push(AgentMessage::user(format!("prompt {t}"), t.to_f64_lossy()));
        let mut a =
            AgentMessage::assistant_placeholder("claude-sonnet-4-6".into(), t.to_f64_lossy());
        a.status = MessageStatus::Finished {
            at_ms: Some(t.to_f64_lossy() + 5000.0),
        };
        a.parts = vec![
            MessagePart::Thinking("thinking about it".into()),
            MessagePart::ToolCall(call(
                format!("{t}-r"),
                "Read",
                json!({"file_path": format!("/repo/src/f{t}.rs")}),
            )),
            MessagePart::ToolCall(call(
                format!("{t}-e"),
                "Edit",
                json!({"file_path": format!("/repo/src/f{t}.rs"), "old_string": old, "new_string": new}),
            )),
            MessagePart::Text(prose.clone()),
        ];
        out.push(a);
    }
    out
}

/// Group rows by message (the old `turns`).
fn group(rows: &[TimelineRow]) -> Vec<(String, Vec<TimelineRow>)> {
    let mut out: Vec<(String, Vec<TimelineRow>)> = Vec::new();
    for row in rows {
        match out.last_mut() {
            Some((id, rs)) if id == row.message_id() => rs.push(row.clone()),
            _ => out.push((row.message_id().to_string(), vec![row.clone()])),
        }
    }
    out
}

fn push_token(m: &mut AgentMessage, i: usize) {
    match m.parts.last_mut() {
        Some(MessagePart::Text(t)) => t.push_str("tok "),
        _ => m.parts.push(MessagePart::Text(format!("tok{i} "))),
    }
}

fn per_token_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0 / TOKENS.to_f64_lossy()
}

/// Per token, before: the stream buffer is patched, cloned whole into
/// `messages`, every row rebuilt, the turn list regrouped, and every turn's
/// rows re-filtered out of the full row list (each memo then compares).
/// After: only the live message is patched and only its turn rebuilt and
/// compared; the settled transcript's memos are not touched.
#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn streaming_tick_cost() {
    let root = Some("/repo");
    let settled = {
        let mut s = fixture();
        s.push(AgentMessage::user("live prompt".into(), 1.0));
        s
    };
    let placeholder = AgentMessage::assistant_placeholder("m".into(), 1.0);

    // ── Before ──────────────────────────────────────────────────────────
    let mut buffer = settled.clone();
    buffer.push(placeholder.clone());
    let mut prev_rows = build_rows(&buffer, root);
    let mut prev_turns: Vec<String> = group(&prev_rows).into_iter().map(|(id, _)| id).collect();
    let mut prev_turn_rows: Vec<Vec<TimelineRow>> = Vec::new();
    let mut sink = 0usize;
    let started = Instant::now();
    for i in 0..TOKENS {
        if let Some(m) = buffer.last_mut() {
            push_token(m, i);
        }
        let visible = buffer.clone();
        let rows = build_rows(&visible, root);
        let rows_changed = rows != prev_rows;
        let turn_list: Vec<String> = group(&rows).into_iter().map(|(id, _)| id).collect();
        let list_changed = turn_list != prev_turns;
        let turn_rows: Vec<Vec<TimelineRow>> = turn_list
            .iter()
            .map(|mid| {
                rows.iter()
                    .filter(|r| r.message_id() == mid)
                    .cloned()
                    .collect()
            })
            .collect();
        let changed = turn_rows
            .iter()
            .zip(prev_turn_rows.iter())
            .filter(|(a, b)| a != b)
            .count();
        sink += usize::from(rows_changed) + usize::from(list_changed) + changed + visible.len();
        prev_rows = rows;
        prev_turns = turn_list;
        prev_turn_rows = turn_rows;
    }
    let before = per_token_ms(started);

    // ── After ───────────────────────────────────────────────────────────
    let mut live = placeholder;
    let settled_turns = build_turns(&settled, root, Some(&live.id));
    let mut prev_live: Option<Turn> = None;
    let mut prev_live_rows: Vec<TimelineRow> = Vec::new();
    let started = Instant::now();
    for i in 0..TOKENS {
        push_token(&mut live, i);
        // live_id memo
        let live_id = Some(live.id.clone());
        // live_turn memo
        let turn = RowBuilder::after(&settled, root, live_id.as_deref()).turn(&live);
        let live_changed = Some(&turn) != prev_live.as_ref();
        // the running turn's rows memo
        let rows = turn.rows.clone();
        let rows_changed = rows != prev_live_rows;
        sink += usize::from(live_changed) + usize::from(rows_changed) + settled_turns.len();
        prev_live = Some(turn);
        prev_live_rows = rows;
    }
    let after = per_token_ms(started);

    println!(
        "{TOKENS} tokens streamed into a {MESSAGES}-message thread: \
         before {before:.3} ms/token, after {after:.4} ms/token ({:.0}x) (sink {sink})",
        before / after
    );
}
