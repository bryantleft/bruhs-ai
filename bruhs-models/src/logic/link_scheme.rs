//! Which links from untrusted text (a reply, a bruh's words) the apps may
//! follow: http, https and mailto only. Anything else (`file:`,
//! `javascript:`, another app's scheme) would hand the OS a launcher.
//! Moved from the web's `markdown::safe_url`, so the desktop's Markdown
//! links follow the same rule.

/// The schemes a link from untrusted text may use.
pub const SAFE_LINK_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// `url` as a browser's URL parser reads it: leading and trailing C0
/// controls and spaces ignored, tabs and newlines removed anywhere.
pub fn normalized_url(url: &str) -> String {
    url.trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

/// The scheme of `url`, lowercased and read the way a browser reads it, so
/// `"\tjava\nscript:"` is `javascript`. `None` when it has none: a relative
/// path, `//host`, `?query` or `#fragment`.
pub fn link_scheme(url: &str) -> Option<String> {
    let normalized = normalized_url(url);
    let at = normalized.find([':', '/', '?', '#'])?;
    // A colon before any path, query or fragment delimiter ends a scheme.
    normalized[at..]
        .starts_with(':')
        .then(|| normalized[..at].to_ascii_lowercase())
}

/// Whether a click on `url` may hand it to the browser or the mail app:
/// its scheme is one of [`SAFE_LINK_SCHEMES`]. A link with no scheme is
/// not safe here; a page that resolves relative links checks those itself.
pub fn is_safe_link(url: &str) -> bool {
    link_scheme(url).is_some_and(|scheme| SAFE_LINK_SCHEMES.contains(&scheme.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_links_are_safe() {
        for url in [
            "https://example.com/a?b#c",
            "HTTP://example.com",
            " https://example.com",
            "ht\ttps://example.com",
            "mailto:me@example.com",
            "MailTo:me@example.com",
        ] {
            assert!(is_safe_link(url), "{url:?}");
        }
    }

    #[test]
    fn every_other_scheme_and_schemeless_links_are_not() {
        for url in [
            "javascript:alert(1)",
            "\tjava\nscript:alert(1)",
            "\u{1}javascript:alert(1)",
            "data:text/html,<b>x</b>",
            "vbscript:msgbox(1)",
            "file:///Users/me/Downloads/run.command",
            "vscode://file/etc/hosts",
            "zoommtg://zoom.us/join",
            "x-apple.systempreferences:com.apple.preference.security",
            "smb://host/share",
            "bruhs://thread/1",
            "https\u{0}://example.com",
            "notes/plan.md",
            "/Users/me/plan.md",
            "./a:b",
            "//example.com/x",
            "#top",
            "",
        ] {
            assert!(!is_safe_link(url), "{url:?}");
        }
    }

    #[test]
    fn the_scheme_is_read_like_a_browser() {
        assert_eq!(link_scheme("JaVa\tScript:x").as_deref(), Some("javascript"));
        assert_eq!(link_scheme("./a:b"), None);
        assert_eq!(link_scheme("?q=a:b"), None);
        assert_eq!(link_scheme("#a:b"), None);
        assert_eq!(link_scheme("me@example.com"), None);
    }
}
