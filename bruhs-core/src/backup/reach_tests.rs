//! D21, ruling F30: no bruh tool, command, change or bridge request names
//! backup or restore. The helpers' tool lists are private functions, and
//! some names come from constants, so this reads their source: no string
//! literal in any of them holds `backup` or `restore` (which covers
//! `/api/backup`). Scanning the source keeps the test from widening them.

use bruhs_models::reactive::{ReactiveChange, ReactiveCommand};
use bruhs_models::workspace_bridge::BridgeRequest;

/// Every helper Bruhs starts, and the other tool surfaces: the MCP server
/// and its tool files.
const SOURCES: [(&str, &str); 14] = [
    ("workbench_tools.rs", include_str!("../workbench_tools.rs")),
    ("thread_link.rs", include_str!("../thread_link.rs")),
    (
        "thread_link/whiteboards.rs",
        include_str!("../thread_link/whiteboards.rs"),
    ),
    ("secrets/mcp.rs", include_str!("../secrets/mcp.rs")),
    ("reactive_tools.rs", include_str!("../reactive_tools.rs")),
    ("database_tools.rs", include_str!("../database_tools.rs")),
    (
        "api_client_tools.rs",
        include_str!("../api_client_tools.rs"),
    ),
    ("website_reader.rs", include_str!("../website_reader.rs")),
    ("artifact_tools.rs", include_str!("../artifact_tools.rs")),
    (
        "bruhs-mcp/src/tools.rs",
        include_str!("../../../bruhs-mcp/src/tools.rs"),
    ),
    (
        "bruhs-mcp/src/tools/audit.rs",
        include_str!("../../../bruhs-mcp/src/tools/audit.rs"),
    ),
    (
        "bruhs-mcp/src/tools/life_schema.rs",
        include_str!("../../../bruhs-mcp/src/tools/life_schema.rs"),
    ),
    (
        "bruhs-mcp/src/tools/workflow_schema.rs",
        include_str!("../../../bruhs-mcp/src/tools/workflow_schema.rs"),
    ),
    (
        "bruhs-mcp/src/tools/tests.rs",
        include_str!("../../../bruhs-mcp/src/tools/tests.rs"),
    ),
];

/// The words no helper's literal may hold, in any case.
const WORDS: [&str; 2] = ["backup", "restore"];

/// The string literals of Rust `source`: plain, raw and byte strings, with
/// comments and character literals skipped. Escapes stay as written.
fn literals(source: &str) -> Vec<String> {
    let text: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        match rest {
            ['/', '/', ..] => {
                while at < text.len() && text[at] != '\n' {
                    at += 1;
                }
            }
            ['/', '*', ..] => {
                let mut depth = 0_usize;
                while at < text.len() {
                    if text[at..].starts_with(&['/', '*']) {
                        depth += 1;
                        at += 2;
                    } else if text[at..].starts_with(&['*', '/']) {
                        depth -= 1;
                        at += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        at += 1;
                    }
                }
            }
            ['\'', '\\', ..] => {
                // An escaped character: up to its closing quote.
                at += 2;
                while at < text.len() && text[at] != '\'' {
                    at += 1;
                }
                at += 1;
            }
            ['\'', _, '\'', ..] => at += 3,
            ['"', ..] => {
                let mut literal = String::new();
                at += 1;
                while at < text.len() && text[at] != '"' {
                    if text[at] == '\\' && at + 1 < text.len() {
                        literal.push(text[at]);
                        at += 1;
                    }
                    literal.push(text[at]);
                    at += 1;
                }
                at += 1;
                found.push(literal);
            }
            ['r', ..] | ['b', 'r', ..] if raw_start(rest).is_some() => {
                let (skip, hashes) = raw_start(rest).unwrap_or_default();
                at += skip;
                let end: Vec<char> = std::iter::once('"')
                    .chain(std::iter::repeat_n('#', hashes))
                    .collect();
                let mut literal = String::new();
                while at < text.len() && !text[at..].starts_with(&end) {
                    literal.push(text[at]);
                    at += 1;
                }
                at += end.len();
                found.push(literal);
            }
            [first, ..] if first.is_alphanumeric() || *first == '_' => {
                // A whole identifier, so `br"` or `r#"` inside one never
                // starts a literal.
                at += 1;
                while at < text.len() && (text[at].is_alphanumeric() || text[at] == '_') {
                    if matches!(text[at], 'r' | 'b') && raw_start(&text[at..]).is_some() {
                        break;
                    }
                    at += 1;
                }
            }
            _ => at += 1,
        }
    }
    found
}

/// For a raw string opening at the start of `text` (`r"`, `r#"`, `br#"`),
/// the characters up to its body and the number of `#`.
fn raw_start(text: &[char]) -> Option<(usize, usize)> {
    let mut at = usize::from(text.first() == Some(&'b'));
    if text.get(at) != Some(&'r') {
        return None;
    }
    at += 1;
    let hashes = text[at..].iter().take_while(|c| **c == '#').count();
    at += hashes;
    (text.get(at) == Some(&'"')).then_some((at + 1, hashes))
}

#[test]
fn the_scanner_reads_literals_and_skips_comments() {
    let source = r##"
        // restores its "parent" in a comment
        /* a "backup" here /* nested */ too */
        let quote = '"';
        let escaped = '\'';
        fn life<'a>(x: &'a str) {}
        let plain = "one \"two\"";
        let raw = r#"three "four""#;
        let bytes = br"five";
        let name = number;
    "##;
    assert_eq!(
        literals(source),
        ["one \\\"two\\\"", "three \"four\"", "five"]
    );
}

#[test]
fn no_bruh_tool_or_command_reaches_backup() {
    for (file, source) in SOURCES {
        assert!(!literals(source).is_empty(), "{file} reads as no source");
        for literal in literals(source) {
            let lower = literal.to_lowercase();
            for word in WORDS {
                assert!(
                    !lower.contains(word),
                    "{file} has a literal naming {word}: {literal:?}"
                );
            }
        }
    }

    for word in WORDS {
        let command = serde_json::json!({ "action": word });
        assert!(serde_json::from_value::<ReactiveCommand>(command).is_err());
        let change = serde_json::json!({ "type": word });
        assert!(serde_json::from_value::<ReactiveChange>(change.clone()).is_err());
        let applied = serde_json::json!({ "action": "apply", "change": change });
        assert!(serde_json::from_value::<ReactiveCommand>(applied).is_err());
        for tag in ["kind", "op"] {
            let request = serde_json::json!({ tag: word });
            assert!(serde_json::from_value::<BridgeRequest>(request).is_err());
        }
    }
    // A request of the right shape does parse, so the checks above refuse
    // the word and not the shape.
    assert!(
        serde_json::from_value::<ReactiveCommand>(serde_json::json!({
            "action": "stop_all"
        }))
        .is_ok()
    );
}
