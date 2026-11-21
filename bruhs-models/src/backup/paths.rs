//! Where things sit inside a backup and on disk: the archive's entry paths
//! (D9), what a backup leaves out (D4), and how a restore moves absolute
//! paths from the backup's computer to this one (D10, D11).
//!
//! Every relative path here uses `/` and is relative to its data root.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use super::{BackupOptions, PathRewrite, UNSAFE_ENTRY};

/// A backup holds at most this many tar entries: files and folders, plus
/// `keychain.json`, `web/part.bin` and `manifest.json`. A GNU long-name
/// header belongs to the entry after it and doesn't count (B Task 7 I1).
pub const MAX_ENTRIES: u64 = 1_000_000;
/// `manifest.json` is at most this many bytes. A restore holds it in
/// memory; a backup whose manifest would be larger isn't made (B Task 7
/// I1).
pub const MANIFEST_MAX_BYTES: u64 = 64 << 20;
/// All entry paths of one backup together are at most this many bytes:
/// files, folders and the parts beside the data, each counted once
/// without its long-name header. A restore holds every path it staged in
/// memory, so this bounds that work within the entry limit (final review
/// A-I1); a backup past it isn't made.
pub const MAX_TOTAL_PATH_BYTES: u64 = 2 * MANIFEST_MAX_BYTES;
/// `keychain.json` is at most this many bytes. A restore holds it in
/// memory; a backup whose items would be larger isn't made (B Task 7 N1).
pub const KEYCHAIN_MAX_BYTES: u64 = 4 << 20;
/// The folder a staged restore and a pre-restore set keep beside the data
/// (`meta/`). No top-level entry of either root may take its name, in any
/// case (B Task 7 N2).
pub const RESERVED_TOP: &str = "meta";
/// An entry path is at most this many bytes.
const MAX_PATH_BYTES: usize = 4096;
/// Each component of an entry path is at most this many bytes.
const MAX_COMPONENT_BYTES: usize = 255;
/// An entry path below `core/` or `desktop/` has at most this many
/// components (B Task 5 N1 ruling): the backup's walk holds one folder
/// handle a level, so a deeper tree would exhaust the server's open files.
pub const MAX_ENTRY_DEPTH: usize = 64;

/// Top-level entries of either root that never go in: machine-local state,
/// caches and the backups themselves.
pub const EXCLUDED_TOP: [&str; 9] = [
    "backups",
    "capture",
    "codex-lean-sessions",
    "codex-lean-sessions.lock",
    "logs",
    "models",
    "service",
    "updates",
    "worktrees",
];

/// Top-level entries the desktop app owns, and the text generators' store
/// that Bruhs keeps beside them. On a one-root computer they sit in the
/// shared root; on two roots, in the desktop root (D11, ruling F8).
pub const DESKTOP_ENTRIES: [&str; 7] = [
    "desktop-state.json",
    "desktop-state.json.bak",
    "knowledge-attachments",
    "delivery",
    "diff-viewed.json",
    "keybindings.json",
    "text-generators.sqlite3",
];

/// Top-level entries of the core root that Bruhs keeps in
/// `ProjectDirs::config_dir()`. Where that is a folder of its own (Linux),
/// a backup takes them from there as the core root's, a restore puts them
/// back there, and nothing else of it goes in (ruling F10 a). On macOS it
/// is the core root.
pub const CONFIG_ENTRIES: [&str; 2] = ["calendar", "secret-sources.json"];

/// Top-level folders that hold the owner's own files, which a restore
/// copies byte for byte and never rewrites.
pub const CONTENT_FOLDERS: [&str; 4] = [
    "attachments",
    "audio",
    "rich-responses",
    "knowledge-attachments",
];

/// Other top-level names Bruhs keeps in its roots. With [`EXCLUDED_TOP`],
/// [`CONTENT_FOLDERS`], [`DESKTOP_ENTRIES`] and [`CONFIG_ENTRIES`], an
/// entry whose first name matches one of these only when case is ignored
/// is refused (I1 ruling): on a case-insensitive volume it would land in,
/// or replace, the real one.
const KNOWN_TOP: [&str; 8] = [
    "server",
    "credentials",
    "connections",
    "workbench",
    "pith-mcp.json",
    "worktree-meter.json",
    "voice-dictionaries.sqlite3",
    "fast-worktree-checkout",
];
/// Lock file ending a backup leaves out of Bruhs' own folders. Owner
/// content keeps it (D4 ruling).
const LOCK_SUFFIX: &str = ".lock";
/// SQLite side files, left out of Bruhs' own databases. Owner content and a
/// bruh's browser profile, whose databases are copied raw (F15), keep them:
/// without its `-wal` or hot `-journal` a raw copy loses committed data.
const SQLITE_SIDE_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];
/// Media file extensions under `audio/` that the audio option covers
/// (ruling F9: transcripts, centroids and OCR always go in).
const AUDIO_EXTENSIONS: [&str; 9] = [
    "wav", "m4a", "mp3", "ogg", "opus", "flac", "caf", "aac", "aiff",
];
/// Chromium cache folders inside a bruh's browser profile, never backed up.
const BROWSER_CACHES: [&str; 6] = [
    "Cache",
    "Code Cache",
    "GPUCache",
    "ShaderCache",
    "GrShaderCache",
    "DawnCache",
];
/// Absolute paths that are never named as outside the home.
const NEVER_OUTSIDE: [&str; 5] = ["/tmp", "/dev", "/proc", "/private/tmp", "/var/folders"];
/// At most this many outside paths are collected.
const MAX_OUTSIDE: usize = 20;

/// Where an archive entry goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryPath {
    Manifest,
    Keychain,
    WebPart,
    /// A path under the core root.
    Core(String),
    /// A path under the desktop root.
    Desktop(String),
}

/// Checks an archive entry path (D9): `manifest.json`, `keychain.json`,
/// `web/part.bin`, or a relative path under `core/` or `desktop/` whose
/// components are all normal names. Refuses with [`UNSAFE_ENTRY`]:
/// - an empty, `.` or `..` component (so a doubled or trailing `/` too), a
///   leading `/`, a component over 255 bytes, a path over 4096 bytes and
///   more than [`MAX_ENTRY_DEPTH`] components below the root's name;
/// - a backslash, NUL or any control character, and the invisible format,
///   bidi and line or paragraph separator characters that can disguise a
///   name;
/// - a first name a backup never holds ([`EXCLUDED_TOP`], in any case), or
///   one that matches a known top-level name only when case is ignored
///   (`Credentials`, `SERVICE`), I1 ruling;
/// - [`RESERVED_TOP`] as the first name, folded as a volume would fold it
///   (B Task 7 N2).
pub fn safe_entry_path(path: &str) -> Result<EntryPath, &'static str> {
    if path.len() > MAX_PATH_BYTES
        || path
            .chars()
            .any(|c| c.is_control() || c == '\\' || invisible(c))
    {
        return Err(UNSAFE_ENTRY);
    }
    match path {
        "manifest.json" => return Ok(EntryPath::Manifest),
        "keychain.json" => return Ok(EntryPath::Keychain),
        "web/part.bin" => return Ok(EntryPath::WebPart),
        _ => {}
    }
    let (rest, entry): (&str, fn(String) -> EntryPath) =
        if let Some(rest) = path.strip_prefix("core/") {
            (rest, EntryPath::Core)
        } else if let Some(rest) = path.strip_prefix("desktop/") {
            (rest, EntryPath::Desktop)
        } else {
            return Err(UNSAFE_ENTRY);
        };
    let normal = |component: &str| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && component.len() <= MAX_COMPONENT_BYTES
    };
    let first = rest.split('/').next().unwrap_or_default();
    if rest.split('/').count() <= MAX_ENTRY_DEPTH
        && rest.split('/').all(normal)
        && !refused_top(first)
    {
        Ok(entry(rest.to_owned()))
    } else {
        Err(UNSAFE_ENTRY)
    }
}

/// The name two entry paths share on a volume that ignores case and
/// Unicode normalization (APFS, HFS+): lower case, then NFC. A restore
/// refuses two different paths with one key (`NAMES_CLASH`), since one
/// would land on the other.
pub fn folded_entry_key(path: &str) -> String {
    path.nfd().flat_map(char::to_lowercase).nfc().collect()
}

/// Whether an entry's first name is never restored: an excluded top-level
/// name in any case, or a case variant of a known one.
fn refused_top(first: &str) -> bool {
    let variant = |known: &&str| first.eq_ignore_ascii_case(known) && first != *known;
    EXCLUDED_TOP
        .iter()
        .any(|excluded| first.eq_ignore_ascii_case(excluded))
        || KNOWN_TOP.iter().any(variant)
        || CONTENT_FOLDERS.iter().any(variant)
        || DESKTOP_ENTRIES.iter().any(variant)
        || CONFIG_ENTRIES.iter().any(variant)
        || folded_entry_key(first) == RESERVED_TOP
}

/// The format (Cf), line separator (Zl) and paragraph separator (Zp)
/// characters: soft hyphen, zero-width and joiner characters, bidi marks,
/// embeddings, overrides and isolates, the byte order mark, interlinear
/// annotation and tag characters, and the like.
fn invisible(c: char) -> bool {
    matches!(
        u32::from(c),
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x2028..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

/// Why a path is left out of a backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Excluded {
    /// It goes in.
    No,
    /// It never goes in.
    Always,
    /// A meeting audio file: in only with [`BackupOptions::audio`].
    Audio,
    /// A bruh's browser profile: in only with
    /// [`BackupOptions::browser_profiles`].
    Browser,
}

/// What D4 says about `rel` (relative to its root, `/`-separated). Lock
/// files and SQLite side files are left out of Bruhs' own folders, but
/// owner content (the content folders and each bruh's `work` folder) keeps
/// them, so a project's `Cargo.lock` or an owner database's journal comes
/// back whole; a bruh's browser profile keeps its SQLite side files. Always
/// out: the broker files under `credentials/brokers` (each holds a live
/// bearer token), a bruh's `browser.pid` and `browser-cache`, and the
/// profile's cache folders (I2 ruling). By option: its `browser` profile
/// and the media files under `audio/`. Everything else goes in, including
/// folders a later Bruhs adds.
pub fn excluded(rel: &str, is_dir: bool) -> Excluded {
    let parts: Vec<&str> = rel.split('/').collect();
    if EXCLUDED_TOP.contains(&parts[0]) {
        return Excluded::Always;
    }
    if parts[0] == "credentials" && parts.get(1) == Some(&"brokers") {
        return Excluded::Always;
    }
    let name = parts.last().copied().unwrap_or_default();
    let in_browser = bruh_folder(&parts) == Some("browser");
    if !is_dir
        && !owner_content(&parts)
        && (name.ends_with(LOCK_SUFFIX)
            || (!in_browser
                && SQLITE_SIDE_SUFFIXES
                    .iter()
                    .any(|suffix| name.ends_with(suffix))))
    {
        return Excluded::Always;
    }
    if let Some(folder) = bruh_folder(&parts) {
        match folder {
            "browser-cache" | "browser.pid" => return Excluded::Always,
            "browser" if parts[4..].iter().any(|part| BROWSER_CACHES.contains(part)) => {
                return Excluded::Always
            }
            "browser" => return Excluded::Browser,
            _ => {}
        }
    }
    if parts[0] == "audio" && !is_dir && parts.len() > 1 && audio_media(name) {
        return Excluded::Audio;
    }
    Excluded::No
}

/// Whether `rel` goes into a backup made with `options`.
pub fn included(rel: &str, is_dir: bool, options: &BackupOptions) -> bool {
    match excluded(rel, is_dir) {
        Excluded::No => true,
        Excluded::Always => false,
        Excluded::Audio => options.audio,
        Excluded::Browser => options.browser_profiles,
    }
}

/// Whether a restore may rewrite paths inside the staged file `rel` (D10).
/// Not under `credentials/` (re-keyed on its own, D12), the content
/// folders, or a bruh's `work`, `browser` and `browser-cache` folders, all
/// of which hold the owner's own files.
pub fn rewritable(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    parts[0] != "credentials"
        && !owner_content(&parts)
        && !matches!(bruh_folder(&parts), Some("browser" | "browser-cache"))
}

/// Whether a path sits in the owner's own files: a content folder or a
/// bruh's `work` folder.
fn owner_content(parts: &[&str]) -> bool {
    CONTENT_FOLDERS.contains(&parts[0]) || bruh_folder(parts) == Some("work")
}

/// Whether a top-level entry belongs to the desktop app.
pub fn desktop_entry(top: &str) -> bool {
    DESKTOP_ENTRIES.contains(&top)
}

/// The folder right under a bruh's tree, for a path inside
/// `server/bruhs/<id>/<folder>`.
fn bruh_folder<'a>(parts: &[&'a str]) -> Option<&'a str> {
    match parts {
        ["server", "bruhs", _, folder, ..] => Some(folder),
        _ => None,
    }
}

fn audio_media(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, extension)| {
        AUDIO_EXTENSIONS
            .iter()
            .any(|known| known.eq_ignore_ascii_case(extension))
    })
}

/// A computer's home and data roots, as a backup's manifest names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootsInfo {
    pub home: String,
    pub core: String,
    pub desktop: String,
}

impl RootsInfo {
    /// Whether the core and desktop roots are one folder (Linux).
    pub fn same(&self) -> bool {
        self.core == self.desktop
    }
}

/// The prefixes a restore from `from` onto `to` rewrites (D10, D11, ruling
/// F8), longest `from` first. A one-root backup restored onto two roots
/// sends each of [`DESKTOP_ENTRIES`] to the desktop root and the rest to
/// the core root; a two-root backup onto one root sends both roots there.
/// Equal pairs, a side that isn't absolute or holds a control character,
/// and a second rule for the same `from` are dropped, so the same computer
/// gets no rules.
pub fn rewrite_rules(from: &RootsInfo, to: &RootsInfo) -> Vec<PathRewrite> {
    let mut pairs = Vec::new();
    if from.same() {
        if !to.same() {
            for name in DESKTOP_ENTRIES {
                pairs.push((join(&from.core, name), join(&to.desktop, name)));
            }
        }
    } else if to.same() {
        pairs.push((from.desktop.clone(), to.core.clone()));
    } else {
        pairs.push((from.desktop.clone(), to.desktop.clone()));
    }
    pairs.push((from.core.clone(), to.core.clone()));
    pairs.push((from.home.clone(), to.home.clone()));

    let mut rules: Vec<PathRewrite> = Vec::new();
    for (from, to) in pairs {
        let (from, to) = (trim_slash(&from), trim_slash(&to));
        if !absolute(from) || !absolute(to) || from == to {
            continue;
        }
        if rules.iter().all(|rule| rule.from != from) {
            rules.push(PathRewrite {
                from: from.to_owned(),
                to: to.to_owned(),
            });
        }
    }
    rules.sort_by_key(|rule| std::cmp::Reverse(rule.from.len()));
    rules
}

/// A rule side a restore trusts: absolute, with no control character.
fn absolute(path: &str) -> bool {
    path.starts_with('/') && !path.chars().any(char::is_control)
}

fn join(root: &str, name: &str) -> String {
    format!("{}/{name}", trim_slash(root))
}

fn trim_slash(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        path
    } else {
        trimmed
    }
}

/// Whether `text` is `prefix` or sits under it.
fn under(text: &str, prefix: &str) -> bool {
    text.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// `text` with the first matching rule's prefix replaced, or `None` when no
/// rule's `from` is the whole of `text` or a leading folder of it.
pub fn rewrite_path(text: &str, rules: &[PathRewrite]) -> Option<String> {
    let rule = rules.iter().find(|rule| under(text, &rule.from))?;
    Some(format!("{}{}", rule.to, &text[rule.from.len()..]))
}

/// Rewrites every string value and object key in `value` and returns how
/// many changed. A rewritten key that lands on an existing key replaces it.
pub fn rewrite_json(value: &mut serde_json::Value, rules: &[PathRewrite]) -> usize {
    match value {
        serde_json::Value::String(text) => match rewrite_path(text, rules) {
            Some(rewritten) => {
                *text = rewritten;
                1
            }
            None => 0,
        },
        serde_json::Value::Array(items) => {
            items.iter_mut().map(|item| rewrite_json(item, rules)).sum()
        }
        serde_json::Value::Object(map) => {
            let mut changed = 0;
            let entries = std::mem::take(map);
            for (key, mut item) in entries {
                changed += rewrite_json(&mut item, rules);
                let key = match rewrite_path(&key, rules) {
                    Some(rewritten) => {
                        changed += 1;
                        rewritten
                    }
                    None => key,
                };
                map.insert(key, item);
            }
            changed
        }
        _ => 0,
    }
}

/// The JSON fields and SQLite columns that hold absolute paths, the only
/// places [`collect_outside`] looks (D10 ruling): project paths (threads,
/// work items, bruhs and the credential catalog), agent working folders,
/// meeting and capture audio, and database connection files. A bare `path`
/// isn't one, on purpose: it also names API routes such as `/api/events`,
/// so the worktree and whiteboard `path` fields go unflagged.
pub const PATH_FIELDS: [&str; 19] = [
    "project_path",
    "projectPath",
    "cwd",
    "working_path",
    "workingPath",
    "workspace_path",
    "workspacePath",
    "workspace_root",
    "workspaceRoot",
    "repo_root",
    "repoRoot",
    "worktree_path",
    "worktreePath",
    "connection_path",
    "connectionPath",
    "audio_path",
    "audioPath",
    "wav_path",
    "wavPath",
];

/// Whether a JSON field or SQLite column holds paths.
pub fn path_field(name: &str) -> bool {
    PATH_FIELDS.contains(&name)
}

/// Adds to `into` the absolute paths held in [`PATH_FIELDS`] anywhere in
/// `value` (a field's string, or every string inside its array or object
/// value) that sit under no rule's `from` or `to`, by their first two
/// components (`/Volumes/Work`), up to 20 in all (D10). A path is a string
/// that starts with `/`, has at least two components, no newline and at
/// most 4096 bytes; one with a `.`, `..` or empty component (a single
/// trailing `/` aside) is named whole, since it may lead anywhere. Other
/// strings and object keys are never read.
/// Temporary and system folders are never named, and nothing is when there
/// are no rules, since then nothing moves.
pub fn collect_outside(
    value: &serde_json::Value,
    rules: &[PathRewrite],
    into: &mut BTreeSet<String>,
) {
    walk_outside(value, false, rules, into);
}

/// [`collect_outside`] for a value held in the field or SQLite column
/// `field`: a cell of a [`path_field`] column is read whole, any other
/// cell only through the path fields inside it.
pub fn collect_outside_in(
    field: &str,
    value: &serde_json::Value,
    rules: &[PathRewrite],
    into: &mut BTreeSet<String>,
) {
    walk_outside(value, path_field(field), rules, into);
}

fn walk_outside(
    value: &serde_json::Value,
    in_field: bool,
    rules: &[PathRewrite],
    into: &mut BTreeSet<String>,
) {
    if rules.is_empty() {
        return;
    }
    match value {
        serde_json::Value::String(text) if in_field => note_outside(text, rules, into),
        serde_json::Value::Array(items) => {
            for item in items {
                walk_outside(item, in_field, rules, into);
            }
        }
        serde_json::Value::Object(map) => {
            for (key, item) in map {
                walk_outside(item, in_field || path_field(key), rules, into);
            }
        }
        _ => {}
    }
}

fn note_outside(text: &str, rules: &[PathRewrite], into: &mut BTreeSet<String>) {
    if into.len() >= MAX_OUTSIDE
        || !text.starts_with('/')
        || text.len() > MAX_PATH_BYTES
        || text.contains('\n')
    {
        return;
    }
    let body = &text[1..];
    let parts: Vec<&str> = body.strip_suffix('/').unwrap_or(body).split('/').collect();
    let [first, second, ..] = parts[..] else {
        return;
    };
    // A `.`, `..` or empty component can lead anywhere, whatever prefix the
    // text starts with (`/home/ada/../../Volumes/x`), so it is flagged whole.
    if parts.iter().any(|part| matches!(*part, "" | "." | "..")) {
        into.insert(text.to_owned());
        return;
    }
    if NEVER_OUTSIDE.iter().any(|prefix| under(text, prefix))
        || rules
            .iter()
            .any(|rule| under(text, &rule.from) || under(text, &rule.to))
    {
        return;
    }
    into.insert(format!("/{first}/{second}"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn entry_paths_never_leave_the_tree() {
        assert_eq!(
            safe_entry_path("core/server/reactive.sqlite3"),
            Ok(EntryPath::Core("server/reactive.sqlite3".into()))
        );
        assert_eq!(safe_entry_path("manifest.json"), Ok(EntryPath::Manifest));
        assert_eq!(safe_entry_path("keychain.json"), Ok(EntryPath::Keychain));
        assert_eq!(safe_entry_path("web/part.bin"), Ok(EntryPath::WebPart));
        assert_eq!(
            safe_entry_path("desktop/knowledge-attachments/k.pdf"),
            Ok(EntryPath::Desktop("knowledge-attachments/k.pdf".into()))
        );
        let long = format!("core/{}", "a".repeat(256));
        let too_long = format!("core/{}", ["a"; 2050].join("/"));
        for bad in [
            "../x",
            "/etc/passwd",
            "core/../../x",
            "core/./a",
            "core//a",
            "core",
            "core/",
            "core/a/",
            "other/a",
            "core/a\\..\\b",
            "core/a\0b",
            "core/a\nb",
            "web/other.bin",
            "C:/x",
            "",
            long.as_str(),
            too_long.as_str(),
        ] {
            assert_eq!(safe_entry_path(bad), Err(UNSAFE_ENTRY), "{bad:?}");
        }
        assert!(safe_entry_path(&format!("core/{}", "a".repeat(255))).is_ok());
    }

    /// B Task 7 N2: `meta` is the staging's own, at the top of either root.
    #[test]
    fn the_staging_meta_name_is_reserved_at_the_top() {
        for bad in ["core/meta", "core/meta/x", "desktop/Meta/x", "core/META"] {
            assert_eq!(safe_entry_path(bad), Err(UNSAFE_ENTRY), "{bad}");
        }
        for good in ["core/server/meta/x", "core/metadata", "desktop/meta.json"] {
            assert!(safe_entry_path(good).is_ok(), "{good}");
        }
    }

    #[test]
    fn folded_keys_match_names_one_volume_would_merge() {
        let key = folded_entry_key;
        assert_eq!(key("core/Notes.txt"), key("core/notes.txt"));
        assert_eq!(key("core/caf\u{e9}"), key("core/cafe\u{301}"));
        assert_eq!(key("core/CAF\u{c9}"), key("core/cafe\u{301}"));
        assert_ne!(key("core/notes.txt"), key("core/notes.md"));
        assert_ne!(key("core/a"), key("desktop/a"));
    }

    /// B Task 5 N1 ruling: at most 64 levels below the root's name.
    #[test]
    fn entry_paths_are_at_most_64_levels_deep() {
        let at = |levels: usize| vec!["a"; levels].join("/");
        for root in ["core", "desktop"] {
            assert!(safe_entry_path(&format!("{root}/{}", at(MAX_ENTRY_DEPTH))).is_ok());
            assert_eq!(
                safe_entry_path(&format!("{root}/{}", at(MAX_ENTRY_DEPTH + 1))),
                Err(UNSAFE_ENTRY)
            );
        }
    }

    /// I1 ruling and the invisible-character minor.
    #[test]
    fn entry_paths_refuse_excluded_tops_case_variants_and_invisible_names() {
        for bad in [
            "core/backups/staging/x",
            "core/capture/daemon-token",
            "core/Capture/x",
            "core/service/releases/x",
            "core/SERVICE",
            "core/Credentials/vault.json",
            "core/Backups",
            "core/codex-lean-sessions.lock",
            "desktop/logs/x.log",
            "core/Server/reactive.sqlite3",
            "desktop/Delivery/journal.json",
            "desktop/Desktop-State.json",
            "core/Secret-Sources.json",
            "core/Calendar/google/x",
            "desktop/Keybindings.json",
            "core/Voice-Dictionaries.sqlite3",
            "core/AUDIO/m1.wav",
            "core/a\u{202E}gpj.exe",
            "core/a\u{200B}",
            "core/a\u{2028}b",
            "core/a\u{2029}b",
            "core/\u{FEFF}server/x",
            "core/a\u{2066}b",
            "core/a\u{E0041}",
            "core/a\u{00AD}b",
        ] {
            assert_eq!(safe_entry_path(bad), Err(UNSAFE_ENTRY), "{bad:?}");
        }
        for good in [
            "core/server/reactive.sqlite3",
            "core/credentials/vault.json",
            "desktop/delivery/journal.json",
            "core/secret-sources.json",
            "core/calendar/google/x",
            "desktop/keybindings.json",
            "core/servers/x",
            "core/server/bruhs/a1/work/Service/x",
            "core/C:/x",
            "core/a\u{FF0F}b",
            "core/\u{2025}",
            "core/caf\u{E9}",
        ] {
            assert!(safe_entry_path(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn exclusions_follow_the_options() {
        let off = BackupOptions {
            keychain: true,
            audio: false,
            browser_profiles: false,
        };
        for rel in [
            "logs",
            "models/ggml.bin",
            "service/releases/x",
            "capture/daemon-token",
            "server/reactive.sqlite3.lock",
            "server/mail.sqlite3-wal",
            "server/bruhs/a1/browser-cache/x",
            "backups/staging/x",
        ] {
            assert!(!included(rel, false, &BackupOptions::default()), "{rel}");
        }
        assert!(
            !included("audio/m1/a.wav", false, &off)
                && included("audio/m1/a.wav", false, &BackupOptions::default())
        );
        let on = BackupOptions {
            browser_profiles: true,
            ..off
        };
        assert!(!included(
            "server/bruhs/a1/browser/Default/Cookies",
            false,
            &off
        ));
        assert!(included(
            "server/bruhs/a1/browser/Default/Cookies",
            false,
            &on
        ));
        assert!(!included(
            "server/bruhs/a1/browser/Default/Code Cache/js/x",
            false,
            &on
        ));
        assert!(
            included("server/bruhs/a1/work/notes.md", false, &off)
                && included("credentials/vault.json", false, &off)
        );
        assert!(
            !rewritable("credentials/catalog.json")
                && !rewritable("server/bruhs/a1/work/x.json")
                && rewritable("server/task-workspaces.json")
        );
    }

    /// D4 ruling: lock and journal files are left out of Bruhs' own
    /// folders only; owner content keeps them.
    #[test]
    fn owner_content_keeps_its_lock_and_journal_files() {
        let options = BackupOptions {
            audio: true,
            browser_profiles: true,
            ..BackupOptions::default()
        };
        for kept in [
            "server/bruhs/a1/work/Cargo.lock",
            "server/bruhs/a1/work/app/yarn.lock",
            "server/bruhs/a1/work/notes.db-journal",
            "server/bruhs/a1/work/notes.db-wal",
            "attachments/t1/flake.lock",
            "knowledge-attachments/k.sqlite-journal",
        ] {
            assert_eq!(excluded(kept, false), Excluded::No, "{kept}");
            assert!(included(kept, false, &options), "{kept}");
        }
        for dropped in [
            "server/reactive.sqlite3.lock",
            "server/meetings.sqlite3-journal",
            "server/mail.sqlite3-shm",
            "calendar/google/state.lock",
            "server/bruhs/a1/profile.json.lock",
        ] {
            assert_eq!(excluded(dropped, false), Excluded::Always, "{dropped}");
            assert!(!included(dropped, false, &options), "{dropped}");
        }
    }

    /// I2 ruling: broker tokens and a browser's pid file never go in.
    #[test]
    fn brokers_and_browser_pid_files_are_always_out() {
        let all = BackupOptions {
            keychain: true,
            audio: true,
            browser_profiles: true,
        };
        for rel in [
            "credentials/brokers",
            "credentials/brokers/broker-1-x.json",
            "server/bruhs/a1/browser.pid",
        ] {
            assert_eq!(
                excluded(rel, rel.ends_with("brokers")),
                Excluded::Always,
                "{rel}"
            );
            assert!(!included(rel, false, &all), "{rel}");
        }
        assert!(included("credentials/vault.json", false, &all));
        assert!(included("credentials/catalog.json", false, &all));
    }

    /// Minor 4: a bruh's browser databases are copied raw, so their SQLite
    /// side files stay with them; lock files still don't.
    #[test]
    fn browser_profiles_keep_sqlite_side_files() {
        for rel in [
            "server/bruhs/a1/browser/Default/Cookies-journal",
            "server/bruhs/a1/browser/Default/History-wal",
            "server/bruhs/a1/browser/Default/History-shm",
        ] {
            assert_eq!(excluded(rel, false), Excluded::Browser, "{rel}");
        }
        assert_eq!(
            excluded("server/bruhs/a1/browser/Default/x.lock", false),
            Excluded::Always
        );
        assert_eq!(
            excluded("server/bruhs/a1/profile.sqlite3-wal", false),
            Excluded::Always
        );
    }

    /// Ruling F9: audio off leaves out media files only.
    #[test]
    fn audio_off_keeps_transcripts_centroids_and_ocr() {
        let off = BackupOptions {
            audio: false,
            ..BackupOptions::default()
        };
        for kept in [
            "audio/m1.jsonl",
            "audio/m1/m1.centroids.json",
            "audio/m1/m1.ocr.json",
        ] {
            assert_eq!(excluded(kept, false), Excluded::No, "{kept}");
            assert!(included(kept, false, &off), "{kept}");
        }
        assert!(included("audio", true, &off) && included("audio/m1", true, &off));
        for media in [
            "audio/m1/m1.mic.wav",
            "audio/m1/m1.sys.wav",
            "audio/m1/m1.mixed.WAV",
            "audio/m1/x.m4a",
        ] {
            assert_eq!(excluded(media, false), Excluded::Audio, "{media}");
        }
        assert_eq!(excluded("attachments/a.wav", false), Excluded::No);
    }

    #[test]
    fn exclusion_kinds_name_the_reason() {
        assert_eq!(excluded("server/bruhs/a1/browser", true), Excluded::Browser);
        assert_eq!(
            excluded("server/bruhs/a1/browser/GrShaderCache", true),
            Excluded::Always
        );
        assert_eq!(
            excluded("server/bruhs/a1/browser-cache", true),
            Excluded::Always
        );
        assert_eq!(excluded("server/reactive.sqlite3", false), Excluded::No);
        assert_eq!(
            excluded("server/meetings.sqlite3-journal", false),
            Excluded::Always
        );
        assert_eq!(excluded("server/x-shm", false), Excluded::Always);
        assert_eq!(
            excluded("codex-lean-sessions.lock", false),
            Excluded::Always
        );
        assert_eq!(excluded("a-future-folder/x", false), Excluded::No);
        assert_eq!(excluded("worktrees", true), Excluded::Always);
        assert_eq!(EXCLUDED_TOP.len(), 9);
        assert!(!rewritable("audio/m1.jsonl"));
        assert!(!rewritable("knowledge-attachments/k.json"));
        assert!(!rewritable("server/bruhs/a1/browser/Default/Preferences"));
        assert!(rewritable("server/bruhs/a1/profile.json"));
        assert!(rewritable("desktop-state.json"));
    }

    #[test]
    fn desktop_entries_include_diff_viewed() {
        for name in [
            "desktop-state.json",
            "desktop-state.json.bak",
            "knowledge-attachments",
            "delivery",
            "diff-viewed.json",
            "keybindings.json",
            "text-generators.sqlite3",
        ] {
            assert!(desktop_entry(name), "{name}");
        }
        assert!(!desktop_entry("server") && !desktop_entry("attachments"));
    }

    fn linux() -> RootsInfo {
        RootsInfo {
            home: "/home/ada".into(),
            core: "/home/ada/.local/share/bruhs-ai".into(),
            desktop: "/home/ada/.local/share/bruhs-ai".into(),
        }
    }

    fn mac() -> RootsInfo {
        RootsInfo {
            home: "/Users/ada".into(),
            core: "/Users/ada/Library/Application Support/dev.bruhs.bruhs-ai".into(),
            desktop: "/Users/ada/Library/Application Support/bruhs-ai".into(),
        }
    }

    fn assert_unique_from(rules: &[PathRewrite]) {
        let froms: BTreeSet<&str> = rules.iter().map(|rule| rule.from.as_str()).collect();
        assert_eq!(froms.len(), rules.len(), "{rules:?}");
        assert!(rules.windows(2).all(|w| w[0].from.len() >= w[1].from.len()));
    }

    #[test]
    fn rewrite_rules_map_roots_before_home() {
        let (linux, mac) = (linux(), mac());
        let rules = rewrite_rules(&linux, &mac);
        assert_unique_from(&rules);
        assert_eq!(
            rewrite_path(
                "/home/ada/.local/share/bruhs-ai/server/bruhs/a1/work",
                &rules
            )
            .as_deref(),
            Some("/Users/ada/Library/Application Support/dev.bruhs.bruhs-ai/server/bruhs/a1/work")
        );
        assert_eq!(
            rewrite_path(
                "/home/ada/.local/share/bruhs-ai/knowledge-attachments/k.pdf",
                &rules
            )
            .as_deref(),
            Some("/Users/ada/Library/Application Support/bruhs-ai/knowledge-attachments/k.pdf")
        );
        assert_eq!(
            rewrite_path("/home/ada/src/app", &rules).as_deref(),
            Some("/Users/ada/src/app")
        );
        assert_eq!(rewrite_path("/home/adam/src", &rules), None);
        assert_eq!(rewrite_path("see /home/ada/src", &rules), None);
        assert!(rewrite_rules(&mac, &mac).is_empty());
        let mut doc = json!({"project_path": "/home/ada/src/app", "/home/ada/k": 1, "n": [{"cwd": "/Volumes/Work/repo"}, {"cwd": "/tmp/x"}]});
        assert_eq!(rewrite_json(&mut doc, &rules), 2);
        assert_eq!(doc["project_path"], "/Users/ada/src/app");
        assert!(doc.get("/Users/ada/k").is_some());
        let mut outside = BTreeSet::new();
        collect_outside(&doc, &rules, &mut outside);
        assert_eq!(outside.into_iter().collect::<Vec<_>>(), ["/Volumes/Work"]);
    }

    /// Ruling F8: two roots onto one, and one root onto one.
    #[test]
    fn rewrite_rules_merge_two_roots_onto_one() {
        let (linux, mac) = (linux(), mac());
        let rules = rewrite_rules(&mac, &linux);
        assert_unique_from(&rules);
        assert_eq!(
            rewrite_path(
                "/Users/ada/Library/Application Support/bruhs-ai/delivery/journal.json",
                &rules
            )
            .as_deref(),
            Some("/home/ada/.local/share/bruhs-ai/delivery/journal.json")
        );
        assert_eq!(
            rewrite_path(
                "/Users/ada/Library/Application Support/dev.bruhs.bruhs-ai/server",
                &rules
            )
            .as_deref(),
            Some("/home/ada/.local/share/bruhs-ai/server")
        );
        let other = RootsInfo {
            home: "/home/bo".into(),
            core: "/home/bo/.local/share/bruhs-ai".into(),
            desktop: "/home/bo/.local/share/bruhs-ai".into(),
        };
        let rules = rewrite_rules(&linux, &other);
        assert_unique_from(&rules);
        assert_eq!(rules.len(), 2, "{rules:?}");
        assert_eq!(
            rewrite_path("/home/ada/.local/share/bruhs-ai/delivery", &rules).as_deref(),
            Some("/home/bo/.local/share/bruhs-ai/delivery")
        );
    }

    #[test]
    fn outside_paths_are_capped_and_skip_system_folders() {
        let rules = rewrite_rules(&linux(), &mac());
        let mut outside = BTreeSet::new();
        collect_outside(
            &json!({"cwd": [
                "/dev/null",
                "/private/tmp/x/y",
                "/var/folders/ab/T",
                "/proc/1/fd",
                "/usr",
                "/a/b\nc"
            ]}),
            &rules,
            &mut outside,
        );
        assert!(outside.is_empty(), "{outside:?}");
        let many: Vec<String> = (0..30).map(|n| format!("/Volumes/V{n}/x")).collect();
        collect_outside(&json!({ "project_path": many }), &rules, &mut outside);
        assert_eq!(outside.len(), 20);
        let mut none = BTreeSet::new();
        collect_outside(&json!({"cwd": "/Volumes/Work/repo"}), &[], &mut none);
        assert!(none.is_empty());
    }

    /// Minor 3: a `.`, `..` or empty component can't hide a path behind a
    /// rule's prefix or a system folder.
    #[test]
    fn irregular_paths_are_flagged_whole() {
        let rules = rewrite_rules(&linux(), &mac());
        let mut outside = BTreeSet::new();
        collect_outside(
            &json!({"cwd": [
                "/home/ada/../../Volumes/x",
                "/tmp/../Volumes/y",
                "//host/x",
                "/Volumes/./z",
                "/home/ada/src/",
                "/Volumes/Work/",
            ]}),
            &rules,
            &mut outside,
        );
        assert_eq!(
            outside.iter().map(String::as_str).collect::<Vec<_>>(),
            [
                "//host/x",
                "/Volumes/./z",
                "/Volumes/Work",
                "/home/ada/../../Volumes/x",
                "/tmp/../Volumes/y",
            ]
        );
    }

    /// Minor 7: a side that isn't absolute, or holds a control character,
    /// makes no rule.
    #[test]
    fn rewrite_rules_need_absolute_sides() {
        let relative = RootsInfo {
            home: "a".into(),
            core: "a/x".into(),
            desktop: "a/x".into(),
        };
        assert!(rewrite_rules(&relative, &mac()).is_empty());
        let mut broken = linux();
        broken.home = "/home/ada\n".into();
        let rules = rewrite_rules(&broken, &mac());
        assert!(
            rules.iter().all(|rule| rule.from != "/home/ada\n"),
            "{rules:?}"
        );
        assert_eq!(rewrite_path("/home/ada/src", &rules), None);
        assert!(rewrite_path("/home/ada/.local/share/bruhs-ai/server", &rules).is_some());
    }

    /// D10 ruling: only the known path fields are read, so routes and other
    /// strings that look like paths never reach the preview.
    #[test]
    fn outside_paths_come_only_from_path_fields() {
        let rules = rewrite_rules(&linux(), &mac());
        let mut outside = BTreeSet::new();
        let record = json!({
            "path": "/api/events",
            "url": "/api/events/stream",
            "/Volumes/Key/x": 1,
            "text": "/Volumes/Notes/x",
            "call": {"method": "GET", "path": "/api/events"},
            "items": ["/Volumes/Loose/x"],
            "work": {"cwd": "/Volumes/Work/repo"},
            "meeting": {"audio_path": "/Volumes/Rec/m1.wav"},
        });
        collect_outside(&record, &rules, &mut outside);
        assert_eq!(
            outside.iter().map(String::as_str).collect::<Vec<_>>(),
            ["/Volumes/Rec", "/Volumes/Work"]
        );
        assert!(!outside.contains("/api/events"));
        let mut cells = BTreeSet::new();
        collect_outside_in("project_path", &json!("/srv/repos/app"), &rules, &mut cells);
        collect_outside_in("summary", &json!("/api/events"), &rules, &mut cells);
        collect_outside_in(
            "metadata",
            &json!({"cwd": "/opt/work/x"}),
            &rules,
            &mut cells,
        );
        assert_eq!(
            cells.iter().map(String::as_str).collect::<Vec<_>>(),
            ["/opt/work", "/srv/repos"]
        );
        assert!(path_field("cwd") && !path_field("path"));
        assert!(path_field("worktreePath") && path_field("connection_path"));
    }
}
