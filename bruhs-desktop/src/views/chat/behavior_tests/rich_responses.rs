#[cfg(target_os = "macos")]
use super::*;

#[gpui::test]
#[cfg(target_os = "macos")]
fn interactive_response_source_can_be_opened_without_a_native_handle(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.messages = vec![rich_message(
            "```bruhs-html Counter\n<button onclick='this.textContent++'>0</button>\n```",
        )];
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-rich-response-0-0").is_some());
    assert!(cx.debug_bounds("desktop-rich-source-0-0").is_none());
    let toggle = cx
        .debug_bounds("desktop-rich-source-toggle-0-0")
        .expect("source button");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-rich-source-0-0").is_some());
}

#[gpui::test]
#[cfg(target_os = "macos")]
fn external_media_requires_a_click_for_each_changed_source(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    chat.update(cx, |chat, cx| {
        chat.messages = vec![rich_message("![Example](https://example.com/image.png)")];
        cx.notify();
    });
    cx.run_until_parked();
    let load = cx
        .debug_bounds("desktop-rich-load-0-0")
        .expect("explicit media consent");
    cx.simulate_click(load.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-rich-load-0-0").is_none());
    chat.update(cx, |chat, cx| {
        chat.messages[0].content = "![Changed](https://example.com/other.png)".into();
        cx.notify();
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("desktop-rich-load-0-0").is_some());
}

#[test]
fn open_interactive_fences_stay_as_code_until_closed() {
    use super::super::{parse_assistant_content, ParsedAssistantBlock};
    let open = "```bruhs-html Counter\n<button>0</button>";
    for streaming in [true, false] {
        assert!(!parse_assistant_content(open, streaming)
            .iter()
            .any(|block| matches!(block, ParsedAssistantBlock::Rich(_))));
    }
    assert!(parse_assistant_content(&format!("{open}\n```"), true)
        .iter()
        .any(|block| matches!(block, ParsedAssistantBlock::Rich(_))));
}

#[test]
fn rich_responses_preserve_surrounding_math_and_mermaid_blocks() {
    use super::super::{parse_assistant_content, ParsedAssistantBlock};
    let blocks = parse_assistant_content(
        "$$x^2$$\n\n```bruhs-html Counter\n<button>0</button>\n```\n\n```mermaid\ngraph TD\nA-->B\n```",
        false,
    );
    assert!(matches!(
        &blocks[..],
        [
            ParsedAssistantBlock::Math { .. },
            ParsedAssistantBlock::Rich(_),
            ParsedAssistantBlock::Mermaid(_)
        ]
    ));
}

/// A reply can carry a link to a launcher (a `.command` file, another
/// app's scheme). As in crew prose, a click opens only a web or mail link.
#[gpui::test]
#[cfg(target_os = "macos")]
fn reply_links_open_only_web_and_mail_addresses(cx: &mut TestAppContext) {
    let (chat, cx) = chat_window(cx);
    let mut click = |url: &str| {
        chat.update(cx, |chat, cx| {
            chat.messages = vec![rich_message(&format!("[open the full report]({url})"))];
            cx.notify();
        });
        cx.run_until_parked();
        let bounds = cx.debug_bounds("desktop-prose-0-0").expect("the reply");
        let on_link = gpui::point(
            bounds.origin.x + gpui::px(20.0),
            bounds.origin.y + gpui::px(8.0),
        );
        cx.simulate_click(on_link, gpui::Modifiers::default());
        cx.run_until_parked();
        cx.opened_url()
    };
    for url in [
        "file:///Users/me/Downloads/run.command",
        "vscode://file/etc/hosts",
    ] {
        assert_eq!(click(url), None, "{url}");
    }
    assert_eq!(
        click("mailto:bo@example.com").as_deref(),
        Some("mailto:bo@example.com")
    );
}
