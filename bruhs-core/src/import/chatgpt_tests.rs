//! ChatGPT conversations along the branch the owner saw (Task 4), and the
//! text rules every export reader shares. Every fixture is synthetic.

use super::chatgpt::{conversation, is_conversation, is_manifest};
use super::test_support::{assert_scales_linearly, fixture};
use super::text::{
    clip_part, export_time, strip_citations, strip_grok_markup, strip_writing_directives,
};
use super::ParsedConversation;
use bruhs_models::agent::Role;
use bruhs_models::imports::{
    attachment_line, cut_short_line, AUDIO_NOT_IMPORTED, IMAGE_NOT_IMPORTED, MAX_MESSAGES,
};
use bruhs_models::logic::portable::{ImportSource, PortablePart};
use serde_json::{json, Map, Value};
use std::path::Path;

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
    // Two clips, one line; the transcription doesn't replace it.
    assert_eq!(voice.matches(AUDIO_NOT_IMPORTED).count(), 1, "{voice}");
    assert!(all.iter().all(|(_, text)| !text.contains("sediment://")));
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

/// A mapping of `n` nodes in one parent ring, then `n` nodes that are
/// their own parent.
fn looped_mapping(n: usize) -> Value {
    let mut mapping = Map::new();
    for i in 0..n {
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        let parent = format!("r{}", (i + 1) % n);
        mapping.insert(
            format!("r{i}"),
            node(Some(&parent), role, &format!("ring {i}")),
        );
        let own = format!("s{i}");
        mapping.insert(own.clone(), node(Some(&own), "user", "My own parent."));
    }
    json!({ "conversation_id": "conv-loops", "current_node": "r0", "mapping": mapping })
}

/// A chain of `total` nodes, each the parent of the next.
fn chain_of(total: usize) -> Value {
    let mut mapping = Map::new();
    for i in 0..total {
        let parent = (i > 0).then(|| format!("n{:06}", i - 1));
        let role = if i % 2 == 0 { "user" } else { "assistant" };
        mapping.insert(
            format!("n{i:06}"),
            node(parent.as_deref(), role, &format!("message {i}")),
        );
    }
    json!({
        "conversation_id": "conv-chain",
        "current_node": format!("n{:06}", total - 1),
        "mapping": Value::Object(mapping),
    })
}

#[test]
fn cycles_and_huge_trees_stay_bounded() {
    // Cycles and long chains take time in step with their size, and a
    // loop that never ends fails the guard.
    assert_scales_linearly("ChatGPT parent cycles", 5_000, looped_mapping, |v| {
        let _ = conversation(v);
    });
    assert_scales_linearly("a ChatGPT chain", 25_000, chain_of, |v| {
        let _ = conversation(v);
    });

    let cycle = json!({
        "conversation_id": "conv-cycle",
        "current_node": "a",
        "mapping": {
            "a": node(Some("b"), "user", "Round and round."),
            "b": node(Some("a"), "assistant", "And back again."),
            "s": node(Some("s"), "user", "My own parent."),
        },
    });
    let parsed = conversation(&cycle);
    assert!(parsed.is_none_or(|parsed| parsed.messages.len() <= 2));

    let alone = json!({
        "conversation_id": "conv-self",
        "mapping": { "s": node(Some("s"), "user", "My own parent.") },
    });
    let parsed = conversation(&alone);
    assert!(parsed.is_none_or(|parsed| parsed.messages.len() <= 1));

    let parsed = conversation(&chain_of(100_001)).expect("a long chain imports");
    assert_eq!(parsed.messages.len(), MAX_MESSAGES);
    // The walk stops at MAX_NODES nodes, so n099999 is the newest kept,
    // and the oldest messages are the ones dropped.
    let last = parsed.messages.last().map(|message| message.text());
    assert_eq!(last.as_deref(), Some("message 99999"));
    assert_eq!(parsed.messages[0].text(), "message 80000");
    // Untitled: the branch's first prompt names it, though it wasn't kept.
    assert_eq!(parsed.candidate.title, "message 0");
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

#[test]
fn markup_edges_keep_the_text_around_them() {
    // M3: a lone U+E200 before a real span deletes nothing.
    assert_eq!(
        strip_citations("A \u{e200} lone. Then \u{e200}cite\u{e202}x\u{e201} end"),
        "A  lone. Then  end"
    );
    assert_eq!(strip_citations("Kept 【a】 and 【b"), "Kept 【a】 and 【b");
    // M4: text after the opener's `}` on its line stays.
    assert_eq!(
        strip_writing_directives(":::writing{variant=\"email\" id=\"1\"} Hi there\nbody\n:::"),
        "Hi there\nbody"
    );
    assert_eq!(
        strip_writing_directives(":::writing{title=\"a } b\"}\nbody\n:::"),
        "body"
    );
    // I1: a `<grok:render` that never ends keeps what follows it.
    assert_eq!(
        strip_grok_markup("Before <grok:render type=\"cite\" and after"),
        "Before <grok:render type=\"cite\" and after"
    );
    assert_eq!(strip_grok_markup("A<grok:render/>B"), "AB");
    assert_eq!(
        strip_grok_markup("A<grok:render>x</grok:render>B<grok:render>y C"),
        "ABy C"
    );
    // I1: a longer tag name hides nothing after it.
    assert_eq!(
        strip_grok_markup(
            "<xaiArtifactX>keep</xaiArtifactX> <xaiArtifact id=\"1\">in</xaiArtifact>"
        ),
        "<xaiArtifactX>keep</xaiArtifactX> in"
    );
    assert_eq!(
        strip_grok_markup("a <xaiArtifact never closes"),
        "a <xaiArtifact never closes"
    );
}

/// `unit` repeated to about `bytes` bytes, then `tail`.
fn repeated(unit: &str, tail: &str, bytes: usize) -> String {
    let mut text = unit.repeat(bytes / unit.len());
    text.push_str(tail);
    text
}

/// Each input that made a stripper search the rest of the text again for
/// every tag, at 256 KiB and 1 MiB: the time grows with the size.
#[test]
fn hostile_markup_strips_in_linear_time() {
    const SIZE: usize = 256 << 10;
    let grok = [
        ("<grok:render>", ""),
        ("<grok:render", ""),
        ("<grok:render>x", "</grok:render>"),
        ("<grok:render/>", ""),
        ("</grok:render>", ""),
        ("</xaiArtifact>", ""),
        ("<xaiArtifact ", ""),
        ("<xaiArtifact>", ""),
        ("<xaiArtifactX>", "<xaiArtifact>"),
        ("<", ""),
    ];
    for (unit, tail) in grok {
        assert_scales_linearly(
            &format!("strip_grok_markup on {unit:?}"),
            SIZE,
            |bytes| repeated(unit, tail, bytes),
            |text| {
                let _ = strip_grok_markup(text);
            },
        );
    }
    let citations = [
        ("\u{e200}", "\u{e201}"),
        ("\u{e201}", ""),
        ("【", "】"),
        ("【†", "】"),
        ("】", ""),
        (":::writing{\n", ":::"),
    ];
    for (unit, tail) in citations {
        assert_scales_linearly(
            &format!("the citation strippers on {unit:?}"),
            SIZE,
            |bytes| repeated(unit, tail, bytes),
            |text| {
                let _ = strip_writing_directives(&strip_citations(text));
            },
        );
    }
}

#[test]
fn an_empty_conversation_id_falls_back_to_id() {
    let record = json!({
        "conversation_id": "",
        "id": "conv-from-id",
        "mapping": { "u": node(None, "user", "Hello there.") },
    });
    let parsed = conversation(&record).expect("the record imports");
    assert_eq!(parsed.candidate.id, "conv-from-id");
}

#[test]
fn messages_hidden_by_recipient_or_tool_role_alone_stay_out() {
    let mut memory = node(Some("u"), "assistant", "Remember: likes trams.");
    memory["message"]["recipient"] = json!("bio");
    let tool = node(Some("m"), "tool", "Model set context updated.");
    let record = json!({
        "conversation_id": "conv-bio",
        "current_node": "a",
        "mapping": {
            "u": node(None, "user", "I like trams."),
            "m": memory,
            "t": tool,
            "a": node(Some("t"), "assistant", "Noted."),
        },
    });
    let parsed = conversation(&record).expect("the record imports");
    assert_eq!(
        texts(&parsed),
        vec![
            (Role::User, "I like trams.".into()),
            (Role::Assistant, "Noted.".into()),
        ]
    );
}

#[test]
fn wrong_typed_values_are_skipped_without_panicking() {
    let mut huge = node(Some("n"), "user", "");
    huge["message"]["content"]["parts"] = Value::Array(vec![json!("x"); 100_000]);
    let record = json!({
        "conversation_id": "conv-odd",
        "current_node": "huge",
        "title": 7,
        "update_time": "soon",
        "mapping": {
            "root": null,
            "n": 5,
            "s": { "parent": "root", "message": { "author": { "role": "user" }, "content": { "content_type": "text", "parts": "a string" } } },
            "a": { "parent": "s", "message": { "author": 3, "content": { "parts": ["x"] } } },
            "b": { "parent": "a", "message": { "author": { "role": "user" }, "content": { "parts": ["Hi"] }, "metadata": { "attachments": { "name": "x.pdf" } } } },
            "c": { "parent": ["b"], "message": { "author": { "role": "assistant" }, "content": 9 } },
            "d": { "parent": "b", "message": { "author": { "role": "assistant" }, "create_time": [1], "content": { "content_type": "text", "parts": [null, 4, { "text": 5 }, "Hello."] } } },
            "huge": huge,
        },
    });
    assert!(conversation(&record).is_some_and(|parsed| parsed.messages.len() <= 1));
    let parsed = conversation(&json!({
        "conversation_id": "conv-odd-2",
        "current_node": "d",
        "mapping": record["mapping"].clone(),
    }))
    .expect("the visible part imports");
    assert_eq!(
        texts(&parsed),
        vec![
            (Role::User, "Hi".into()),
            (Role::Assistant, "Hello.".into()),
        ]
    );
    for odd in [
        json!(null),
        json!(3),
        json!("x"),
        json!([]),
        json!({ "mapping": [] }),
    ] {
        assert!(conversation(&odd).is_none());
    }
}

#[test]
fn a_missing_parent_mid_branch_ends_the_walk() {
    let record = json!({
        "conversation_id": "conv-gap",
        "current_node": "a2",
        "mapping": {
            "root": { "parent": null },
            "u1": node(Some("root"), "user", "First."),
            "a1": node(Some("u1"), "assistant", "One."),
            "u2": node(Some("gone"), "user", "Second."),
            "a2": node(Some("u2"), "assistant", "Two."),
        },
    });
    let parsed = conversation(&record).expect("the record imports");
    assert_eq!(
        texts(&parsed),
        vec![
            (Role::User, "Second.".into()),
            (Role::Assistant, "Two.".into())
        ]
    );
    // The part above the gap is a branch the walk didn't reach.
    assert_eq!(parsed.dropped_branches, 1);
}

#[test]
fn an_interior_current_node_drops_only_the_other_branches() {
    let record = json!({
        "conversation_id": "conv-interior",
        "current_node": "a1",
        "mapping": {
            "u1": node(None, "user", "Pick a bread."),
            "a1": node(Some("u1"), "assistant", "Sourdough."),
            "u2": node(Some("a1"), "user", "Why?"),
            "u3": node(Some("a1"), "user", "And rye?"),
        },
    });
    let parsed = conversation(&record).expect("the record imports");
    assert_eq!(parsed.messages.len(), 2);
    assert_eq!(parsed.dropped_branches, 1);
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
        seen += 1;
        assert_eq!(not_synthetic(&text), None, "{}", path.display());
    }
    assert!(seen >= 2, "the fixtures were found");
}

/// Why `text` can't be a synthetic fixture: a home path, an address or
/// `email_address` outside example.com, or a key-shaped value (F32).
fn not_synthetic(text: &str) -> Option<&'static str> {
    if text.contains("/Users/") || text.contains("/home/") {
        return Some("a home path");
    }
    if text
        .match_indices('@')
        .any(|(at, _)| !is_example_domain(&text[at + 1..]))
    {
        return Some("an address outside example.com");
    }
    if has_key(text, "sk-", |c| {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    }) || has_key(text, "xai-", |c| c.is_ascii_alphanumeric())
    {
        return Some("a key");
    }
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
        if !value
            .rsplit_once('@')
            .is_some_and(|(_, domain)| is_example_domain(domain))
        {
            return Some("an email_address outside example.com");
        }
    }
    None
}

/// Whether `after` (what follows an `@`) starts with the domain
/// example.com and nothing more of a domain.
fn is_example_domain(after: &str) -> bool {
    after.strip_prefix("example.com").is_some_and(|rest| {
        let mut rest = rest.chars();
        match rest.next() {
            None => true,
            Some('.') => !rest.next().is_some_and(|c| c.is_ascii_alphanumeric()),
            Some(c) => !(c.is_ascii_alphanumeric() || c == '-'),
        }
    })
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
fn the_synthetic_guard_matches_shapes_not_words() {
    assert_eq!(not_synthetic("a task-list on the desk-side"), None);
    assert_eq!(not_synthetic("key sk-proj_abcdefghijklmnop"), Some("a key"));
    assert_eq!(not_synthetic("xai-abcdefghijklmnop0"), Some("a key"));
    assert_eq!(not_synthetic("xai-short and taxi-rank"), None);
    assert_eq!(not_synthetic("pat@example.com."), None);
    assert_eq!(not_synthetic("\"pat@example.com\""), None);
    assert_eq!(
        not_synthetic("pat@example.com.other.org"),
        Some("an address outside example.com")
    );
    assert_eq!(
        not_synthetic("pat@example.community"),
        Some("an address outside example.com")
    );
    assert_eq!(
        not_synthetic("pat@elsewhere.org"),
        Some("an address outside example.com")
    );
    assert_eq!(
        not_synthetic("\"email_address\": \"pat at elsewhere\""),
        Some("an email_address outside example.com")
    );
    assert_eq!(not_synthetic("\"email_address\": null"), None);
    assert_eq!(not_synthetic("/home/pat"), Some("a home path"));
}
