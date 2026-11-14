//! Rich assistant output stays in the saved Markdown, so replay and export
//! need no parallel artifact store or message schema migration.

use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};

use super::attachments::{kind_for_name, FileKind};

mod protocol;
pub use protocol::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseBlock {
    Markdown(String),
    Media(MediaBlock),
    Html(HtmlBlock),
    Artifact(ArtifactBlock),
    Error { source: String, message: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    #[default]
    Image,
    Video,
    Audio,
    Pdf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaBlock {
    pub kind: MediaKind,
    pub src: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<MediaTrack>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaTrack {
    pub src: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub language: String,
    #[serde(default = "subtitle_kind")]
    pub kind: String,
}

fn subtitle_kind() -> String {
    "subtitles".into()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactBlock {
    pub id: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlBlock {
    pub html: String,
    pub title: String,
}

/// Only explicitly closed rich fences execute, including after a stopped or
/// failed turn. Ordinary HTML code examples remain ordinary code examples.
pub fn response_blocks(markdown: &str, _streaming: bool) -> Vec<ResponseBlock> {
    if !markdown.contains("bruhs-") && !markdown.contains('[') {
        return vec![ResponseBlock::Markdown(markdown.to_string())];
    }
    let parser = Parser::new_ext(markdown, Options::all());
    let mut definitions: Vec<_> = parser
        .reference_definitions()
        .iter()
        .map(|(_, def)| def.span.clone())
        .collect();
    definitions.sort_by_key(|range| range.start);
    let mut events = parser.into_offset_iter();
    let mut blocks = Vec::new();
    let mut cursor = 0;
    let mut nesting = 0_usize;
    while let Some((event, range)) = events.next() {
        let block = match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let mut content = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) => content.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let CodeBlockKind::Fenced(info) = kind else {
                    continue;
                };
                let (language, title) = info.split_once(char::is_whitespace).unwrap_or((&info, ""));
                if !matches!(language, "bruhs-html" | "bruhs-media" | "bruhs-artifact")
                    || nesting != 0
                    || !closed_fence(&markdown[range.clone()])
                {
                    continue;
                }
                match language {
                    "bruhs-html" if !content.trim().is_empty() => ResponseBlock::Html(HtmlBlock {
                        html: content,
                        title: if title.trim().is_empty() { "Interactive response".into() } else { title.trim().into() },
                    }),
                    "bruhs-html" => ResponseBlock::Error {
                        source: markdown[range.clone()].into(),
                        message: "The interactive response is empty. Ask the agent to provide its HTML.".into(),
                    },
                    "bruhs-artifact" => match serde_json::from_str::<ArtifactBlock>(&content) {
                        Ok(artifact) if uuid::Uuid::parse_str(&artifact.id).is_ok() => ResponseBlock::Artifact(artifact),
                        _ => ResponseBlock::Error { source: markdown[range.clone()].into(), message: "This artifact needs a valid published artifact ID. Ask the agent to publish its HTML file.".into() },
                    },
                    _ => match serde_json::from_str::<MediaBlock>(&content) {
                        Ok(mut media) => {
                            media.src = media.src.trim().into();
                            match validate_media(&media) {
                                Ok(()) => ResponseBlock::Media(media),
                                Err(message) => ResponseBlock::Error { source: markdown[range.clone()].into(), message: message.into() },
                            }
                        }
                        Err(error) => ResponseBlock::Error {
                            source: markdown[range.clone()].into(),
                            message: format!("Could not display this media: {error}. Use kind, src, and an optional title."),
                        },
                    },
                }
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let mut alt = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        Event::End(TagEnd::Image) => break,
                        _ => {}
                    }
                }
                if nesting != 0
                    || !standalone(markdown, &range)
                    || validate_media_source(&dest_url).is_err()
                {
                    continue;
                }
                ResponseBlock::Media(MediaBlock {
                    kind: MediaKind::Image,
                    src: dest_url.into_string(),
                    title: if alt.is_empty() {
                        title.into_string()
                    } else {
                        alt
                    },
                    ..Default::default()
                })
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let mut title = String::new();
                let mut image_source = None;
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) | Event::Code(text) => title.push_str(&text),
                        Event::Start(Tag::Image { dest_url, .. }) => image_source = Some(dest_url),
                        Event::End(TagEnd::Link) => break,
                        _ => {}
                    }
                }
                let (kind, dest_url) = if let Some(source) = image_source {
                    (MediaKind::Image, source)
                } else {
                    let kind = match kind_for_name(&dest_url) {
                        FileKind::Video => MediaKind::Video,
                        FileKind::Audio => MediaKind::Audio,
                        FileKind::Pdf => MediaKind::Pdf,
                        _ => continue,
                    };
                    (kind, dest_url)
                };
                if nesting != 0
                    || !standalone(markdown, &range)
                    || validate_media_source(&dest_url).is_err()
                {
                    continue;
                }
                ResponseBlock::Media(MediaBlock {
                    kind,
                    src: dest_url.into_string(),
                    title,
                    ..Default::default()
                })
            }
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => continue,
            Event::Start(_) => {
                nesting += 1;
                continue;
            }
            Event::End(_) => {
                nesting = nesting.saturating_sub(1);
                continue;
            }
            _ => continue,
        };
        if cursor < range.start {
            blocks.push(ResponseBlock::Markdown(
                markdown[cursor..range.start].into(),
            ));
        }
        blocks.push(block);
        cursor = range.end;
    }
    if cursor < markdown.len() || blocks.is_empty() {
        blocks.push(ResponseBlock::Markdown(markdown[cursor..].into()));
    }
    if blocks.len() > 1 && !definitions.is_empty() {
        let prefix = definitions
            .iter()
            .map(|range| &markdown[range.clone()])
            .collect::<Vec<_>>()
            .join("\n");
        for block in &mut blocks {
            if let ResponseBlock::Markdown(text) = block {
                if !text.trim().is_empty() {
                    // Prefixing keeps definitions out of an unfinished code
                    // fence at the end of a streamed prose fragment.
                    *text = format!("{prefix}\n\n{text}");
                }
            }
        }
    }
    blocks
}

fn standalone(markdown: &str, range: &std::ops::Range<usize>) -> bool {
    let start = markdown[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let end = markdown[range.end..]
        .find('\n')
        .map_or(markdown.len(), |index| range.end + index);
    markdown[start..range.start].trim().is_empty() && markdown[range.end..end].trim().is_empty()
}

/// Asset discovery also visits media nested in Markdown containers. It is
/// separate from block splitting so tables and lists keep their structure.
pub fn all_media_sources(markdown: &str) -> std::collections::BTreeSet<String> {
    let mut sources = std::collections::BTreeSet::new();
    for event in Parser::new_ext(markdown, Options::all()) {
        match event {
            Event::Start(Tag::Image { dest_url, .. })
                if validate_media_source(&dest_url).is_ok() =>
            {
                sources.insert(dest_url.into_string());
            }
            Event::Start(Tag::Link { dest_url, .. })
                if matches!(
                    kind_for_name(&dest_url),
                    FileKind::Video | FileKind::Audio | FileKind::Pdf
                ) && validate_media_source(&dest_url).is_ok() =>
            {
                sources.insert(dest_url.into_string());
            }
            _ => {}
        }
    }
    for block in response_blocks(markdown, false) {
        if let ResponseBlock::Media(media) = block {
            sources.insert(media.src);
            sources.extend(media.poster);
            sources.extend(media.tracks.into_iter().map(|track| track.src));
        }
    }
    sources
}

/// Rewrite only actual Markdown media spans, never matching prose, examples
/// or JSON strings. Reference images become inline links to the bundled asset.
pub fn rewrite_media_paths(
    markdown: &str,
    paths: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut events = Parser::new_ext(markdown, Options::all()).into_offset_iter();
    let mut edits = Vec::new();
    while let Some((event, range)) = events.next() {
        let (image, dest_url, title) = match event {
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => (true, dest_url, title),
            Event::Start(Tag::Link {
                dest_url, title, ..
            }) if paths.contains_key(dest_url.as_ref()) => (false, dest_url, title),
            _ => continue,
        };
        let mut label = String::new();
        for (event, _) in events.by_ref() {
            match event {
                Event::Text(text) | Event::Code(text) => label.push_str(&text),
                Event::End(TagEnd::Image) if image => break,
                Event::End(TagEnd::Link) if !image => break,
                _ => {}
            }
        }
        let Some(path) = paths.get(dest_url.as_ref()) else {
            continue;
        };
        let label = label
            .replace('\\', "\\\\")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace(['\r', '\n'], " ");
        let title = if title.is_empty() {
            String::new()
        } else {
            format!(
                " \"{}\"",
                title
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace(['\r', '\n'], " ")
            )
        };
        edits.push((
            range,
            format!("{}[{label}](<{path}>{title})", if image { "!" } else { "" }),
        ));
    }
    let mut result = String::with_capacity(markdown.len());
    let mut cursor = 0;
    for (range, value) in edits {
        if range.start < cursor {
            continue;
        }
        result.push_str(&markdown[cursor..range.start]);
        result.push_str(&value);
        cursor = range.end;
    }
    result.push_str(&markdown[cursor..]);
    result
}

/// Persist immutable media bindings in the message itself, so later turns
/// may overwrite a project filename without changing historical replies.
pub fn rewrite_response_media(
    markdown: &str,
    paths: &std::collections::BTreeMap<String, String>,
) -> String {
    let markdown = rewrite_media_paths(markdown, paths);
    let mut events = Parser::new_ext(&markdown, Options::all()).into_offset_iter();
    let mut edits = Vec::new();
    let mut nesting = 0_usize;
    while let Some((event, range)) = events.next() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let mut json = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::Text(text) => json.push_str(&text),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let CodeBlockKind::Fenced(info) = kind else {
                    continue;
                };
                if nesting != 0
                    || info.split_whitespace().next() != Some("bruhs-media")
                    || !closed_fence(&markdown[range.clone()])
                {
                    continue;
                }
                let Ok(mut media) = serde_json::from_str::<MediaBlock>(&json) else {
                    continue;
                };
                let before = media.clone();
                if let Some(path) = paths.get(&media.src) {
                    media.src.clone_from(path)
                }
                if let Some(poster) = &mut media.poster {
                    if let Some(path) = paths.get(poster) {
                        poster.clone_from(path)
                    }
                }
                for track in &mut media.tracks {
                    if let Some(path) = paths.get(&track.src) {
                        track.src.clone_from(path)
                    }
                }
                if media != before {
                    if let Ok(json) = serde_json::to_string(&media) {
                        let suffix = if markdown[range.clone()].ends_with("\r\n") {
                            "\r\n"
                        } else if markdown[range.clone()].ends_with('\n') {
                            "\n"
                        } else {
                            ""
                        };
                        edits.push((range, format!("```bruhs-media\n{json}\n```{suffix}")))
                    }
                }
            }
            Event::Start(Tag::Paragraph) | Event::End(TagEnd::Paragraph) => {}
            Event::Start(_) => nesting += 1,
            Event::End(_) => nesting = nesting.saturating_sub(1),
            _ => {}
        }
    }
    let mut output = String::with_capacity(markdown.len());
    let mut cursor = 0;
    for (range, value) in edits {
        output.push_str(&markdown[cursor..range.start]);
        output.push_str(&value);
        cursor = range.end;
    }
    output.push_str(&markdown[cursor..]);
    output
}

/// GPUI's Markdown renderer also accepts raw HTML images. Drop raw tags in
/// assistant prose so they cannot bypass the explicit remote-media gate.
/// Images nested in Markdown containers stay links rather than breaking
/// their surrounding table, list, or emphasis into independent fragments.
/// Code examples, including HTML code fences, are preserved verbatim.
pub fn markdown_without_raw_html(markdown: &str) -> String {
    markdown_with_allowed_images(markdown, &std::collections::BTreeSet::new())
}

pub fn markdown_with_allowed_images(
    markdown: &str,
    allowed: &std::collections::BTreeSet<String>,
) -> String {
    if !markdown.contains('<') && !markdown.contains("![") {
        return markdown.to_string();
    }
    let mut out = String::with_capacity(markdown.len());
    let mut cursor = 0;
    for (event, range) in Parser::new(markdown).into_offset_iter() {
        let remove = match event {
            Event::Html(_) | Event::InlineHtml(_) => Some(range),
            Event::Start(Tag::Image { dest_url, .. }) if !allowed.contains(dest_url.as_ref()) => {
                Some(range.start..range.start + 1)
            }
            _ => None,
        };
        if let Some(range) = remove {
            if cursor < range.start {
                out.push_str(&markdown[cursor..range.start]);
            }
            cursor = range.end;
        }
    }
    out.push_str(&markdown[cursor..]);
    out
}

fn closed_fence(source: &str) -> bool {
    let mut lines = source.lines();
    let opening = lines.next().unwrap_or("").trim_start();
    let Some(marker @ (b'`' | b'~')) = opening.bytes().next() else {
        return false;
    };
    let count = opening.bytes().take_while(|b| *b == marker).count();
    let Some(closing) = lines.rev().find(|line| !line.trim_ascii().is_empty()) else {
        return false;
    };
    // A lookalike delimiter inside code (four-space indentation, quoted
    // marker, Unicode whitespace) is not a closed executable fence.
    let indentation = closing.bytes().take_while(|b| *b == b' ').count();
    if indentation > 3 {
        return false;
    }
    let closing = &closing[indentation..];
    if closing.bytes().next() != Some(marker) {
        return false;
    }
    let closing = closing.trim_end_matches([' ', '\t', '\r']);
    closing.len() >= count && closing.bytes().all(|b| b == marker)
}

pub fn media_is_remote(src: &str) -> bool {
    let src = src.trim();
    src.starts_with("//")
        || src
            .get(..7)
            .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
        || src
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
}

pub fn validate_media_source(src: &str) -> Result<(), &'static str> {
    if src.is_empty() || src.chars().any(char::is_control) || src.contains('\\') {
        return Err("The media path is empty or contains unsupported characters.");
    }
    if media_is_remote(src) {
        return Ok(());
    }
    let path = src.strip_prefix("file://").unwrap_or(src);
    if path.starts_with('#')
        || path
            .split('/')
            .next()
            .is_some_and(|part| part.contains(':'))
    {
        return Err("Media must use a project file, an attachment, or an HTTP(S) URL.");
    }
    if src.starts_with("file://") && !path.starts_with('/') {
        return Err("File URLs must name a path on this machine.");
    }
    Ok(())
}

fn validate_media(media: &MediaBlock) -> Result<(), &'static str> {
    validate_media_source(&media.src)?;
    if let Some(poster) = &media.poster {
        validate_media_source(poster)?;
    }
    for track in &media.tracks {
        validate_media_source(&track.src)?;
        if !matches!(
            track.kind.as_str(),
            "subtitles" | "captions" | "descriptions" | "chapters" | "metadata"
        ) {
            return Err(
                "Subtitle kind must be subtitles, captions, descriptions, chapters, or metadata.",
            );
        }
    }
    Ok(())
}

/// Local files always go through the scoped media route, never the general
/// file viewer. External resources are loaded only after a user action.
pub fn media_url(api_base: &str, src: &str, project: Option<&str>) -> String {
    if src.starts_with("//") {
        return format!("https:{src}");
    }
    if media_is_remote(src) {
        return src.to_string();
    }
    let path = src.strip_prefix("file://").unwrap_or(src);
    let decoded = percent_decode_str(path).decode_utf8_lossy();
    let mut url = format!(
        "{}/api/agent/media?path={}",
        api_base.trim_end_matches('/'),
        utf8_percent_encode(&decoded, NON_ALPHANUMERIC)
    );
    if let Some(project) = project.filter(|p| !p.is_empty()) {
        url.push_str("&project=");
        url.push_str(&utf8_percent_encode(project, NON_ALPHANUMERIC).to_string());
    }
    url
}

pub const INTERACTIVE_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; media-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'";

/// The first policy cannot be loosened by later assistant-supplied markup.
pub fn interactive_document(html: &str) -> String {
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{INTERACTIVE_CSP}\"><meta name=\"referrer\" content=\"no-referrer\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>:root{{color-scheme:dark light}}body{{margin:0;padding:12px;font:14px var(--bruhs-font,ui-monospace,monospace);background:var(--bruhs-background,transparent);color:var(--bruhs-text,inherit);overflow-wrap:anywhere}}button,input,select,textarea{{font:inherit;accent-color:var(--bruhs-accent,auto)}}img,video,canvas,svg{{max-width:100%}}*{{box-sizing:border-box}}</style><script>{}</script></head><body>{html}</body></html>", include_str!("rich_response/widget-runtime.js"))
}

/// A network document has its own CSP; srcdoc/blob documents would inherit
/// the app's script policy, which deliberately forbids inline JavaScript.
/// The fragment never reaches the server, request logs, or a Referer.
pub fn interactive_url(api_base: &str, html: &str) -> String {
    format!(
        "{}/api/agent/rich-response#{}",
        api_base.trim_end_matches('/'),
        utf8_percent_encode(&interactive_document(html), NON_ALPHANUMERIC)
    )
}

pub fn interactive_url_with_nonce(api_base: &str, html: &str, nonce: &str) -> String {
    let payload = serde_json::json!({"html": interactive_document(html), "nonce": nonce});
    format!(
        "{}/api/agent/rich-response#{}",
        api_base.trim_end_matches('/'),
        utf8_percent_encode(&payload.to_string(), NON_ALPHANUMERIC)
    )
}

/// Trusted native initialization: run in the shell's main frame, never via
/// an untrusted frame's IPC or by inserting a script tag with innerHTML.
pub fn interactive_host_script(html: &str, nonce: &str) -> String {
    let payload = serde_json::json!({"html":interactive_document(html),"nonce":nonce})
        .to_string()
        .replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!(
        "{}\nwindow.bruhsCreateWidget({payload});",
        include_str!("rich_response/host-runtime.js")
    )
}

pub fn standalone_widget_document(
    html: &str,
    state: &WidgetState,
    theme: &std::collections::BTreeMap<String, String>,
) -> String {
    let init = serde_json::json!({"bruhs":1,"type":"init","state":state,"theme":theme})
        .to_string()
        .replace('<', "\\u003c");
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-src about:; connect-src 'none'; base-uri 'none'; form-action 'none'\"><meta name=\"referrer\" content=\"no-referrer\"><style>html,body{{margin:0;height:100%;min-height:80px}}iframe{{border:0;width:100%;height:100%}}</style></head><body><script>{}\nwindow.bruhsHostInit({init});</script></body></html>",interactive_host_script(html,"offline"))
}

/// External media uses a controlled document instead of loosening the
/// app's own image policy (which would also affect Mermaid and previews).
pub fn media_frame_url(api_base: &str, media: &MediaBlock) -> String {
    let mut media = media.clone();
    media.src = media_url(api_base, &media.src, None);
    let json = serde_json::json!(media).to_string();
    format!(
        "{}/api/agent/rich-media#{}",
        api_base.trim_end_matches('/'),
        utf8_percent_encode(&json, NON_ALPHANUMERIC)
    )
}

/// The fragment is data only. Never use innerHTML or execute the supplied
/// source; even a URL mislabeled as PDF is loaded in a scriptless sandbox.
pub const MEDIA_HOST: &str = r#"<!doctype html><html><head><meta charset="utf-8"><meta name="referrer" content="no-referrer"><style>:root{color-scheme:dark light}html,body{margin:0;width:100%;height:100%;overflow:hidden}img,video,iframe{border:0;width:100%;height:100%;object-fit:contain}audio{width:100%}p{font:13px ui-monospace,monospace;padding:12px}</style></head><body><script>
if(location.hash.length>1){try{const data=JSON.parse(decodeURIComponent(location.hash.slice(1)));const tag={image:'img',video:'video',audio:'audio',pdf:'iframe'}[data.kind];if(!tag||typeof data.src!=='string'||!/^https?:\/\//i.test(data.src))throw Error('Unsupported media');const media=document.createElement(tag);const title=typeof data.title==='string'?data.title:'Response media';media.setAttribute('aria-label',title);media.setAttribute('referrerpolicy','no-referrer');if(tag==='img')media.alt=title;if(tag==='video'&&typeof data.poster==='string'&&/^https?:\/\//i.test(data.poster))media.poster=data.poster;if(tag==='iframe'){media.setAttribute('sandbox','');media.title=title;}if(tag==='video'||tag==='audio'){media.controls=true;media.preload='none';media.setAttribute('playsinline','');if(Array.isArray(data.tracks)&&data.tracks.length){media.crossOrigin='anonymous';data.tracks.forEach((item,index)=>{if(!item||typeof item.src!=='string'||!/^https?:\/\//i.test(item.src)||!['subtitles','captions','descriptions','chapters','metadata'].includes(item.kind||'subtitles'))return;const track=document.createElement('track');track.kind=item.kind||'subtitles';track.src=item.src;track.label=typeof item.label==='string'?item.label:'Captions';track.srclang=typeof item.language==='string'?item.language:'';track.default=index===0&&['subtitles','captions'].includes(track.kind);media.append(track);});}}media.addEventListener('error',()=>{const message=document.createElement('p');message.textContent='Could not load this media. Check its source or format.';media.remove();document.body.append(message);parent.postMessage({bruhsMedia:'error'},'*');},{once:true});media.src=data.src;media.addEventListener('load',()=>{if(tag==='img')parent.postMessage({bruhsMedia:'size',width:media.naturalWidth,height:media.naturalHeight},'*')},{once:true});document.body.append(media);}catch{document.body.textContent='Could not display this media. Check its source or format.';}}
</script></body></html>"#;

/// Only this trusted shell runs at the API origin. Assistant HTML always
/// runs in an opaque-origin child, with no app bridge or resource fetching.
pub const INTERACTIVE_HOST: &str = concat!(
    r#"<!doctype html><html><head><meta charset="utf-8"><meta name="referrer" content="no-referrer"><style>:root{color-scheme:dark light}html,body{margin:0;width:100%;height:100%;overflow:hidden}iframe{border:0;width:100%;height:100%}</style></head><body><script>"#,
    include_str!("rich_response/host-runtime.js"),
    r#"if(location.hash.length>1){try{const decoded=decodeURIComponent(location.hash.slice(1));const payload=decoded.trimStart().startsWith('{')?JSON.parse(decoded):{html:decoded,nonce:''};window.bruhsCreateWidget(payload);}catch{document.body.textContent='Could not open this interactive response. Reload it or ask the agent to regenerate it.';}}</script></body></html>"#
);

/// Kept short because this is appended to every provider's developer context.
pub const RESPONSE_INSTRUCTIONS: &str = r#"Rich replies: embed images with ![description](path-or-https-url). Embed video, audio, or PDF with a bruhs-media fenced JSON block: {"kind":"video|audio|pdf|image","src":"project-relative-or-absolute-file-path-or-https-url","title":"description"}. Local media must be inside the current project/worktree or Bruhs attachments. For interactive charts, forms, simulations, or widgets, use a bruhs-html fenced block (optional title after the language) containing self-contained HTML, CSS, and inline JavaScript. Always close the fence. It runs in a sandbox: no parent/app access, fetches, external resources, popups, or server submissions; bundle data and code inline. External media is shown after the user clicks Load. Ordinary html code fences remain source code. These are display features, not image/video generation tools; never claim to have generated media without creating it."#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_media_rewrite_preserves_fence_boundaries_and_examples() {
        let bindings = std::collections::BTreeMap::from([
            ("output.png".into(), "/saved/immutable.png".into()),
            ("poster.png".into(), "/saved/poster.png".into()),
        ]);
        for newline in ["\n", "\r\n"] {
            let input=format!("```bruhs-media{newline}{{\"kind\":\"video\",\"src\":\"output.png\",\"poster\":\"poster.png\"}}{newline}```{newline}Following prose{newline}");
            let output = rewrite_response_media(&input, &bindings);
            let blocks = response_blocks(&output, false);
            assert!(
                matches!(&blocks[0],ResponseBlock::Media(media)if media.src=="/saved/immutable.png"&&media.poster.as_deref()==Some("/saved/poster.png"))
            );
            assert!(
                matches!(blocks.last(),Some(ResponseBlock::Markdown(text))if text.contains("Following prose"))
            );
        }
        let example = "```markdown\n![Example](output.png)\n```";
        assert_eq!(rewrite_response_media(example, &bindings), example);
        let table = "| Photo |\n| --- |\n| ![Photo](output.png) |";
        let output = rewrite_response_media(table, &bindings);
        assert!(output.contains("| ![Photo](</saved/immutable.png>) |"));
    }

    #[test]
    fn closed_interactive_fences_render_without_running_partial_scripts() {
        let open =
            "Before\n\n```bruhs-html\n<button onclick=\"this.textContent='Done'\">Try</button>\n";
        assert_eq!(
            response_blocks(open, true),
            vec![ResponseBlock::Markdown(open.into())]
        );
        let closed = format!("{open}```\n\nAfter");
        let blocks = response_blocks(&closed, false);
        assert!(
            matches!(&blocks[1], ResponseBlock::Html(block) if block.html.contains("onclick="))
        );
        assert_eq!(
            blocks.last(),
            Some(&ResponseBlock::Markdown("\n\nAfter".into()))
        );
    }

    #[test]
    fn normal_code_examples_and_incomplete_fences_never_execute() {
        for source in [
            "```html\n<button onclick=\"alert(1)\">Example</button>\n```",
            "````text\n```bruhs-html\n<script>run()</script>\n```\n````",
            "```bruhs-html\n<script>run()</script>",
            "~~~bruhs-html\n<script>run()</script>\n~~",
            "```bruhs-html\n<script>run()</script>\n> ```",
            "```bruhs-html\n<script>run()</script>\n    ```",
            "```bruhs-html\n<script>run()</script>\n\t```",
            "```bruhs-html\n<script>run()</script>\n \t```",
            "```bruhs-html\n<script>run()</script>\n\u{a0}```",
        ] {
            for streaming in [true, false] {
                assert_eq!(
                    response_blocks(source, streaming),
                    vec![ResponseBlock::Markdown(source.into())],
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn explicit_media_and_markdown_media_keep_order_and_descriptions() {
        let source = "Before\n\n![Photo](output/photo.png)\n\n[Watch demo](output/demo.mp4)\n\n```bruhs-media\n{\"kind\":\"audio\",\"src\":\"audio.wav\",\"title\":\"Listen\"}\n```\n\nAfter";
        let blocks = response_blocks(source, false);
        let media: Vec<_> = blocks
            .iter()
            .filter_map(|b| {
                if let ResponseBlock::Media(m) = b {
                    Some((m.kind, m.src.as_str(), m.title.as_str()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            media,
            vec![
                (MediaKind::Image, "output/photo.png", "Photo"),
                (MediaKind::Video, "output/demo.mp4", "Watch demo"),
                (MediaKind::Audio, "audio.wav", "Listen")
            ]
        );
        assert_eq!(
            blocks.first(),
            Some(&ResponseBlock::Markdown("Before\n\n".into()))
        );
        assert!(
            matches!(blocks.last(), Some(ResponseBlock::Markdown(text)) if text.ends_with("After"))
        );
    }

    #[test]
    fn invalid_media_is_visible_and_cannot_become_executable_content() {
        for json in [
            "{\"kind\":\"image\",\"src\":\"javascript:alert(1)\"}",
            "{\"kind\":\"image\",\"src\":\"data:text/html,<script>run()</script>\"}",
            "{\"kind\":\"video\",\"src\":\"\"}",
            "{\"kind\":\"iframe\",\"src\":\"https://example.com\"}",
            "not JSON",
        ] {
            assert!(matches!(
                response_blocks(&format!("```bruhs-media\n{json}\n```"), false).as_slice(),
                [ResponseBlock::Error { .. }]
            ));
        }
    }

    #[test]
    fn inline_links_code_and_images_do_not_eat_other_markdown() {
        let source = "`![not an image](x.png)`\n[Guide](docs.md)";
        assert_eq!(
            response_blocks(source, false),
            vec![ResponseBlock::Markdown(source.into())]
        );
    }

    #[test]
    fn linked_images_and_raw_html_cannot_bypass_the_media_gate() {
        let source = "[![Preview](https://example.com/image.png)](https://example.com)";
        assert!(
            matches!(response_blocks(source, false).as_slice(), [ResponseBlock::Media(media)] if media.src == "https://example.com/image.png")
        );
        let source = "Before <img src=\"https://example.com/tracker.png\"> after\n\n```html\n<img src=\"example.png\">\n```";
        let safe = markdown_without_raw_html(source);
        assert!(!safe.contains("tracker.png"));
        assert!(safe.contains("<img src=\"example.png\">"));
        assert!(safe.starts_with("Before  after"));
    }

    #[test]
    fn nested_media_preserves_its_markdown_container() {
        for source in [
            "| Label | Photo |\n| --- | --- |\n| Before | ![Photo](https://example.com/p.png) |",
            "**before ![Photo](p.png) after**",
            "**before\n![Photo](p.png)\nafter**",
            "- ![Photo](p.png)",
            "> ![Photo](p.png)",
        ] {
            assert_eq!(
                response_blocks(source, false),
                vec![ResponseBlock::Markdown(source.into())]
            );
            assert!(!markdown_without_raw_html(source).contains("!["));
        }
    }

    #[test]
    fn reference_links_still_resolve_across_a_media_block() {
        let source =
            "[Guide][guide]\n\n![Photo](photo.png)\n\nTail\n\n[guide]: /docs \"Documentation\"";
        for block in response_blocks(source, false) {
            if let ResponseBlock::Markdown(text) = block {
                if text.contains("[Guide][guide]") {
                    assert!(Parser::new(&text).any(|event| matches!(event, Event::Start(Tag::Link {dest_url, ..}) if dest_url.as_ref() == "/docs")));
                }
            }
        }
    }

    #[test]
    fn rich_blocks_remain_equal_as_trailing_tokens_arrive() {
        let initial = "```bruhs-html Counter\n<button>Count</button>\n```";
        let blocks = response_blocks(initial, true);
        let extended = response_blocks(&format!("{initial}\n\nMore text"), true);
        assert_eq!(blocks.first(), extended.first());
        assert!(matches!(&blocks[0], ResponseBlock::Html(block) if block.title == "Counter"));
    }

    #[test]
    fn media_paths_are_encoded_once_and_bound_to_the_callers_project() {
        assert_eq!(media_url("http://localhost:3001/", "file:///tmp/my%20photo.png", Some("/work/tree & branch")), "http://localhost:3001/api/agent/media?path=%2Ftmp%2Fmy%20photo%2Epng&project=%2Fwork%2Ftree%20%26%20branch");
        assert_eq!(
            media_url("", "https://example.com/image.png", None),
            "https://example.com/image.png"
        );
        assert_eq!(
            media_url("", "//example.com/video.mp4", None),
            "https://example.com/video.mp4"
        );
        assert!(media_is_remote("HTTPS://example.com/image.png"));
        assert!(!media_is_remote("/output/image.png"));
    }

    #[test]
    fn interactive_documents_enforce_policy_before_untrusted_markup() {
        let html = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src *\"><script>window.parent.document.body.remove()</script>";
        let doc = interactive_document(html);
        assert!(doc.find(INTERACTIVE_CSP).unwrap() < doc.find(html).unwrap());
        assert!(INTERACTIVE_CSP.contains("connect-src 'none'"));
        assert!(INTERACTIVE_HOST.contains("'sandbox', 'allow-scripts'"));
        assert!(!INTERACTIVE_HOST.contains("allow-same-origin"));
        let url = interactive_url("http://localhost:3001", html);
        let (route, fragment) = url.split_once('#').unwrap();
        assert_eq!(route, "http://localhost:3001/api/agent/rich-response");
        assert_eq!(percent_decode_str(fragment).decode_utf8().unwrap(), doc);
    }
}
