//! Chats from chat services' data exports (ChatGPT, Claude.ai, Grok) and
//! bruhs from local agents (OpenClaw, Hermes Agent): the caps every read
//! keeps to, the wire types the server answers with, the rules both apps
//! share, and every line of app-owned copy ([`ALL_COPY`]).
//!
//! `bruhs_core::import` does the reading; this module holds no I/O.

mod copy;
pub use copy::*;

use crate::logic::portable::{ImportCandidate, ImportSource};
use crate::time::EpochMs;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

/// Bytes one read takes in total, across every uploaded or picked file.
pub const MAX_UPLOAD_BYTES: u64 = 4 * GIB;
/// Files one read takes.
pub const MAX_UPLOAD_FILES: usize = 16;
/// Entries one ZIP may hold.
pub const MAX_ARCHIVE_ENTRIES: usize = 200_000;
/// Levels of ZIP read inside the uploaded one.
pub const MAX_ZIP_NESTING: usize = 2;
/// Bytes one JSON entry may inflate to, counted on the bytes read.
pub const MAX_JSON_ENTRY_BYTES: u64 = 2 * GIB;
/// An entry may inflate to at most this many times its compressed size...
pub const MAX_INFLATE_RATIO: u64 = 200;
/// ...or to this many bytes, whichever is more.
pub const RATIO_FLOOR_BYTES: u64 = 64 * MIB;
/// Bytes one read inflates in total, nested copies included.
pub const MAX_INFLATED_BYTES: u64 = 8 * GIB;
/// Bytes of a file read whole: a manifest, a memory file or a bare object.
pub const MAX_SMALL_JSON_BYTES: u64 = 16 * MIB;
/// Bytes of one conversation record.
pub const MAX_RECORD_BYTES: u64 = 32 * MIB;
/// Conversations one read stores; later ones are counted, not stored.
pub const MAX_CONVERSATIONS: usize = 50_000;
/// Mapping or tree nodes walked per conversation.
pub const MAX_NODES: usize = 100_000;
/// Messages kept per conversation; the oldest past it are dropped.
pub const MAX_MESSAGES: usize = 20_000;
/// Characters of one text part.
pub const MAX_PART_CHARS: usize = 200_000;
/// Characters of an attachment's extracted text (Claude's `extracted_content`).
pub const MAX_ATTACHMENT_TEXT_CHARS: usize = 4_000;
/// Bytes of normalized threads one batch stores.
pub const MAX_STORED_BYTES: u64 = 2 * GIB;
/// Batches kept; a read past it discards the oldest.
pub const MAX_BATCHES: usize = 3;
/// How long a batch is kept.
pub const BATCH_TTL_MS: i64 = 86_400_000;
/// Bytes of one persona or memory file.
pub const MAX_PERSONA_FILE_BYTES: u64 = 256 * 1024;
/// `memory/YYYY-MM-DD.md` files read, newest first.
pub const MAX_DAILY_LOGS: usize = 400;
/// Personas one scan answers.
pub const MAX_PERSONAS: usize = 32;
/// Folder levels walked under a picked export folder.
pub const MAX_FOLDER_DEPTH: usize = 4;
/// Bytes of a `POST /api/import/files` body.
pub const MAX_PATHS_BODY: usize = 64 * 1024;
/// Threads one press of Import all brings in.
pub const IMPORT_ALL_MAX: usize = 100;
/// Rows a list shows; search reaches the rest.
pub const MAX_ROWS_SHOWN: usize = 500;

/// Where one read of an export stands (`GET /api/import/uploads/{id}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ImportJobState {
    Reading {
        #[serde(default)]
        files_done: u32,
        #[serde(default)]
        files: u32,
        #[serde(default)]
        conversations: u32,
    },
    Ready {
        batch: Box<ImportBatch>,
    },
    Failed {
        #[serde(default)]
        message: String,
    },
}

/// A read of an export: its id is its batch's id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportJob {
    #[serde(default)]
    pub id: String,
    pub state: ImportJobState,
}

/// What one read found, kept for [`BATCH_TTL_MS`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportBatch {
    pub id: String,
    pub candidates: Vec<ImportCandidate>,
    pub memory: Vec<MemorySuggestion>,
    pub counts: BatchCounts,
    pub warnings: Vec<String>,
    /// Set when the batch holds a Grok conversation.
    pub best_effort: bool,
    pub expires_at_ms: EpochMs,
}

/// The counts a batch's summary lines report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchCounts {
    pub conversations: u32,
    pub skipped_empty: u32,
    pub skipped_unknown: u32,
    pub duplicates: u32,
    pub dropped_branches: u32,
    pub over_limit: u32,
    pub files_read: u32,
}

/// Memory an export holds about the owner, offered as a bruh's memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySuggestion {
    pub source: ImportSource,
    #[serde(default)]
    pub text: String,
}

/// `POST /api/import/files`: files or folders the desktop's picker chose.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPaths {
    pub paths: Vec<String>,
}

/// `GET /api/import/agents?source=&path=`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentScanQuery {
    pub source: String,
    /// A folder the owner typed or picked, else the default places.
    #[serde(default)]
    pub path: Option<String>,
}

/// The personas one scan found, and where it looked.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentScan {
    pub personas: Vec<PersonaCandidate>,
    pub looked_in: Vec<String>,
}

/// An OpenClaw workspace or a Hermes Agent home, ready to become a bruh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaCandidate {
    #[serde(default)]
    pub id: String,
    pub source: ImportSource,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub memory: String,
    #[serde(default)]
    pub files: Vec<PersonaFile>,
    /// Some text was cut to fit.
    #[serde(default)]
    pub clipped: bool,
    /// Labels of identity fields not carried over, such as "Emoji".
    #[serde(default)]
    pub dropped_identity: Vec<String>,
}

/// One file a scan saw, and what became of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersonaFile {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "use")]
    pub use_: FileUse,
}

/// What a persona file was read for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FileUse {
    Identity,
    Instructions,
    Memory,
    Skipped { reason: SkipReason },
}

/// Why a persona file was left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// Listed by name only: a file Bruhs doesn't import.
    NotImported,
    Link,
    TooLarge,
    NotAFile,
    Unreadable,
}

/// Whether `id` names a batch (and its job): 32 lowercase hex characters.
pub fn valid_batch_id(id: &str) -> bool {
    crate::backup::valid_id(id)
}

/// The candidate id for a provider's conversation id: the id itself when
/// it is 1 to 128 of `[A-Za-z0-9_.:-]`, else `h-` and 16 hex characters of
/// its SHA-256.
pub fn conversation_key(raw: &str) -> String {
    let plain = (1..=128).contains(&raw.len())
        && raw
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-'));
    if plain {
        return raw.to_string();
    }
    let digest = Sha256::digest(raw.as_bytes());
    let mut key = String::with_capacity(18);
    key.push_str("h-");
    for byte in &digest[..8] {
        let _ = write!(key, "{byte:02x}");
    }
    key
}

/// The key a thread imported from `source` keeps, so it isn't offered
/// again: `"<source wire>:<candidate id>"`.
pub fn import_key(source: ImportSource, id: &str) -> String {
    format!("{}:{id}", source.as_str())
}

/// `n` and the word for it: `"1 chat"`, `"3 chats"`.
pub fn plural(n: u64, one: &str, other: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { other })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::portable::ImportSource;
    use serde_json::json;

    #[test]
    fn keys_and_ids_are_safe() {
        assert_eq!(conversation_key("abc-123_x.y:z"), "abc-123_x.y:z");
        let hashed = conversation_key("../etc/passwd");
        assert!(hashed.starts_with("h-") && hashed.len() == 18);
        assert_eq!(conversation_key(&"a".repeat(129)).len(), 18);
        assert_eq!(import_key(ImportSource::ChatGpt, "abc"), "chatgpt:abc");
        assert!(valid_batch_id(&"a1".repeat(16)) && !valid_batch_id("../x"));
        assert_eq!(
            (plural(1, "chat", "chats"), plural(3, "chat", "chats")),
            ("1 chat".into(), "3 chats".into())
        );
    }

    #[test]
    fn copy_states_the_caps() {
        assert_eq!(crate::reactive::MAX_TEXT_CHARS, 16_000);
        assert!(PERSONA_CLIPPED.contains("16,000") && MOVE_IN_TOO_LONG.contains("16,000"));
        assert_eq!((MAX_UPLOAD_BYTES, MAX_UPLOAD_FILES), (4 << 30, 16));
        assert!(EXPORT_TOO_LARGE.contains("4 GB") && TOO_MANY_FILES.contains("16 files"));
        assert_eq!(MAX_RECORD_BYTES, 32 << 20);
        assert!(CONVERSATION_TOO_LARGE.contains("32 MB"));
        assert_eq!(MAX_CONVERSATIONS, 50_000);
        assert!(OVER_LIMIT_TEMPLATE.contains("50,000"));
        assert_eq!(MAX_PERSONA_FILE_BYTES, 256 << 10);
        assert!(SKIPPED_TOO_LARGE_TEMPLATE.contains("256 KB"));
    }

    #[test]
    fn wire_shapes_are_stable() {
        let job: ImportJob =
            serde_json::from_value(json!({"id":"a","state":{"state":"failed","message":"m"}}))
                .unwrap();
        assert_eq!(
            job.state,
            ImportJobState::Failed {
                message: "m".into()
            }
        );
        let batch: ImportBatch = serde_json::from_value(json!({"id":"a","future":1})).unwrap();
        assert!(batch.candidates.is_empty() && !batch.best_effort);
        assert!(serde_json::from_str::<ImportPaths>(r#"{"paths":[],"x":1}"#).is_err());
        let file = PersonaFile {
            name: "x".into(),
            use_: FileUse::Skipped {
                reason: SkipReason::Link,
            },
        };
        assert_eq!(
            serde_json::to_value(&file).unwrap(),
            json!({"name":"x","use":{"kind":"skipped","reason":"link"}})
        );
    }
}
