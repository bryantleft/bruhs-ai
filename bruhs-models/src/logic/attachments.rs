//! File attachments and the read-only file viewer.
//!
//! Limits match the current T3 composer: 100 files, 10 MiB per image, 80 MiB
//! of images in one message, and 50 MiB for every other file. HEIC and HEIF
//! are rejected here. Bruhs does not convert or resize photos.

use serde::{Deserialize, Serialize};

/// Files one message may attach.
pub const MAX_FILES_PER_MESSAGE: usize = 100;
/// One image, in bytes.
pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
/// Every image in one message, in bytes.
pub const MAX_IMAGE_TOTAL_BYTES: u64 = 80 * 1024 * 1024;
/// Any other file, including video, in bytes.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
/// Text and HTML previews stop after this many bytes.
pub const TEXT_PREVIEW_BYTES: usize = 256 * 1024;

pub const TOO_MANY_FILES: &str = "A message can attach up to 100 files.";
pub const IMAGE_TOO_LARGE: &str = "This image is larger than 10 MiB.";
pub const IMAGE_TOTAL_TOO_LARGE: &str = "Images in this message are larger than 80 MiB.";
pub const FILE_TOO_LARGE: &str = "This file is larger than 50 MiB.";
pub const EMPTY_FILE: &str = "This file is empty.";
pub const EMPTY_NAME: &str = "This file has no name.";
pub const HEIC_NOT_CONVERTED: &str = "HEIC and HEIF photos are not converted here.";
pub const UPLOAD_DID_NOT_FINISH: &str = "This upload did not finish.";
pub const SEND_WAITS: &str = "Send waits until uploads finish or a failed one is removed.";
pub const COULD_NOT_READ: &str = "Could not read that file.";

/// How the file viewer presents a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Image,
    Audio,
    Video,
    Html,
    Pdf,
    Text,
    Other,
}

/// Whether an upload still blocks send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadPhase {
    Pending,
    Ready,
    Failed,
}

/// A file the server or desktop already wrote for the agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadedFile {
    pub name: String,
    pub path: String,
}

/// Text, HTML, or a path-only preview. Media is the raw response body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenedText {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    pub mime: String,
    pub truncated: bool,
    pub text: String,
}

/// A file link the viewer can open. `source` asks an HTML file to show markup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewerLink {
    pub path: String,
    pub source: bool,
}

/// Admit one file into a message that already holds `existing_count` admitted
/// files and `existing_image_bytes` of admitted images.
pub fn admit(
    existing_count: usize,
    existing_image_bytes: u64,
    name: &str,
    bytes: u64,
) -> Result<FileKind, &'static str> {
    if existing_count >= MAX_FILES_PER_MESSAGE {
        return Err(TOO_MANY_FILES);
    }
    if upload_label(name).is_empty() {
        return Err(EMPTY_NAME);
    }
    if bytes == 0 {
        return Err(EMPTY_FILE);
    }
    if matches!(file_extension(name).as_deref(), Some("heic" | "heif")) {
        return Err(HEIC_NOT_CONVERTED);
    }
    let kind = kind_for_name(name);
    if kind == FileKind::Image {
        if bytes > MAX_IMAGE_BYTES {
            return Err(IMAGE_TOO_LARGE);
        }
        if existing_image_bytes.saturating_add(bytes) > MAX_IMAGE_TOTAL_BYTES {
            return Err(IMAGE_TOTAL_TOO_LARGE);
        }
    } else if bytes > MAX_FILE_BYTES {
        return Err(FILE_TOO_LARGE);
    }
    Ok(kind)
}

/// `true` when any upload is still pending or failed. An empty list can send.
pub fn send_blocked(phases: &[UploadPhase]) -> bool {
    phases.iter().any(|phase| *phase != UploadPhase::Ready)
}

/// Kind from the filename extension. `#` and `?` stay in the name; the
/// extension is the alphanumeric run after the last dot of the last path
/// component. A leading-dot name such as `.gitignore` has none.
pub fn kind_for_name(name: &str) -> FileKind {
    match file_extension(name).as_deref() {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "avif" | "ico" | "svg") => {
            FileKind::Image
        }
        Some("mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "weba" | "aif" | "aiff") => {
            FileKind::Audio
        }
        Some("mp4" | "webm" | "mov" | "mkv" | "avi" | "m4v" | "ogv") => FileKind::Video,
        Some("html" | "htm") => FileKind::Html,
        Some("pdf") => FileKind::Pdf,
        Some(
            "txt" | "md" | "markdown" | "json" | "csv" | "tsv" | "rs" | "toml" | "css" | "js"
            | "ts" | "tsx" | "jsx" | "py" | "go" | "yaml" | "yml" | "log" | "sh" | "xml" | "c"
            | "h" | "sql" | "ini" | "rb" | "php" | "java",
        ) => FileKind::Text,
        _ => FileKind::Other,
    }
}

/// Lowercase alphanumeric extension, if the last path component has one.
pub fn file_extension(name: &str) -> Option<String> {
    let component = last_component(name);
    let (stem, rest) = component.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let mut ext = String::new();
    for ch in rest.chars() {
        if ch.is_ascii_alphanumeric() {
            ext.push(ch.to_ascii_lowercase());
        } else {
            break;
        }
    }
    if ext.is_empty() {
        None
    } else {
        Some(ext)
    }
}

/// The name shown on a tile and written next to the path.
pub fn upload_label(name: &str) -> String {
    last_component(name).trim().to_string()
}

/// Disk name under the attachment directory. `unique` is a caller-chosen id.
pub fn upload_disk_name(unique: &str, name: &str) -> String {
    format!("{unique}-{}", safe_upload_filename(name))
}

/// A single path segment that cannot escape the attachment directory.
pub fn safe_upload_filename(name: &str) -> String {
    let mut out = String::new();
    for ch in last_component(name).chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else if ch.is_whitespace() {
            out.push('_');
        }
    }
    let out = out.trim_matches('.').to_string();
    if out.is_empty() {
        "file".to_string()
    } else {
        out
    }
}

pub fn mime_for(kind: FileKind, name: &str) -> &'static str {
    match kind {
        FileKind::Image => match file_extension(name).as_deref() {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("gif") => "image/gif",
            Some("webp") => "image/webp",
            Some("bmp") => "image/bmp",
            Some("avif") => "image/avif",
            Some("svg") => "image/svg+xml",
            Some("ico") => "image/x-icon",
            _ => "image/png",
        },
        FileKind::Audio => match file_extension(name).as_deref() {
            Some("wav") => "audio/wav",
            Some("ogg" | "opus") => "audio/ogg",
            Some("m4a") => "audio/mp4",
            Some("aac") => "audio/aac",
            Some("flac") => "audio/flac",
            _ => "audio/mpeg",
        },
        FileKind::Video => match file_extension(name).as_deref() {
            Some("webm") => "video/webm",
            Some("mov") => "video/quicktime",
            Some("ogv") => "video/ogg",
            _ => "video/mp4",
        },
        FileKind::Html => "text/html",
        FileKind::Pdf => "application/pdf",
        FileKind::Text => "text/plain",
        FileKind::Other => "application/octet-stream",
    }
}

/// A link the viewer should open. Web, mail, and script URLs stay in the page.
pub fn viewer_target(href: &str) -> Option<ViewerLink> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') || href.starts_with("//") {
        return None;
    }
    if let Some(scheme) = scheme_of(href) {
        if scheme != "file" {
            return None;
        }
    }
    let decoded = percent_decode(&strip_file_scheme(href));
    let (path, source) = split_fragment(&decoded);
    if path.is_empty() {
        None
    } else {
        Some(ViewerLink { path, source })
    }
}

/// The provider prompt. Paths are named after the visible text.
pub fn append_uploaded_files(prompt: &str, files: &[UploadedFile]) -> String {
    if files.is_empty() {
        return prompt.to_string();
    }
    let mut out = String::from(prompt);
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str("Read these attached files");
    for file in files {
        out.push('\n');
        out.push_str(&one_line(&file.name));
        out.push(' ');
        out.push_str(&one_line(&file.path));
    }
    out
}

fn last_component(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

fn one_line(value: &str) -> String {
    let mut out = String::new();
    for (index, word) in value.split_whitespace().enumerate() {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn scheme_of(href: &str) -> Option<String> {
    let end = href.find([':', '/', '?', '#'])?;
    if !href[end..].starts_with(':') {
        return None;
    }
    let scheme = &href[..end];
    if scheme.len() < 2 || !scheme.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

fn strip_file_scheme(href: &str) -> String {
    let Some(rest) = strip_prefix_ignore_ascii_case(href, "file:") else {
        return href.to_string();
    };
    let rest = rest.strip_prefix("//localhost").unwrap_or(rest);
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    rest.to_string()
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    if value.len() < prefix.len() {
        return None;
    }
    let (head, tail) = value.split_at(prefix.len());
    head.eq_ignore_ascii_case(prefix).then_some(tail)
}

/// A trailing `#fragment` with no slash is a link fragment. A `#` inside a
/// directory or a filename that has no extension yet stays in the path.
/// `L12` and `12` open HTML as source.
fn split_fragment(path: &str) -> (String, bool) {
    let Some((head, frag)) = path.rsplit_once('#') else {
        return (path.to_string(), false);
    };
    if frag.is_empty() || frag.contains(['/', '\\']) {
        return (path.to_string(), false);
    }
    let digits = frag.strip_prefix(['L', 'l']).unwrap_or(frag);
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return (head.to_string(), true);
    }
    let component = last_component(head);
    let has_ext = component.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && ext.starts_with(|ch: char| ch.is_ascii_alphanumeric())
    });
    if has_ext {
        (head.to_string(), false)
    } else {
        (path.to_string(), false)
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_ignore_hashes_questions_and_leading_dots() {
        assert_eq!(file_extension("dir#1/a.PDF"), Some("pdf".into()));
        assert_eq!(file_extension("report.pdf#1"), Some("pdf".into()));
        assert_eq!(file_extension("photo.jpeg?size=1"), Some("jpeg".into()));
        assert_eq!(file_extension(".gitignore"), None);
        assert_eq!(file_extension("notes"), None);
        assert_eq!(kind_for_name("dir#1/a.PDF"), FileKind::Pdf);
        assert_eq!(kind_for_name("page.HTML"), FileKind::Html);
        assert_eq!(kind_for_name("clip.MP3"), FileKind::Audio);
        assert_eq!(kind_for_name("tour.webm"), FileKind::Video);
        assert_eq!(kind_for_name("readme.md"), FileKind::Text);
        assert_eq!(kind_for_name(".gitignore"), FileKind::Other);
    }

    #[test]
    fn admit_enforces_count_and_size() {
        assert_eq!(admit(0, 0, "a.png", 1), Ok(FileKind::Image));
        assert_eq!(admit(0, 0, "a.png", MAX_IMAGE_BYTES), Ok(FileKind::Image));
        assert_eq!(
            admit(0, 0, "a.png", MAX_IMAGE_BYTES + 1),
            Err(IMAGE_TOO_LARGE)
        );
        assert_eq!(
            admit(0, MAX_IMAGE_TOTAL_BYTES, "b.png", 1),
            Err(IMAGE_TOTAL_TOO_LARGE)
        );
        assert_eq!(admit(0, 0, "clip.mp4", MAX_FILE_BYTES), Ok(FileKind::Video));
        assert_eq!(
            admit(0, 0, "clip.mp4", MAX_FILE_BYTES + 1),
            Err(FILE_TOO_LARGE)
        );
        assert_eq!(
            admit(MAX_FILES_PER_MESSAGE, 0, "a.txt", 1),
            Err(TOO_MANY_FILES)
        );
        assert_eq!(admit(0, 0, "empty.txt", 0), Err(EMPTY_FILE));
        assert_eq!(admit(0, 0, "   ", 1), Err(EMPTY_NAME));
        assert_eq!(admit(0, 0, "photo.HEIC", 10), Err(HEIC_NOT_CONVERTED));
        assert_eq!(admit(0, 0, "photo.heif", 10), Err(HEIC_NOT_CONVERTED));
    }

    #[test]
    fn send_waits_for_every_upload() {
        assert!(!send_blocked(&[]));
        assert!(!send_blocked(&[UploadPhase::Ready, UploadPhase::Ready]));
        assert!(send_blocked(&[UploadPhase::Ready, UploadPhase::Pending]));
        assert!(send_blocked(&[UploadPhase::Failed]));
    }

    #[test]
    fn viewer_opens_file_links_and_leaves_the_web_alone() {
        assert_eq!(viewer_target(""), None);
        assert_eq!(viewer_target("#top"), None);
        assert_eq!(viewer_target("https://example.com/a.md"), None);
        assert_eq!(viewer_target("javascript:alert(1)"), None);
        assert_eq!(viewer_target("//cdn.example/a.png"), None);
        assert_eq!(
            viewer_target("docs/a.md"),
            Some(ViewerLink {
                path: "docs/a.md".into(),
                source: false,
            })
        );
        assert_eq!(
            viewer_target("file:///Users/me/a.md"),
            Some(ViewerLink {
                path: "/Users/me/a.md".into(),
                source: false,
            })
        );
        assert_eq!(
            viewer_target("src/main.rs#L42"),
            Some(ViewerLink {
                path: "src/main.rs".into(),
                source: true,
            })
        );
        assert_eq!(
            viewer_target("page.html#section"),
            Some(ViewerLink {
                path: "page.html".into(),
                source: false,
            })
        );
        assert_eq!(
            viewer_target("dir#1/a.PDF"),
            Some(ViewerLink {
                path: "dir#1/a.PDF".into(),
                source: false,
            })
        );
        assert_eq!(
            viewer_target("weird#name.txt"),
            Some(ViewerLink {
                path: "weird#name.txt".into(),
                source: false,
            })
        );
    }

    #[test]
    fn attached_files_are_named_after_the_visible_prompt() {
        assert_eq!(append_uploaded_files("hello", &[]), "hello");
        let prompt = append_uploaded_files(
            "hello",
            &[UploadedFile {
                name: "shot.png".into(),
                path: "/tmp/shot.png".into(),
            }],
        );
        assert_eq!(
            prompt,
            "hello\n\nRead these attached files\nshot.png /tmp/shot.png"
        );
        assert!(upload_disk_name("abc", "../secret.png").ends_with("-secret.png"));
        assert!(!upload_disk_name("abc", "../secret.png").contains('/'));
    }
}
