//! ChatGPT conversations along the branch the owner saw (Task 4), and the
//! text rules every export reader shares. Every fixture is synthetic.

use super::chatgpt::{conversation, is_conversation, is_manifest};
use super::test_support::fixture;
use super::text::{
    clip_part, export_time, strip_citations, strip_grok_markup, strip_writing_directives,
};
use super::ParsedConversation;
use bruhs_models::agent::Role;
use bruhs_models::imports::{attachment_line, cut_short_line, IMAGE_NOT_IMPORTED, MAX_MESSAGES};
use bruhs_models::logic::portable::{ImportSource, PortablePart};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::{Duration, Instant};

/// The six fixture conversations, A to F.
fn fixtures() -> Vec<Value> {
    let all: Value =
        serde_json::from_slice(fixture("chatgpt-conversations.json")).expect("the fixture is JSON");
    let Value::Array(all) = all else {
        panic!("the fixture is an array");
    };
    assert_eq!(all.len(), 6);
    all
}

fn parsed(index: usize) -> ParsedConversation {
    conversation(&fixtures()[index]).expect("the fixture conversation imports")
}

/// Each message's role and its text parts joined.
fn texts(parsed: &ParsedConversation) -> Vec<(Role, String)> {
    parsed
        .messages
        .iter()
        .map(|message| (message.role, message.text()))
        .collect()
}

/// A visible message node.
fn node(parent: Option<&str>, role: &str, text: &str) -> Value {
    json!({
        "parent": parent,
        "message": {
            "author": { "role": role },
            "content": { "content_type": "text", "parts": [text] },
        },
    })
}

#[test]
fn the_current_branch_imports_and_plumbing_stays_out() {
    let a = parsed(0);
    assert_eq!(a.messages.len(), 2, "{:?}", a.messages);
    assert_eq!(a.messages[0].role, Role::User);
    assert_eq!(
        a.messages[0].parts,
        vec![PortablePart::Text(
            "Plan three days in Lisbon on a budget.".into()
        )]
    );
    assert_eq!(a.messages[0].started_at_ms, Some(1_763_000_100_000));
    let reply = &a.messages[1];
    assert_eq!(reply.role, Role::Assistant);
    assert_eq!(
        reply.parts,
        vec![
            PortablePart::Thinking("Budget picks\n\nTrams are cheap.".into()),
            PortablePart::Text(
                "Day one: free viewpoints in Alfama. Day two: ride tram 28.\n\
                 Subject: Lisbon plan\nSee you at the castle.\nEnjoy the trip."
                    .into()
            ),
        ]
    );
    assert_eq!(reply.model.as_deref(), Some("gpt-5"));
    assert_eq!(a.dropped_branches, 1);

    let candidate = &a.candidate;
    assert_eq!(candidate.source, ImportSource::ChatGpt);
    assert_eq!(candidate.id, "conv-lisbon-a");
    assert_eq!(candidate.title, "Lisbon on a budget");
    assert_eq!(candidate.updated_at.as_millis(), 1_763_000_300_500);
    assert_eq!(candidate.session_id, None);
    assert_eq!(candidate.provider, None);
    assert_eq!(candidate.cwd, None);
    assert_eq!(candidate.model, None);
}

#[test]
fn children_are_rebuilt_from_parents_when_missing() {
    let b = parsed(1);
    assert_eq!(
        texts(&b),
        vec![
            (Role::User, "How long should sourdough proof?".into()),
            (
                Role::Assistant,
                "Four to six hours at room temperature.".into()
            ),
            (Role::User, "And overnight in the fridge?".into()),
            (
                Role::Assistant,
                "Twelve to sixteen hours works well.".into()
            ),
        ]
    );
    assert_eq!(b.messages[0].started_at_ms, Some(1_763_100_050_250));
    assert_eq!(b.dropped_branches, 0);
    assert_eq!(b.candidate.updated_at.as_millis(), 1_763_100_400_750);

    // With no title, the first prompt names it; with no update time, the
    // newest message dates it.
    let mut untitled = fixtures().swap_remove(1);
    let record = untitled.as_object_mut().expect("a conversation object");
    record.insert("title".into(), Value::Null);
    record.remove("update_time");
    record.remove("create_time");
    let untitled = conversation(&untitled).expect("an untitled conversation imports");
    assert_eq!(untitled.candidate.title, "How long should sourdough proof?");
    assert_eq!(untitled.candidate.updated_at.as_millis(), 1_763_100_300_500);
}

#[test]
fn a_dangling_current_node_falls_back_to_the_newest_leaf() {
    let c = parsed(2);
    assert_eq!(
        texts(&c),
        vec![
            (Role::User, "Which herbs grow on a balcony?".into()),
            (Role::Assistant, "Rosemary and thyme.".into()),
        ]
    );
    assert_eq!(c.dropped_branches, 1);
}

#[test]
fn multimodal_parts_and_attachments_leave_placeholders() {
    let d = parsed(3);
    let all = texts(&d);
    assert_eq!(all.len(), 3, "{all:?}");
    let (role, first) = &all[0];
    assert_eq!(*role, Role::User);
    assert!(first.contains(IMAGE_NOT_IMPORTED), "{first}");
    assert!(first.contains("Here is the kitchen floor plan."), "{first}");
    assert!(first.contains(&attachment_line("plan.pdf")), "{first}");
    let (role, voice) = &all[2];
    assert_eq!(*role, Role::User);
    assert!(
        voice.contains("Can we move the sink under the window?"),
        "{voice}"
    );
}

#[test]
fn empty_conversations_are_skipped() {
    let all = fixtures();
    assert!(conversation(&all[4]).is_none(), "only a system message");
    assert!(conversation(&all[5]).is_none(), "an empty object");
    assert!(!is_conversation(&all[5]));
    assert!(is_conversation(&all[4]));

    let deepseek = json!({
        "id": "ds-1",
        "title": "A DeepSeek chat",
        "inserted_at": "2026-01-01T00:00:00Z",
        "mapping": {
            "root": { "id": "root", "parent": null, "children": ["1"], "message": null },
            "1": {
                "id": "1",
                "parent": "root",
                "children": [],
                "message": { "fragments": [{ "type": "REQUEST", "content": "Hello" }] },
            },
        },
    });
    assert!(!is_conversation(&deepseek));
    assert!(conversation(&deepseek).is_none());

    let manifest: Value = serde_json::from_slice(fixture("chatgpt-export_manifest.json"))
        .expect("the manifest is JSON");
    assert!(is_manifest(&manifest));
    assert!(is_manifest(
        &json!({ "export_files": [{ "path": "a.json", "size_bytes": 2 }] })
    ));
    assert!(!is_manifest(&all[0]));
    assert!(!is_conversation(&manifest));
}

#[test]
fn cycles_and_huge_trees_stay_bounded() {
    let cycle = json!({
        "conversation_id": "conv-cycle",
        "current_node": "a",
        "mapping": {
            "a": node(Some("b"), "user", "Round and round."),
            "b": node(Some("a"), "assistant", "And back again."),
            "s": node(Some("s"), "user", "My own parent."),
        },
    });
    let started = Instant::now();
    let parsed = conversation(&cycle);
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(parsed.is_none_or(|parsed| parsed.messages.len() <= 2));

    let alone = json!({
        "conversation_id": "conv-self",
        "mapping": { "s": node(Some("s"), "user", "My own parent.") },
    });
    let started = Instant::now();
    let parsed = conversation(&alone);
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(parsed.is_none_or(|parsed| parsed.messages.len() <= 1));

    let total = 100_001;
    let mut mapping = Map::new();
    for i in 0..total {
        let parent = (i > 0).then(|| format!("n{:06}", i - 1));
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        mapping.insert(
            format!("n{i:06}"),
            node(parent.as_deref(), role, &format!("message {i}")),
        );
    }
    let chain = json!({
        "conversation_id": "conv-chain",
        "current_node": format!("n{:06}", total - 1),
        "mapping": Value::Object(mapping),
    });
    let started = Instant::now();
    let parsed = conversation(&chain).expect("a long chain imports");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(parsed.messages.len(), MAX_MESSAGES);
    // The walk stops at MAX_NODES nodes, so n099999 is the newest kept,
    // and the oldest messages are the ones dropped.
    let last = parsed.messages.last().map(|message| message.text());
    assert_eq!(last.as_deref(), Some("message 99999"));
    assert_eq!(parsed.messages[0].text(), "message 80000");
}

/// Empty prompts make fewer messages than prompts, so the newest
/// `MAX_MESSAGES` prompts aren't always enough to read.
#[test]
fn a_long_branch_of_empty_prompts_still_keeps_its_messages() {
    let pairs = 100 + MAX_MESSAGES;
    let mut mapping = Map::new();
    let mut parent: Option<String> = None;
    for i in 0..pairs {
        let prompt = if i < 100 {
            format!("prompt {i}")
        } else {
            String::new()
        };
        for (role, text) in [("user", prompt), ("assistant", format!("reply {i}"))] {
            let id = format!("n{:06}", mapping.len());
            mapping.insert(id.clone(), node(parent.as_deref(), role, &text));
            parent = Some(id);
        }
    }
    let chain = json!({
        "conversation_id": "conv-empty-prompts",
        "current_node": parent,
        "mapping": Value::Object(mapping),
    });
    let parsed = conversation(&chain).expect("the chain imports");
    // 100 prompts and 100 replies; every reply after the last prompt joins
    // the 100th.
    assert_eq!(parsed.messages.len(), 200);
    assert_eq!(parsed.messages[0].text(), "prompt 0");
}

#[test]
fn timestamps_take_every_form() {
    assert_eq!(
        export_time(&json!(1_700_000_000.5)),
        Some(1_700_000_000_500)
    );
    assert_eq!(
        export_time(&json!(1_700_000_000_500_i64)),
        Some(1_700_000_000_500)
    );
    assert_eq!(
        export_time(&json!("2026-11-08T01:40:28.550Z")),
        Some(1_794_102_028_550)
    );
    assert_eq!(export_time(&json!("1700000000")), Some(1_700_000_000_000));
    assert_eq!(
        export_time(&json!({ "$date": { "$numberLong": "1700000000500" } })),
        Some(1_700_000_000_500)
    );
    assert_eq!(
        export_time(&json!({ "$date": "2026-11-08T01:40:28.550Z" })),
        Some(1_794_102_028_550)
    );
    assert_eq!(export_time(&Value::Null), None);
    assert_eq!(export_time(&json!("not a time")), None);
    assert_eq!(export_time(&json!(-5)), None);
}

#[test]
fn markup_is_stripped_and_long_parts_are_cut() {
    assert_eq!(
        strip_citations("Hills.\u{e200}cite\u{e202}turn0search1\u{e201} Trams.【3†source】"),
        "Hills. Trams."
    );
    assert_eq!(
        strip_citations("A lone \u{e200} marker stays text."),
        "A lone  marker stays text."
    );
    assert_eq!(
        strip_writing_directives(
            "Here:\n:::writing{variant=\"email\" id=\"1\"}\nHi there.\n:::\nDone."
        ),
        "Here:\nHi there.\nDone."
    );
    assert_eq!(
        strip_writing_directives("A ::: in a line stays.\n:::\nSo does a lone closer."),
        "A ::: in a line stays.\n:::\nSo does a lone closer."
    );
    assert_eq!(
        strip_grok_markup(
            "Fact<grok:render type=\"cite\"><argument>1</argument></grok:render>. \
             <xaiArtifact title=\"Plan\">Step one.</xaiArtifact>"
        ),
        "Fact. Step one."
    );

    assert_eq!(clip_part("short".into(), 10), "short");
    let cut = clip_part("ééééé".into(), 3);
    assert_eq!(cut, format!("ééé\n{}", cut_short_line(2)));
}

/// Fixtures stand in for real exports, so none may hold a real path, an
/// address or a key.
#[test]
fn fixtures_are_synthetic() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/import/fixtures");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("the fixtures folder reads") {
        let path = entry.expect("a fixture entry").path();
        let text = std::fs::read_to_string(&path).expect("a fixture is text");
        let name = path.display();
        seen += 1;
        assert!(!text.contains("/Users/"), "{name} holds a home path");
        assert!(!text.contains("/home/"), "{name} holds a home path");
        for (at, _) in text.match_indices('@') {
            assert!(
                text[at..].starts_with("@example.com"),
                "{name} holds an address outside example.com"
            );
        }
        assert!(!has_key(&text, "sk-", |c| c.is_ascii_alphanumeric()
            || c == '_'
            || c == '-'));
        assert!(!has_key(&text, "xai-", |c| c.is_ascii_alphanumeric()));
        for (at, _) in text.match_indices("\"email_address\"") {
            let rest = text[at + "\"email_address\"".len()..]
                .trim_start()
                .trim_start_matches(':')
                .trim_start();
            if rest.starts_with("null") {
                continue;
            }
            let value = rest
                .strip_prefix('"')
                .and_then(|rest| rest.split('"').next())
                .unwrap_or(rest);
            assert!(
                value.ends_with("@example.com"),
                "{name} holds an email_address outside example.com"
            );
        }
    }
    assert!(seen >= 2, "the fixtures were found");
}

/// Whether `text` holds `prefix` followed by 16 or more key characters.
fn has_key(text: &str, prefix: &str, key_char: impl Fn(char) -> bool) -> bool {
    text.match_indices(prefix).any(|(at, _)| {
        text[at + prefix.len()..]
            .chars()
            .take_while(|&c| key_char(c))
            .count()
            >= 16
    })
}

#[test]
fn the_key_guard_matches_shapes_not_words() {
    assert!(!has_key("a task-list on the desk-side", "sk-", |c| c
        .is_ascii_alphanumeric()));
    assert!(has_key("key sk-abcdefghijklmnop0", "sk-", |c| c
        .is_ascii_alphanumeric()));
}
