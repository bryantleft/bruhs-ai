//! Small string helpers.

/// Cut `s` to at most `max` bytes without splitting a UTF-8 sequence.
/// Moved from bruhs-core `git.rs`.
pub fn truncate_utf8(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// A thread title as shown: inline-code backticks dropped when they come in
/// pairs (titles are often a first prompt such as "Fix `parse` in
/// `lib.rs`"), or when the title was cut short ("…") through a pair. Any
/// other odd backtick count is left alone, since one may be literal.
pub fn plain_title(title: &str) -> std::borrow::Cow<'_, str> {
    let ticks = title.matches('`').count();
    if ticks == 0 || (ticks % 2 == 1 && !title.ends_with('…')) {
        return std::borrow::Cow::Borrowed(title);
    }
    std::borrow::Cow::Owned(title.replace('`', ""))
}

/// A thread rename, trimmed; `None` keeps the old title (a blank or
/// unchanged draft). Moved from the web's `chat_header.rs` for desktop.
pub fn resolve_rename(draft: &str, original: &str) -> Option<String> {
    let trimmed = draft.trim();
    if trimmed.is_empty() || trimmed == original {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// `raw` as one percent-encoded URL component: unreserved characters
/// (letters, digits, `-`, `_`, `.`, `~`) as they are, every other byte as
/// `%XX`. Both apps build path and query segments with it.
pub fn url_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn rename_rejects_empty_and_unchanged() {
        use super::resolve_rename;
        assert_eq!(resolve_rename("   ", "Old"), None);
        assert_eq!(resolve_rename("Old", "Old"), None);
        assert_eq!(
            resolve_rename("  New name ", "Old"),
            Some("New name".to_string())
        );
    }

    #[test]
    fn plain_title_drops_paired_backticks_only() {
        use super::plain_title;
        assert_eq!(
            plain_title("Implement `compute_invoice` in `invoice.py`"),
            "Implement compute_invoice in invoice.py"
        );
        assert_eq!(plain_title("Plain title"), "Plain title");
        assert_eq!(plain_title("Why does ` break it"), "Why does ` break it");
        assert_eq!(
            plain_title("Implement `compute_invoice` in `invoice.py` exactly as `SPEC…"),
            "Implement compute_invoice in invoice.py exactly as SPEC…"
        );
    }

    use super::*;

    #[test]
    fn truncate_utf8_respects_char_boundaries() {
        let mut s = "héllo".to_string();
        truncate_utf8(&mut s, 2);
        assert_eq!(s, "h");
        let mut ok = "abc".to_string();
        truncate_utf8(&mut ok, 10);
        assert_eq!(ok, "abc");
        let mut crab = "🦀🦀".to_string();
        truncate_utf8(&mut crab, 5);
        assert_eq!(crab, "🦀");
        truncate_utf8(&mut crab, 0);
        assert_eq!(crab, "");
    }

    #[test]
    fn url_components_keep_only_unreserved_characters() {
        assert_eq!(super::url_component("a b/c"), "a%20b%2Fc");
        assert_eq!(super::url_component("~/my-agent_1.x"), "~%2Fmy-agent_1.x");
        assert_eq!(super::url_component("é&="), "%C3%A9%26%3D");
    }
}
