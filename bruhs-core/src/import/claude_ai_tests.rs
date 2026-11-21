//! Claude.ai conversations along the newest branch, and Claude's memory
//! in both export shapes (Task 5). Every fixture is synthetic.

use super::claude_ai::{conversation, is_conversation, is_manifest, is_memory, memory};
use super::test_support::fixture;
use super::ParsedConversation;
use bruhs_models::agent::Role;
use bruhs_models::imports::{
    attached_line, attachment_line, cut_short_line, IMAGE_NOT_IMPORTED, MAX_ATTACHMENT_TEXT_CHARS,
    MAX_MESSAGES, PROJECT_MEMORY,
};
use bruhs_models::logic::portable::{ImportSource, PortablePart, PortableToolCall};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::{Duration, Instant};

const ROOT: &str = "00000000-0000-4000-8000-000000000000";

/// The four fixture conversations, G to J.
fn fixtures() -> Vec<Value> {
    let all: Value =
        serde_json::from_slice(fixture("claude-conversations.json")).expect("the fixture is JSON");
    let Value::Array(all) = all else {
        panic!("the fixture is an array");
    };
    assert_eq!(all.len(), 4);
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

fn json_fixture(name: &str) -> Value {
    serde_json::from_slice(fixture(name)).expect("the fixture is JSON")
}

/// A one-message conversation holding `message`.
fn holding(message: Value) -> Value {
    json!({ "uuid": "conv-one", "name": "One", "chat_messages": [message] })
}

#[test]
fn the_newest_branch_imports_in_tree_order() {
    let g = parsed(0);
    assert_eq!(g.candidate.source, ImportSource::ClaudeAi);
    assert_eq!(g.candidate.id, "conv-sourdough-g");
    assert_eq!(g.candidate.title, "Sourdough for the weekend");
    assert_eq!(g.candidate.cwd, None);
    assert_eq!(g.candidate.provider, None);
    assert_eq!(g.candidate.session_id, None);
    assert_eq!(g.candidate.model, None);
    // updated_at 09:20:30 is later than the newest message (09:20).
    assert_eq!(g.candidate.updated_at.as_millis(), 1_777_627_230_000);
    assert_eq!(g.dropped_branches, 1);

    let roles: Vec<Role> = g.messages.iter().map(|message| message.role).collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::User, Role::Assistant]
    );
    // Root to the newest leaf, not the array's order, and not the older
    // Friday branch.
    assert!(g.messages[0]
        .text()
        .starts_with("Plan a sourdough bake for Saturday."));
    assert_eq!(g.messages[2].text(), "Make it Sunday instead.");
    assert_eq!(
        g.messages[3].text(),
        "Sunday works: feed the starter on Saturday night."
    );
    assert_eq!(g.messages[0].started_at_ms, Some(1_777_626_000_000));
    let all = texts(&g)
        .into_iter()
        .map(|(_, text)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!all.contains("Friday"), "{all}");

    let reply = &g.messages[1];
    assert_eq!(reply.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(
        reply.parts[0],
        PortablePart::Thinking("Work back from the bake time.".into())
    );
    assert_eq!(
        reply.parts[1],
        PortablePart::Text("Here is a timeline that ends at noon.".into())
    );
    assert_eq!(reply.parts.len(), 3, "{:?}", reply.parts);
    // The search pair and the injected prompt are plumbing.
    let debug = format!("{:?}", g.messages);
    assert!(!debug.contains("web_search"), "{debug}");
    assert!(!debug.contains("Bulk ferment"), "{debug}");
    assert!(!debug.contains("metric units"), "{debug}");
}

#[test]
fn artifacts_come_in_as_tool_calls() {
    let g = parsed(0);
    let input = fixtures()[0]["chat_messages"][3]["content"][4]["input"].clone();
    assert_eq!(input["command"], "create");
    assert_eq!(
        g.messages[1].parts[2],
        PortablePart::ToolCall(PortableToolCall {
            id: "toolu-artifact-01".into(),
            tool: "artifacts".into(),
            input,
            output: Some("OK".into()),
            is_error: false,
        })
    );

    // Each kept tool name comes in; other tools stay out. A call with no id
    // still comes in, and a result naming no call finds the last one.
    let kept = ["artifacts", "create_file", "str_replace", "Write", "Edit"];
    for name in kept.iter().chain(["web_fetch", "repl"].iter()) {
        let parsed = conversation(&holding(json!({
            "sender": "assistant",
            "content": [
                { "type": "text", "text": "Done." },
                { "type": "tool_use", "name": name, "input": { "path": "plan.md" } },
                { "type": "tool_result", "name": name, "content": "Saved." },
            ],
        })))
        .expect("the reply imports");
        let calls: Vec<&PortableToolCall> = parsed.messages[0]
            .parts
            .iter()
            .filter_map(|part| match part {
                PortablePart::ToolCall(call) => Some(call),
                PortablePart::Text(_) | PortablePart::Thinking(_) => None,
            })
            .collect();
        if kept.contains(name) {
            assert_eq!(calls.len(), 1, "{name}");
            assert_eq!(calls[0].tool, *name);
            assert_eq!(calls[0].input, json!({ "path": "plan.md" }));
            assert_eq!(calls[0].output.as_deref(), Some("Saved."));
            assert!(!calls[0].id.is_empty());
        } else {
            assert!(calls.is_empty(), "{name}");
        }
    }
}

#[test]
fn extracted_attachment_text_comes_along() {
    let g = parsed(0);
    assert_eq!(
        g.messages[0].text(),
        format!(
            "Plan a sourdough bake for Saturday.\n\n{}\nStarter fed at 8.\nFlour: 500 g.\n\n{}",
            attached_line("notes.txt"),
            attachment_line("oven.jpg"),
        )
    );

    // Extracted text is cut to MAX_ATTACHMENT_TEXT_CHARS.
    let long = "é".repeat(MAX_ATTACHMENT_TEXT_CHARS + 25);
    let parsed = conversation(&holding(json!({
        "sender": "human",
        "text": "Read this.",
        "attachments": [{ "file_name": "long.txt", "extracted_content": long }],
        "files": [{ "file_name": "long.txt" }, { "file_name": "photo.png" }],
    })))
    .expect("the prompt imports");
    let expected = format!(
        "Read this.\n\n{}\n{}\n{}\n\n{}",
        attached_line("long.txt"),
        "é".repeat(MAX_ATTACHMENT_TEXT_CHARS),
        cut_short_line(25),
        // long.txt is already shown above, so files[] adds only the photo.
        attachment_line("photo.png"),
    );
    assert_eq!(parsed.messages[0].text(), expected);

    // An attachment with nothing extracted, and an image block, leave
    // placeholder lines; a prompt of attachments alone still comes in.
    let parsed = conversation(&holding(json!({
        "sender": "human",
        "text": "",
        "content": [{ "type": "image", "file_uuid": "file-0000-0002" }],
        "attachments": [{ "file_name": "scan.pdf", "extracted_content": "" }],
    })))
    .expect("the prompt imports");
    assert_eq!(
        parsed.messages[0].text(),
        format!("{IMAGE_NOT_IMPORTED}\n\n{}", attachment_line("scan.pdf"))
    );
}

#[test]
fn old_exports_without_parents_follow_time() {
    let h = parsed(1);
    assert_eq!(
        texts(&h),
        [
            (Role::User, "How often should I water tomatoes?".to_string()),
            (
                Role::Assistant,
                "Water deeply in the morning, twice a week.".to_string()
            ),
            (Role::User, "And peppers?".to_string()),
            (
                Role::Assistant,
                "About the same, a little less in shade.".to_string()
            ),
        ]
    );
    assert_eq!(h.candidate.title, "How often should I water tomatoes?");
    assert_eq!(h.dropped_branches, 0);

    // The messages variant, with no parents either.
    let j = parsed(3);
    assert_eq!(j.candidate.id, "conv-packing-j");
    assert_eq!(
        texts(&j),
        [
            (
                Role::User,
                "What should I pack for a weekend hike?".to_string()
            ),
            (Role::Assistant, "Water, layers and a map.".to_string()),
        ]
    );

    // A message whose parent is missing follows the one before it in time.
    let gap = json!({
        "uuid": "conv-gap",
        "chat_messages": [
            { "uuid": "b", "sender": "assistant", "parent_message_uuid": "gone",
              "created_at": "2026-01-01T00:01:00Z", "text": "Second." },
            { "uuid": "a", "sender": "human", "parent_message_uuid": ROOT,
              "created_at": "2026-01-01T00:00:00Z", "text": "First." },
            { "uuid": "c", "sender": "human",
              "created_at": "2026-01-01T00:02:00Z", "text": "Third." },
        ],
    });
    let parsed = conversation(&gap).expect("the gap imports");
    let order: Vec<String> = parsed.messages.iter().map(|m| m.text()).collect();
    assert_eq!(order, ["First.", "Second.", "Third."]);
    assert_eq!(parsed.dropped_branches, 0);
}

#[test]
fn blocks_win_over_text_and_plumbing_stays_out() {
    let parsed = conversation(&holding(json!({
        "sender": "assistant",
        "text": "Old flat text.",
        "content": [
            { "type": "redacted_thinking", "data": "opaque" },
            { "type": "text", "text": "From the blocks." },
            { "type": "token_budget" },
            { "type": "voice_note", "title": "Note" },
            { "type": "text", "text": "Second block." },
        ],
    })))
    .expect("the reply imports");
    assert_eq!(
        parsed.messages[0].parts,
        vec![
            PortablePart::Text("From the blocks.".into()),
            PortablePart::Text("Second block.".into()),
        ]
    );

    // A prompt's text blocks win too.
    let parsed = conversation(&holding(json!({
        "sender": "human",
        "text": "Flat prompt.",
        "content": [{ "type": "text", "text": "Block prompt." }],
    })))
    .expect("the prompt imports");
    assert_eq!(parsed.messages[0].text(), "Block prompt.");

    // With no text block, the flat text comes in after the other parts.
    let parsed = conversation(&holding(json!({
        "sender": "assistant",
        "text": "Only flat.",
        "content": [{ "type": "thinking", "thinking": "Hmm." }],
    })))
    .expect("the reply imports");
    assert_eq!(
        parsed.messages[0].parts,
        vec![
            PortablePart::Thinking("Hmm.".into()),
            PortablePart::Text("Only flat.".into()),
        ]
    );

    // Other senders, and messages left empty, stay out; with nothing left
    // the conversation is skipped.
    assert!(conversation(&json!({
        "uuid": "conv-empty",
        "chat_messages": [
            { "sender": "system", "text": "You are helpful." },
            { "sender": "human", "text": "   " },
            { "sender": "assistant", "content": [{ "type": "injected_prompt_block", "text": "x" }] },
        ],
    }))
    .is_none());
    assert!(conversation(&json!({ "uuid": "conv-none", "chat_messages": [] })).is_none());
    assert!(
        conversation(&json!({ "chat_messages": [{ "sender": "human", "text": "No id." }] }))
            .is_none()
    );
}

#[test]
fn parent_loops_stay_bounded() {
    let started = Instant::now();
    let i = conversation(&fixtures()[2]);
    assert!(started.elapsed() < Duration::from_millis(100));
    let i = i.expect("a loop still imports what it reached");
    let seen: Vec<String> = i.messages.iter().map(|m| m.text()).collect();
    let unique: HashSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len(), "{seen:?}");
    assert!(seen.len() <= 3);

    // A chain past MAX_NODES stops there and keeps the newest MAX_MESSAGES.
    let total = 100_001;
    let messages: Vec<Value> = (0..total)
        .map(|i| {
            let parent = if i == 0 {
                ROOT.to_string()
            } else {
                format!("m{:06}", i - 1)
            };
            json!({
                "uuid": format!("m{i:06}"),
                "sender": if i % 2 == 0 { "human" } else { "assistant" },
                "parent_message_uuid": parent,
                "created_at": 1_700_000_000_000_i64 + i64::from(i),
                "text": format!("message {i}"),
            })
        })
        .collect();
    let chain = json!({ "uuid": "conv-chain", "chat_messages": messages });
    let started = Instant::now();
    let parsed = conversation(&chain).expect("a long chain imports");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(parsed.messages.len(), MAX_MESSAGES);
    assert_eq!(
        parsed.messages.last().map(|m| m.text()).as_deref(),
        Some("message 99999")
    );
    assert_eq!(parsed.messages[0].text(), "message 80000");
    // Untitled: the branch's first prompt names it, though it wasn't kept.
    assert_eq!(parsed.candidate.title, "message 0");
}

/// About 1 MB of hostile input in every text-bearing field finishes in
/// well under a second.
#[test]
fn hostile_sizes_stay_linear() {
    let blocks: Vec<Value> = (0..60_000)
        .map(|i| match i % 3 {
            0 => json!({ "type": "text", "text": "word " }),
            1 => json!({ "type": "tool_use", "name": "artifacts", "id": "same", "input": {} }),
            _ => json!({ "type": "tool_result", "tool_use_id": "unknown", "content": "x" }),
        })
        .collect();
    let attachments: Vec<Value> = (0..40_000)
        .map(|i| json!({ "file_name": format!("f{i}.txt"), "extracted_content": "y" }))
        .collect();
    let files: Vec<Value> = (0..40_000)
        .map(|i| json!({ "file_name": format!("f{i}.txt") }))
        .collect();
    let conversation_value = json!({
        "uuid": "conv-hostile",
        "chat_messages": [
            { "uuid": "a", "sender": "human", "parent_message_uuid": ROOT,
              "text": "z".repeat(1_000_000),
              "attachments": attachments, "files": files,
              "content": [{ "type": "text", "text": "z".repeat(1_000_000) }] },
            { "uuid": "b", "sender": "assistant", "parent_message_uuid": "a",
              "content": blocks },
        ],
    });
    let started = Instant::now();
    let parsed = conversation(&conversation_value).expect("the hostile record imports");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(parsed.messages.len(), 2);

    let memory_value = json!({
        "conversations_memory": "m".repeat(1_000_000),
        "memory_files": (0..50_000)
            .map(|i| json!({ "path": format!("/memories/{i}.md"), "content": "c" }))
            .collect::<Vec<_>>(),
    });
    let started = Instant::now();
    assert!(memory(&memory_value).is_some());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn wrong_typed_values_are_skipped_without_panicking() {
    for odd in [
        json!({ "uuid": "conv-odd", "chat_messages": [null, 3, "text", []] }),
        json!({ "uuid": "conv-odd", "chat_messages": [{ "sender": 7, "text": "x" }] }),
        json!({ "uuid": "conv-odd", "chat_messages": [{
            "sender": "human", "text": 5, "content": "not blocks",
            "attachments": { "file_name": "a" }, "files": "b",
            "parent_message_uuid": 9, "created_at": { "odd": true },
        }] }),
        json!({ "uuid": 4, "chat_messages": [{ "sender": "human", "text": "x" }] }),
        json!({ "uuid": "conv-odd", "chat_messages": [{
            "sender": "assistant",
            "content": [null, 1, { "type": 2 }, { "type": "text", "text": 3 },
                        { "type": "tool_use", "name": "artifacts" },
                        { "type": "tool_result", "content": { "deep": [1] } }],
        }] }),
    ] {
        let _ = conversation(&odd);
    }
    for odd in [
        json!({ "conversations_memory": 3 }),
        json!({ "memory_files": [null, { "path": 1, "content": 2 }, "x"] }),
        json!({ "project_memories": ["not", "a", "map"] }),
        json!([null, 1, { "project_memories": { "p": 5 } }]),
    ] {
        assert!(is_memory(&odd));
        assert_eq!(memory(&odd), None, "{odd}");
    }
}

#[test]
fn both_memory_shapes_become_suggestions() {
    let older = json_fixture("claude-memories.json");
    assert!(is_memory(&older));
    let record = &older[0];
    assert!(is_memory(record));
    let expected = format!(
        "The owner bakes sourdough on weekends and keeps a small garden.\n\n\
         {PROJECT_MEMORY}\nGarden project: tomatoes and peppers on a sunny balcony.\n\n\
         {PROJECT_MEMORY}\nBread project: prefers metric units."
    );
    // The file read whole, or its one record from the stream, says the same.
    for value in [&older, record] {
        let suggestion = memory(value).expect("the older memory imports");
        assert_eq!(suggestion.source, ImportSource::ClaudeAi);
        assert_eq!(suggestion.text, expected);
    }

    let newer = json_fixture("claude-memories-account.json");
    assert!(is_memory(&newer));
    let suggestion = memory(&newer).expect("the newer memory imports");
    assert_eq!(suggestion.source, ImportSource::ClaudeAi);
    assert_eq!(
        suggestion.text,
        "The owner likes short answers and weekend hikes.\n\n\
         ## /memories/hiking.md\n\nPrefers trails under 15 km.\n\n\
         ## /memories/kitchen.md\n\nBakes bread every Sunday."
    );

    // Nothing in it, nothing suggested.
    assert_eq!(
        memory(&json!({ "conversations_memory": "  ", "memory_files": [] })),
        None
    );
    assert!(!is_memory(&json!({ "uuid": "x" })));
}

#[test]
fn the_manifest_is_recognised_not_followed() {
    let manifest = json_fixture("claude-manifest.json");
    assert!(is_manifest(&manifest));
    assert!(!is_conversation(&manifest));
    assert!(!is_memory(&manifest));
    let urls: Vec<&str> = manifest["data_files"]
        .as_array()
        .expect("data_files")
        .iter()
        .filter_map(|file| file["export_url"].as_str())
        .collect();
    assert_eq!(urls.len(), 3);
    assert!(urls
        .iter()
        .all(|url| url.starts_with("https://example.com/")));
    assert!(!is_manifest(
        &json!({ "data_files": [{ "filename": "a.zip" }] })
    ));
    assert!(!is_manifest(&fixtures()[0]));
}

#[test]
fn only_claude_records_are_recognised() {
    for (index, record) in fixtures().iter().enumerate() {
        assert!(is_conversation(record), "{index}");
        assert!(!is_memory(record), "{index}");
        assert!(!is_manifest(record), "{index}");
    }
    // A ChatGPT record has a mapping, even beside chat_messages.
    assert!(!is_conversation(
        &json!({ "mapping": {}, "chat_messages": [] })
    ));
    // The messages variant needs a uuid and a sender.
    assert!(!is_conversation(
        &json!({ "messages": [{ "sender": "human" }] })
    ));
    assert!(!is_conversation(
        &json!({ "uuid": "x", "messages": [{ "role": "user" }] })
    ));
    // users.json is personal data, and none of these.
    let user = json!({
        "uuid": "user-0000-0001",
        "full_name": "Sample Owner",
        "email_address": "owner@example.com",
        "verified_phone_number": null,
    });
    assert!(!is_conversation(&user));
    assert!(!is_memory(&user));
    assert!(!is_manifest(&user));
    assert!(conversation(&user).is_none());
    assert_eq!(memory(&user), None);
}
