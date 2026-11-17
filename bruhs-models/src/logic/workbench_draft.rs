//! Workbench Verification's "Use draft": dictation a Workbench page offers
//! the chat composer. The web receives it with `postMessage` from its
//! frame, the desktop from its embedded page through `window.ipc`; both
//! check and append it the same way. Moved from the web's `workbench.rs`.

use serde::Deserialize;

use super::composer_text::MESSAGE_CHAR_LIMIT;

/// The message type a Workbench page sends with a draft.
pub const DICTATION_DRAFT_MESSAGE: &str = "bruhs-dictation-draft";

#[derive(Deserialize)]
struct DraftMessage {
    r#type: String,
    text: String,
}

/// The draft text in `body`, a page's JSON message, when the page that sent
/// it (`frame_url`) is one of bruhs-server's own at `server_base` and the
/// text is not blank. Anything else is ignored.
pub fn dictation_draft_text(frame_url: &str, server_base: &str, body: &str) -> Option<String> {
    let base = server_base.trim_end_matches('/');
    let own_page = !base.is_empty()
        && frame_url
            .strip_prefix(base)
            .is_some_and(|rest| rest.starts_with('/'));
    if !own_page {
        return None;
    }
    let message: DraftMessage = serde_json::from_str(body).ok()?;
    (message.r#type == DICTATION_DRAFT_MESSAGE && !message.text.trim().is_empty())
        .then_some(message.text)
}

/// The composer text with `draft` added: on a new line unless the text is
/// empty or ends in whitespace, and cut so the whole stays within
/// [`MESSAGE_CHAR_LIMIT`].
pub fn append_dictation_draft(current: &str, draft: &str) -> String {
    let separator = if current.is_empty() || current.ends_with(char::is_whitespace) {
        ""
    } else {
        "\n"
    };
    let remaining =
        MESSAGE_CHAR_LIMIT.saturating_sub(current.chars().count() + separator.chars().count());
    let mut text = String::with_capacity(current.len() + separator.len() + draft.len());
    text.push_str(current);
    text.push_str(separator);
    text.extend(draft.chars().take(remaining));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://127.0.0.1:3001";

    fn message(text: &str) -> String {
        serde_json::json!({ "type": DICTATION_DRAFT_MESSAGE, "text": text }).to_string()
    }

    #[test]
    fn only_bruhs_pages_can_offer_a_draft() {
        let page = "http://127.0.0.1:3001/api/workbench/ui#verification";
        assert_eq!(
            dictation_draft_text(page, BASE, &message("Fix the parser")),
            Some("Fix the parser".into())
        );
        assert_eq!(
            dictation_draft_text(page, "http://127.0.0.1:3001/", &message("x")),
            Some("x".into())
        );
        for other in [
            "http://127.0.0.1:3001.example.com/api/workbench/ui",
            "http://127.0.0.1:30011/api/workbench/ui",
            "https://example.com/http://127.0.0.1:3001/",
            "http://127.0.0.1:3001",
        ] {
            assert_eq!(
                dictation_draft_text(other, BASE, &message("x")),
                None,
                "{other}"
            );
        }
        assert_eq!(dictation_draft_text(page, "", &message("x")), None);
    }

    #[test]
    fn a_draft_needs_its_type_and_some_text() {
        let page = "http://127.0.0.1:3001/api/workbench/ui";
        assert_eq!(dictation_draft_text(page, BASE, &message("  \n ")), None);
        assert_eq!(dictation_draft_text(page, BASE, "not json"), None);
        let other = serde_json::json!({ "type": "bruhs:open-task", "text": "x" }).to_string();
        assert_eq!(dictation_draft_text(page, BASE, &other), None);
    }

    #[test]
    fn a_draft_goes_on_a_new_line_within_the_message_limit() {
        assert_eq!(append_dictation_draft("", "Hello"), "Hello");
        assert_eq!(append_dictation_draft("Fix", "the bug"), "Fix\nthe bug");
        assert_eq!(append_dictation_draft("Fix ", "the bug"), "Fix the bug");
        assert_eq!(append_dictation_draft("Fix\n", "the bug"), "Fix\nthe bug");
        let full = "a".repeat(MESSAGE_CHAR_LIMIT - 3);
        assert_eq!(
            append_dictation_draft(&full, "bcdef"),
            format!("{full}\nbc"),
            "cut at the limit, counting the new line"
        );
        assert_eq!(
            append_dictation_draft(&"a".repeat(MESSAGE_CHAR_LIMIT), "more")
                .chars()
                .count(),
            MESSAGE_CHAR_LIMIT + 1,
            "like the web, the separator is still added when nothing fits"
        );
    }
}
