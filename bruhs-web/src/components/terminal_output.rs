//! Terminal-style output block for Bash tool bodies: command header with a
//! copy button, monospace output, capped so huge logs stay scrollable.

use leptos::prelude::*;

use crate::format::truncate_chars;
use crate::ui::CopyButton;

pub const TERMINAL_MAX_LINES: usize = 200;
pub const TERMINAL_MAX_CHARS: usize = 20_000;

/// Output capped to `max_lines` / `max_chars` (char-safe), plus a note
/// describing what was cut, if anything.
pub fn cap_output(output: &str, max_lines: usize, max_chars: usize) -> (String, Option<String>) {
    let total_lines = output.lines().count();
    let mut shown: String = output
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    let mut notes: Vec<String> = Vec::new();
    if total_lines > max_lines {
        notes.push(format!("{} more lines", total_lines - max_lines));
    }
    if shown.chars().count() > max_chars {
        shown = truncate_chars(&shown, max_chars);
        notes.push("output truncated".to_string());
    }
    let note = if notes.is_empty() {
        None
    } else {
        Some(format!("… {}", notes.join(", ")))
    };
    (shown, note)
}

#[component]
pub fn TerminalOutput(#[prop(optional, into)] command: String, output: String) -> impl IntoView {
    let (shown, note) = cap_output(&output, TERMINAL_MAX_LINES, TERMINAL_MAX_CHARS);
    let header = if command.is_empty() {
        "output".to_string()
    } else {
        format!("$ {command}")
    };
    let has_output = !shown.trim().is_empty();
    let copy_text = if command.is_empty() {
        output
    } else if output.is_empty() {
        command.clone()
    } else {
        format!("$ {command}\n{output}")
    };

    view! {
        <div class="overflow-hidden rounded-grape bg-longan-950 inset-ring inset-ring-white/5">
            <div class="flex items-center gap-2 border-b border-longan-800/50 px-3 py-1 text-code-sm text-rambutan-400">
                <span class="min-w-0 flex-1 truncate" title=command>{header}</span>
                <CopyButton text=copy_text label="Copy command and output" />
            </div>
            {if has_output {
                view! {
                    <pre class="max-h-64 overflow-auto whitespace-pre-wrap break-all px-3 py-2 text-code-sm text-rambutan-300">
                        {shown}
                    </pre>
                }.into_any()
            } else {
                view! {
                    <p class="px-3 py-2 text-code-sm text-rambutan-500">"(no output)"</p>
                }.into_any()
            }}
            {note.map(|n| view! {
                <p class="border-t border-longan-800/50 px-3 py-1 text-label-sm text-rambutan-500">{n}</p>
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_output_limits_lines_and_chars() {
        let many: String = (0..300).map(|i| format!("line {i}\n")).collect();
        let (shown, note) = cap_output(&many, 200, 20_000);
        assert_eq!(shown.lines().count(), 200);
        assert_eq!(note.as_deref(), Some("… 100 more lines"));

        let wide = "é".repeat(50);
        let (shown, note) = cap_output(&wide, 200, 10);
        assert_eq!(shown.chars().count(), 11);
        assert!(shown.ends_with('…'));
        assert_eq!(note.as_deref(), Some("… output truncated"));

        let (shown, note) = cap_output("ok", 200, 20_000);
        assert_eq!(shown, "ok");
        assert!(note.is_none());
    }
}
