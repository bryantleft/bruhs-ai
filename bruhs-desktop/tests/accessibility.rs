//! Static accessibility audit.
//!
//! The macOS AX API can only be queried from a process the user has granted
//! Accessibility permission, which CI and agents do not have. The part
//! that is about *our* elements — does every control announce itself, and can
//! a keyboard user see where they are — is a property of the source, so it is
//! checked here instead.
//!
//! Rules, both of which the web gets for free from real `<button>`s:
//! 1. Anything focusable (`tab_index`) must show a focus ring
//!    (`focus_visible`); gpui draws nothing unless asked.
//! 2. Anything with `Role::Button` must have an accessible name
//!    (`aria_label`, or a `tooltip`, which gpui also exposes as one).
//!
//! Builders in `HELPERS` apply these for their callers, so their own
//! definitions and chains rooted at a call to them are exempt. A chain that
//! is deliberately different can carry an `a11y-exempt: <reason>` comment,
//! which documents the exception where it lives instead of in a list here.

use std::fs;
use std::path::{Path, PathBuf};

const HELPERS: &[&str] = &[
    "ring_text_button",
    "text_button",
    "web_button",
    "activate",
    "sidebar_icon_button",
    "icon_button",
    "activity_row_button",
    "dropdown_trigger",
    "stepper_side",
    // gpui-component widgets carry their own accessibility.
    "Checkbox::new",
    "Switch::new",
];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn bracket_delta(line: &str) -> i32 {
    let mut delta = 0;
    let mut in_str = false;
    let mut prev = '\0';
    for c in line.chars() {
        if in_str {
            if c == '"' && prev != '\\' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => delta += 1,
                ')' | ']' | '}' => delta -= 1,
                _ => {}
            }
        }
        prev = c;
    }
    delta
}

/// A method chain: every line that continues it, including the bodies of
/// multi-line closures nested inside it. Returns `(line number, enclosing fn,
/// chain text)`, where the text also carries the few lines above the chain so
/// an `a11y-exempt:` comment can sit on the element it describes.
fn chains(src: &str) -> Vec<(usize, String, String)> {
    let lines: Vec<&str> = src.lines().collect();
    let mut depths = Vec::with_capacity(lines.len());
    let mut depth = 0;
    for line in &lines {
        depths.push(depth);
        depth += bracket_delta(line);
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with('.') {
            i += 1;
            continue;
        }
        let start = i;
        let base = depths[start];
        // The chain runs while we are nested deeper than where it started, or
        // the line is another `.method()` at the same level.
        while i < lines.len() && (depths[i] > base || lines[i].trim_start().starts_with('.')) {
            i += 1;
        }
        let enclosing = lines[..start]
            .iter()
            .rev()
            .find_map(|l| {
                let t = l.trim_start();
                t.strip_prefix("pub fn ")
                    .or_else(|| t.strip_prefix("fn "))
                    .or_else(|| t.strip_prefix("pub(super) fn "))
                    .or_else(|| t.strip_prefix("pub(crate) fn "))
                    .and_then(|rest| rest.split(['(', '<']).next())
                    .map(str::to_string)
            })
            .unwrap_or_default();
        // Comments introducing the element sit above its first `.method()`.
        let lead = start.saturating_sub(3);
        let preamble: Vec<&str> = lines[lead..start]
            .iter()
            .copied()
            .filter(|l| l.trim_start().starts_with("//"))
            .collect();
        let text = format!("{}\n{}", preamble.join("\n"), lines[start..i].join("\n"));
        out.push((start + 1, enclosing, text));
    }
    out
}

fn audit() -> Vec<String> {
    let mut files = Vec::new();
    rust_sources(Path::new("src"), &mut files);
    files.sort();
    let mut findings = Vec::new();
    for path in files {
        let full = fs::read_to_string(&path).expect("read source");
        // Test modules build fixtures, not UI.
        let src = full.split("\n#[cfg(test)]").next().unwrap_or(&full);
        for (line, enclosing, chain) in chains(src) {
            if HELPERS.iter().any(|h| enclosing == *h || chain.contains(h))
                || chain.contains("a11y-exempt:")
            {
                continue;
            }
            let where_ = format!("{}:{line}", path.display());
            if chain.contains(".tab_index(") && !chain.contains(".focus_visible(") {
                findings.push(format!("{where_}: focusable but no focus_visible"));
            }
            if chain.contains("Role::Button")
                && !chain.contains(".aria_label(")
                && !chain.contains(".tooltip(")
            {
                findings.push(format!("{where_}: Role::Button without an accessible name"));
            }
        }
    }
    findings
}

#[test]
fn every_focusable_control_shows_focus_and_announces_itself() {
    let findings = audit();
    assert!(
        findings.is_empty(),
        "accessibility audit found {} issue(s):\n{}",
        findings.len(),
        findings.join("\n")
    );
}

/// The audit must actually be looking at the UI, not an empty file list.
#[test]
fn audit_covers_the_view_sources() {
    let mut files = Vec::new();
    rust_sources(Path::new("src"), &mut files);
    assert!(
        files.len() > 15,
        "expected the desktop's sources, found {files:?}"
    );
    let chain_count: usize = files
        .iter()
        .map(|p| chains(&fs::read_to_string(p).expect("read")).len())
        .sum();
    assert!(
        chain_count > 500,
        "expected many element chains, got {chain_count}"
    );
}
