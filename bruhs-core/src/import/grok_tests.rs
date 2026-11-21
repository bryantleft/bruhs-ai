//! Grok conversations from the community-observed export format (Task 6,
//! best effort). Every fixture is synthetic.

use super::grok::{conversation, is_conversation};
use super::test_support::{assert_scales_linearly, fixture};
use super::ParsedConversation;
use bruhs_models::agent::Role;
use bruhs_models::imports::{
    attachment_line, IMAGE_NOT_IMPORTED, MAX_MESSAGES, MAX_NODES, MAX_PART_CHARS,
};
use bruhs_models::logic::portable::{title_from_prompt, ImportSource, PortablePart};
use serde_json::{json, Value};
use std::collections::HashSet;

/// The whole `prod-grok-backend.json` fixture.
fn backend() -> Value {
    serde_json::from_slice(fixture("grok-prod-grok-backend.json")).expect("the fixture is JSON")
}

/// The four fixture conversations, K to N.
fn fixtures() -> Vec<Value> {
    let Some(Value::Array(all)) = backend().get("conversations").cloned() else {
        panic!("the fixture holds a conversations array");
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

/// A record with `responses`, each wrapped as the export wraps it.
fn record(meta: Value, responses: Vec<Value>) -> Value {
    let responses: Vec<Value> = responses
        .into_iter()
        .map(|response| json!({ "response": response }))
        .collect();
    json!({ "conversation": meta, "responses": responses })
}

fn mongo(ms: i64) -> Value {
    json!({ "$date": { "$numberLong": ms.to_string() } })
}

#[test]
fn responses_sort_by_time_then_human_first() {
    let k = parsed(0);
    assert_eq!(k.candidate.source, ImportSource::Grok);
    assert_eq!(k.candidate.id, "grok-trip-0001");
    assert_eq!(k.candidate.title, "Weekend in Lisbon");
    assert_eq!(k.candidate.cwd, None);
    assert_eq!(k.candidate.provider, None);
    assert_eq!(k.candidate.session_id, None);
    assert_eq!(k.candidate.model, None);
    assert_eq!(k.dropped_branches, 0);
    assert_eq!(
        texts(&k),
        [
            // Listed after the reply with the same ms, but the prompt
            // comes first.
            (Role::User, "Plan two days in Lisbon.".to_string()),
            (
                Role::Assistant,
                "Lisbon in May is mild. Pack a light jacket.".to_string()
            ),
            // The same ms and the same sender: `_id` orders them.
            (Role::User, "Also add food.".to_string()),
            (Role::User, "Make it a table.".to_string()),
            (
                Role::Assistant,
                "| Day | Plan |\n|---|---|\n| 1 | Alfama and pastel de nata |".to_string()
            ),
        ]
    );
    assert_eq!(k.messages[0].started_at_ms, Some(1_777_626_001_000));
    assert_eq!(k.messages[4].started_at_ms, Some(1_777_626_006_000));

    // A full tie (time, sender and id) keeps the array's order.
    let tie = record(
        json!({ "id": "grok-tie" }),
        vec![
            json!({ "_id": "same", "sender": "human", "message": "one", "create_time": mongo(5) }),
            json!({ "_id": "same", "sender": "human", "message": "two", "create_time": mongo(5) }),
        ],
    );
    let tie = conversation(&tie).expect("the tie imports");
    assert_eq!(tie.messages[0].text(), "one");
    assert_eq!(tie.messages[1].text(), "two");

    // A response with no time sorts where the one before it in the array
    // does, so it stays beside it.
    let untimed = record(
        json!({ "id": "grok-untimed" }),
        vec![
            json!({ "_id": "b", "sender": "human", "message": "late", "create_time": mongo(9_000) }),
            json!({ "_id": "c", "sender": "assistant", "message": "late answer" }),
            json!({ "_id": "a", "sender": "human", "message": "early", "create_time": mongo(1_000) }),
        ],
    );
    let untimed = conversation(&untimed).expect("the untimed record imports");
    assert_eq!(
        texts(&untimed),
        [
            (Role::User, "early".to_string()),
            (Role::User, "late".to_string()),
            (Role::Assistant, "late answer".to_string()),
        ]
    );
    assert_eq!(untimed.messages[2].started_at_ms, None);
}

#[test]
fn the_leaf_chain_wins_when_present() {
    let l = parsed(1);
    assert_eq!(l.candidate.id, "grok-chain-0002");
    assert_eq!(
        texts(&l),
        [
            (Role::User, "How do I feed a starter?".to_string()),
            (
                Role::Assistant,
                "Feed it twice a day at room temperature.".to_string()
            ),
            (Role::User, "And in the fridge?".to_string()),
            (Role::Assistant, "Once a week.".to_string()),
        ]
    );
    // The older answer's newer follow-up is the one other leaf.
    assert_eq!(l.dropped_branches, 1);
    // modify_time (18:30:05) is later than every kept response, so it
    // stands.
    assert_eq!(l.candidate.updated_at.as_millis(), 1_777_746_605_000);

    // A parent cycle falls back to time order.
    let m = parsed(2);
    assert_eq!(
        texts(&m),
        [
            (Role::User, "First in time.".to_string()),
            (Role::Assistant, "Second in time.".to_string()),
        ]
    );
    assert_eq!(m.dropped_branches, 0);
    // `{"$date": "<iso>"}` reads too.
    assert_eq!(m.messages[1].started_at_ms, Some(1_777_792_501_000));

    // A self-parent is a cycle too.
    let selfish = record(
        json!({ "id": "grok-self", "leaf_response_id": "b" }),
        vec![
            json!({ "_id": "a", "sender": "human", "message": "first", "create_time": mongo(1) }),
            json!({ "_id": "b", "sender": "assistant", "message": "second",
                    "parent_response_id": "b", "create_time": mongo(2) }),
        ],
    );
    let selfish = conversation(&selfish).expect("a self-parent imports");
    assert_eq!(selfish.messages.len(), 2);
    assert_eq!(selfish.dropped_branches, 0);

    // A leaf that isn't there, or a chain with no parents at all, reads
    // in time order.
    for meta in [
        json!({ "id": "grok-dangling", "leaf_response_id": "missing" }),
        json!({ "id": "grok-unthreaded", "leaf_response_id": "b" }),
    ] {
        let threaded = meta["id"] == "grok-dangling";
        let parent = |id: &str| if threaded { json!(id) } else { Value::Null };
        let value = record(
            meta,
            vec![
                json!({ "_id": "b", "sender": "assistant", "message": "two",
                        "parent_response_id": parent("a"), "create_time": mongo(2) }),
                json!({ "_id": "a", "sender": "human", "message": "one", "create_time": mongo(1) }),
            ],
        );
        let parsed = conversation(&value).expect("the record imports");
        assert_eq!(
            texts(&parsed),
            [
                (Role::User, "one".to_string()),
                (Role::Assistant, "two".to_string())
            ]
        );
        assert_eq!(parsed.dropped_branches, 0);
    }

    // A parent that names nothing joins the response before it in time,
    // so a half-threaded chain still reaches its start.
    let half = record(
        json!({ "id": "grok-half", "leaf_response_id": "c" }),
        vec![
            json!({ "_id": "a", "sender": "human", "message": "one", "create_time": mongo(1) }),
            json!({ "_id": "b", "sender": "assistant", "message": "two",
                    "parent_response_id": "gone", "create_time": mongo(2) }),
            json!({ "_id": "c", "sender": "human", "message": "three",
                    "parent_response_id": "b", "create_time": mongo(3) }),
        ],
    );
    let half = conversation(&half).expect("the half-threaded chain imports");
    assert_eq!(half.messages.len(), 3);
    assert_eq!(half.dropped_branches, 0);
}

#[test]
fn markup_is_stripped() {
    let reply = |message: &str| {
        let value = record(
            json!({ "id": "grok-markup" }),
            vec![json!({ "_id": "a", "sender": "assistant", "message": message })],
        );
        conversation(&value)
            .map(|parsed| parsed.messages[0].text())
            .unwrap_or_default()
    };
    assert_eq!(
        reply(
            "Fact<grok:render card_id=\"c1\" card_type=\"citation_card\">\
             <argument name=\"citation_id\">1</argument></grok:render>."
        ),
        "Fact."
    );
    assert_eq!(reply("<xaiArtifact title=\"x\">code</xaiArtifact>"), "code");
    // A reply that is only markup is empty, so it isn't there.
    assert_eq!(
        reply("<grok:render card_id=\"c1\">1</grok:render>"),
        String::new()
    );
    // The fixture's citation card is gone, and its artifact's text stays.
    let k = parsed(0);
    assert!(!k.messages[1].text().contains("grok:render"));
    assert!(!k.messages[4].text().contains("xaiArtifact"));

    // What the owner typed is never rewritten.
    let typed = record(
        json!({ "id": "grok-typed" }),
        vec![json!({ "_id": "a", "sender": "human",
                     "message": "What does <xaiArtifact> mean?" })],
    );
    assert_eq!(
        conversation(&typed).expect("the prompt imports").messages[0].text(),
        "What does <xaiArtifact> mean?"
    );
}

#[test]
fn roles_ignore_case_and_never_come_from_model() {
    let k = parsed(0);
    // `ASSISTANT` is the assistant, with its model.
    assert_eq!(k.messages[4].role, Role::Assistant);
    assert_eq!(k.messages[4].model.as_deref(), Some("grok-3"));
    // A human response carries `model` too, and is still the owner's.
    assert_eq!(k.messages[0].role, Role::User);
    assert_eq!(k.messages[0].model, None);
    // The system sender's response is skipped.
    assert!(k
        .messages
        .iter()
        .all(|message| !message.text().contains("system note")));

    let value = record(
        json!({ "id": "grok-roles" }),
        vec![
            json!({ "_id": "a", "sender": "Human", "model": "grok-3", "message": "hi",
                    "create_time": mongo(1) }),
            json!({ "_id": "b", "sender": "tool", "model": "grok-3", "message": "plumbing",
                    "create_time": mongo(2) }),
            json!({ "_id": "c", "model": "grok-3", "message": "no sender",
                    "create_time": mongo(3) }),
            json!({ "_id": "d", "sender": "aSSISTANT", "message": "hello",
                    "create_time": mongo(4) }),
        ],
    );
    let parsed = conversation(&value).expect("the record imports");
    assert_eq!(
        texts(&parsed),
        [
            (Role::User, "hi".to_string()),
            (Role::Assistant, "hello".to_string())
        ]
    );
}

#[test]
fn empty_responses_are_skipped() {
    let n = parsed(3);
    assert_eq!(n.candidate.id, "grok-files-0004");
    // Untitled: the first prompt names it.
    assert_eq!(
        n.candidate.title,
        title_from_prompt("Read this file please.")
    );
    // modify_time (12:00:01) lags the newest kept response (12:00:04).
    assert_eq!(n.candidate.updated_at.as_millis(), 1_777_896_004_000);

    let roles: Vec<Role> = n.messages.iter().map(|message| message.role).collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::User, Role::Assistant]
    );
    assert_eq!(
        n.messages[0].text(),
        format!(
            "Read this file please.\n\n{}",
            attachment_line("asset-0001")
        )
    );
    // The empty reply is absent; the next one brings its reasoning.
    assert_eq!(
        n.messages[1].parts,
        vec![
            PortablePart::Thinking("Reading the attached list.".into()),
            PortablePart::Text("It lists three tasks.".into()),
        ]
    );
    // A prompt that is only a file is there, as its placeholder line.
    assert_eq!(n.messages[2].text(), attachment_line("asset-0002"));
    // A generated image leaves one line, never its link; thinking with no
    // answer is absent.
    assert_eq!(
        n.messages[3].parts,
        vec![PortablePart::Text(format!(
            "Here is a picture of the list.\n\n{IMAGE_NOT_IMPORTED}"
        ))]
    );
    assert!(n
        .messages
        .iter()
        .all(|message| !format!("{:?}", message.parts).contains("example.com")));

    // The interrupted last reply of K is absent too.
    assert_eq!(parsed(0).messages.len(), 5);
    // K's newest kept response (09:00:06) is later than modify_time.
    assert_eq!(
        parsed(0).candidate.updated_at.as_millis(),
        1_777_626_006_000
    );

    // Asset ids come once each, from strings or objects; an image card
    // leaves the one image line.
    let value = record(
        json!({ "id": "grok-assets" }),
        vec![json!({
            "_id": "a", "sender": "assistant", "message": "",
            "file_attachments": ["asset-1", "asset-1", { "asset_id": "asset-2" }, "", 4],
            "card_attachments_json": ["{\"cardType\":\"image_card\"}", { "card_type": "image_card" }],
            "generated_image_urls": ["https://example.com/a.png", "https://example.com/b.png"],
        })],
    );
    let parsed = conversation(&value).expect("files alone import");
    assert_eq!(
        parsed.messages[0].text(),
        [
            IMAGE_NOT_IMPORTED.to_string(),
            attachment_line("asset-1"),
            attachment_line("asset-2"),
        ]
        .join("\n")
    );

    // A conversation with nothing visible is skipped.
    let empty = record(
        json!({ "id": "grok-empty" }),
        vec![
            json!({ "_id": "a", "sender": "assistant", "message": "  " }),
            json!({ "_id": "b", "sender": "assistant", "thinking_trace": "only thinking" }),
        ],
    );
    assert!(is_conversation(&empty));
    assert!(conversation(&empty).is_none());
}

/// `n` responses whose parents form one ring, times the owner's.
fn looped(n: usize) -> Value {
    let responses: Vec<Value> = (0..n)
        .map(|i| {
            json!({ "_id": format!("r{i}"), "sender": if i % 2 == 0 { "human" } else { "assistant" },
                    "message": format!("m{i}"), "parent_response_id": format!("r{}", (i + 1) % n),
                    "create_time": mongo(i64::try_from(i).unwrap_or(0)) })
        })
        .collect();
    record(
        json!({ "id": "grok-loop", "leaf_response_id": "r5" }),
        responses,
    )
}

/// A chain of `total` responses, each the parent of the next.
fn chain_of(total: usize) -> Value {
    let chain: Vec<Value> = (0..total)
        .map(|i| {
            let parent = if i == 0 {
                Value::Null
            } else {
                json!(format!("r{:06}", i - 1))
            };
            json!({ "_id": format!("r{i:06}"), "sender": if i % 2 == 0 { "human" } else { "assistant" },
                    "message": format!("message {i}"), "parent_response_id": parent,
                    "create_time": mongo(1_700_000_000_000 + i64::try_from(i).unwrap_or(0)) })
        })
        .collect();
    let leaf = format!("r{:06}", total.min(MAX_NODES) - 1);
    record(
        json!({ "id": "grok-chain", "leaf_response_id": leaf }),
        chain,
    )
}

#[test]
fn parent_loops_and_long_chains_stay_bounded() {
    // Loops and long chains take time in step with their size, and a
    // loop that never ends fails the guard.
    assert_scales_linearly("a Grok parent ring", 5_000, looped, |v| {
        let _ = conversation(v);
    });
    assert_scales_linearly("a Grok chain", 25_000, chain_of, |v| {
        let _ = conversation(v);
    });

    let parsed = conversation(&looped(2_000)).expect("a loop imports in time order");
    let seen: Vec<String> = parsed.messages.iter().map(|m| m.text()).collect();
    let unique: HashSet<&String> = seen.iter().collect();
    assert_eq!(unique.len(), seen.len());
    assert_eq!(seen.len(), 2_000);

    // A chain past MAX_NODES stops there and keeps the newest MAX_MESSAGES.
    let parsed = conversation(&chain_of(MAX_NODES + 1)).expect("a long chain imports");
    assert_eq!(parsed.messages.len(), MAX_MESSAGES);
    assert_eq!(
        parsed.messages.last().map(|m| m.text()).as_deref(),
        Some(format!("message {}", MAX_NODES - 1).as_str())
    );
    // Untitled: the branch's first prompt names it, though it wasn't kept.
    assert_eq!(parsed.candidate.title, "message 0");
}

/// Hostile input in every text-bearing field, `scale` 4 being about 1 MB
/// of each and 50 000 files and cards.
fn hostile(scale: usize) -> Value {
    let files: Vec<Value> = (0..12_500 * scale)
        .map(|i| json!(format!("asset-{i}")))
        .collect();
    let cards: Vec<Value> = (0..12_500 * scale).map(|_| json!("x".repeat(20))).collect();
    record(
        json!({ "id": "grok-hostile", "title": "t".repeat(250_000 * scale) }),
        vec![
            json!({ "_id": "a", "sender": "human", "message": "z".repeat(250_000 * scale),
                    "file_attachments": files, "create_time": mongo(1) }),
            json!({ "_id": "b", "sender": "assistant", "create_time": mongo(2),
                    "message": "<grok:render>".repeat(20_000 * scale),
                    "thinking_trace": "<xaiArtifact".repeat(20_000 * scale),
                    "card_attachments_json": cards }),
            json!({ "_id": "c", "sender": "assistant", "create_time": mongo(3),
                    "message": "<xaiArtifact x>a</xaiArtifact>".repeat(7_500 * scale) }),
        ],
    )
}

/// Hostile input in every text-bearing field takes time in step with its
/// size, from about 250 KB to 1 MB of each.
#[test]
fn hostile_sizes_stay_linear() {
    assert_scales_linearly("a hostile Grok record", 1, hostile, |v| {
        let _ = conversation(v);
    });
    let parsed = conversation(&hostile(4)).expect("the hostile record imports");
    assert_eq!(parsed.messages.len(), 2);
    // Each part is cut to MAX_PART_CHARS plus the cut line.
    for part in parsed.messages.iter().flat_map(|m| &m.parts) {
        let (PortablePart::Text(text) | PortablePart::Thinking(text)) = part else {
            panic!("only text and thinking");
        };
        assert!(text.chars().count() < MAX_PART_CHARS + 100);
    }
}

#[test]
fn wrong_typed_values_are_skipped_without_panicking() {
    for odd in [
        json!({ "conversation": { "id": "grok-odd" }, "responses": [null, 3, "text", []] }),
        json!({ "conversation": { "id": "grok-odd" },
                "responses": [{ "response": { "sender": 7, "message": "x" } }] }),
        json!({ "conversation": { "id": "grok-odd" },
                "responses": [{ "response": { "sender": "human", "message": 5,
                    "file_attachments": "asset", "generated_image_urls": "u",
                    "card_attachments_json": { "cardType": "image_card" },
                    "create_time": { "odd": true }, "parent_response_id": 9 } }] }),
        json!({ "conversation": { "id": 4 },
                "responses": [{ "response": { "sender": "human", "message": "x" } }] }),
        json!({ "conversation": { "id": "" },
                "responses": [{ "response": { "sender": "human", "message": "x" } }] }),
        json!({ "conversation": { "id": "grok-odd" }, "responses": "not a list" }),
        json!({ "conversation": "grok-odd", "responses": [] }),
    ] {
        assert!(conversation(&odd).is_none(), "{odd}");
    }
    // Odd conversation fields fall back; an unwrapped response reads too.
    let parsed = conversation(&json!({
        "conversation": { "id": "grok-odd", "title": 3, "modify_time": [1],
                          "leaf_response_id": 2 },
        "responses": [
            { "_id": 1, "sender": "human", "message": "unwrapped", "create_time": "x",
              "thinking_trace": 3 },
            { "response": { "_id": "b", "sender": "assistant", "message": "reply",
                            "thinking_trace": ["a", 2, "b"], "model": 9 } },
        ],
    }))
    .expect("the odd record imports");
    assert_eq!(parsed.candidate.title, "unwrapped");
    assert_eq!(parsed.candidate.updated_at.as_millis(), 0);
    assert_eq!(
        texts(&parsed),
        [
            (Role::User, "unwrapped".to_string()),
            (Role::Assistant, "reply".to_string())
        ]
    );
    assert_eq!(parsed.messages[1].model, None);
    assert_eq!(
        parsed.messages[1].parts[0],
        PortablePart::Thinking("a\n\nb".into())
    );
}

#[test]
fn only_grok_records_are_recognised() {
    for (index, record) in fixtures().iter().enumerate() {
        assert!(is_conversation(record), "{index}");
        assert!(!super::chatgpt::is_conversation(record), "{index}");
        assert!(!super::claude_ai::is_conversation(record), "{index}");
    }
    // The file's other lists, and the file itself, aren't conversations.
    let backend = backend();
    for key in ["projects", "tasks", "media_posts"] {
        for item in backend[key].as_array().expect("a list") {
            assert!(!is_conversation(item), "{key}");
            assert!(conversation(item).is_none(), "{key}");
        }
    }
    assert!(!is_conversation(&backend));
    // ChatGPT and Claude records, and both manifests, aren't Grok's.
    let others = [
        json!({ "conversation_id": "c", "mapping": {} }),
        json!({ "uuid": "u", "chat_messages": [] }),
        serde_json::from_slice(fixture("chatgpt-export_manifest.json")).expect("JSON"),
        serde_json::from_slice(fixture("claude-manifest.json")).expect("JSON"),
        json!({ "conversation": { "id": "x" } }),
        json!({ "responses": [] }),
    ];
    for other in &others {
        assert!(!is_conversation(other), "{other}");
        assert!(conversation(other).is_none(), "{other}");
    }
}
