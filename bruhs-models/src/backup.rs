//! Backup: one encrypted file with everything Bruhs keeps, and its restore.
//!
//! The wire types the server, the web and the desktop share, the password
//! rule, names and ids, and the lines both apps show. Pure and wasm-safe: the
//! encryption lives in `bruhs-backup`, the file work in `bruhs-core`.

use serde::{Deserialize, Deserializer, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::crew::CREW_PROTOCOL;
use crate::EpochMs;

mod copy;
pub use copy::*;
mod paths;
pub use paths::*;
mod web;
pub use web::*;

/// The newest manifest format this build reads.
pub const MANIFEST_FORMAT: u16 = 1;
/// A password has at least this many characters.
pub const MIN_PASSWORD_CHARS: usize = 12;
/// A password has at most this many UTF-8 bytes.
pub const MAX_PASSWORD_BYTES: usize = 1024;

/// What goes in besides the data folders.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(default)]
pub struct BackupOptions {
    /// Saved logins, keys and app tokens from the keychain.
    pub keychain: bool,
    /// Meeting audio files (transcripts always go in).
    pub audio: bool,
    /// The bruhs' browser profiles and their sign-ins.
    pub browser_profiles: bool,
}

impl Default for BackupOptions {
    fn default() -> Self {
        Self {
            keychain: true,
            audio: true,
            browser_profiles: false,
        }
    }
}

/// The app a backup was made from.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackupApp {
    #[default]
    Desktop,
    Web,
    /// An app a newer Bruhs added. Read only: requests refuse it.
    #[serde(other)]
    Other,
}

impl BackupApp {
    /// The app's name inside a sentence ("the desktop app").
    pub const fn label(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Web => "web",
            Self::Other => "other",
        }
    }
}

/// `POST /api/backup`. Write-only: deliberately no `Debug` or `Clone`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBackup {
    pub password: String,
    #[serde(default)]
    pub options: BackupOptions,
    #[serde(deserialize_with = "known")]
    pub app: BackupApp,
    /// The id of the browser's encrypted part, uploaded first.
    #[serde(default)]
    pub web_part: Option<String>,
    /// The IndexedDB version the browser dumped its part from. A web part
    /// without one is refused.
    #[serde(default)]
    pub web_db_version: Option<u32>,
    /// The desktop's chosen file; the web downloads instead.
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub overwrite: bool,
}

/// `POST /api/backup/restore`. Write-only: deliberately no `Debug` or `Clone`.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenBackup {
    pub source: OpenSource,
    pub password: String,
}

/// Where the backup to open is: an upload or a file on this computer.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenSource {
    Upload { id: String },
    Path { path: String },
}

/// What a restore puts in place: an opened backup or a kept set.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RestoreSource {
    Staged {
        id: String,
    },
    PreRestore {
        stamp: String,
    },
    /// A source a newer Bruhs added. Read only: requests refuse it.
    #[serde(other)]
    Other,
}

/// Only so a `PendingRestore` can default its fields; an empty id is never
/// valid.
impl Default for RestoreSource {
    fn default() -> Self {
        Self::Staged { id: String::new() }
    }
}

/// `POST /api/backup/restore/apply`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApplyRestore {
    #[serde(deserialize_with = "known")]
    pub source: RestoreSource,
    /// The owner confirmed that their data will be replaced.
    pub replace: bool,
    #[serde(default)]
    pub from_desktop: bool,
}

/// The answer to an accepted apply, sent just before the server stops.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApplyAccepted {
    #[serde(default)]
    pub service_stopped: bool,
}

/// What a running job is doing.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStep {
    Collecting,
    Keychain,
    Writing,
    Checking,
    Preparing,
    /// A step a newer Bruhs added.
    #[serde(other)]
    Other,
}

/// A make or open job.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    Running {
        done_bytes: u64,
        total_bytes: u64,
        step: JobStep,
    },
    Created {
        file: BackupFile,
    },
    Opened {
        preview: Box<RestorePreview>,
    },
    Failed {
        message: String,
    },
    /// A state a newer Bruhs added.
    #[serde(other)]
    Other,
}

/// The read-only `Other` fallbacks above let an older app decode a newer
/// server's answers. A request body never carries one.
trait Known {
    fn is_known(&self) -> bool;
}

impl Known for BackupApp {
    fn is_known(&self) -> bool {
        !matches!(self, Self::Other)
    }
}

impl Known for RestoreSource {
    fn is_known(&self) -> bool {
        !matches!(self, Self::Other)
    }
}

fn known<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Known,
{
    let value = T::deserialize(deserializer)?;
    if value.is_known() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom("unknown variant"))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BackupJob {
    pub id: String,
    pub state: JobState,
}

/// A finished backup file.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BackupFile {
    pub name: String,
    pub bytes: u64,
    /// Where the desktop saved it.
    #[serde(default)]
    pub path: Option<String>,
    /// The web's download URL.
    #[serde(default)]
    pub download: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// A path prefix a restore rewrites.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PathRewrite {
    pub from: String,
    pub to: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(default)]
pub struct BackupCounts {
    pub bruhs: u32,
    pub vault_entries: u32,
    pub keychain_items: u32,
    pub files: u64,
}

/// What an opened backup holds, shown before the owner confirms.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct RestorePreview {
    pub id: String,
    pub made_at_ms: EpochMs,
    pub made_on: BackupApp,
    pub app_version: String,
    pub source_home: String,
    #[serde(default)]
    pub rewrites: Vec<PathRewrite>,
    #[serde(default)]
    pub outside_home: Vec<String>,
    #[serde(default)]
    pub counts: BackupCounts,
    pub bytes: u64,
    #[serde(default)]
    pub has_web_part: bool,
    #[serde(default)]
    pub has_keychain: bool,
    #[serde(default)]
    pub has_audio: bool,
    #[serde(default)]
    pub has_browser_profiles: bool,
    #[serde(default)]
    pub skipped_links: u32,
}

/// `manifest.json`, the last entry of a backup. No `deny_unknown_fields`: a
/// newer format is refused by its numbers ([`BackupManifest::readable`]).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BackupManifest {
    pub format: u16,
    pub app_version: String,
    pub made_at_ms: EpochMs,
    pub made_on: BackupApp,
    pub reactive_protocol: u32,
    #[serde(default)]
    pub web_db_version: Option<u32>,
    pub os: String,
    pub home: String,
    pub core_root: String,
    pub desktop_root: String,
    #[serde(default)]
    pub same_root: bool,
    #[serde(default)]
    pub options: BackupOptions,
    pub files: Vec<ManifestFile>,
    #[serde(default)]
    pub keychain_items: u32,
    #[serde(default)]
    pub skipped_links: u32,
    /// Owner files left out because a restore would refuse their names.
    #[serde(default)]
    pub skipped_names: u32,
    /// Files that changed while they were read; each holds the part there
    /// when the backup listed it (ruling B Task 6 I2).
    #[serde(default)]
    pub changed_files: u32,
    #[serde(default)]
    pub counts: BackupCounts,
}

impl BackupManifest {
    /// Whether this build can restore the backup: neither its format nor its
    /// crew protocol is newer than this build's.
    pub fn readable(&self) -> Result<(), &'static str> {
        if self.format > MANIFEST_FORMAT || self.reactive_protocol > CREW_PROTOCOL {
            Err(NEWER_BACKUP)
        } else {
            Ok(())
        }
    }
}

/// One file in the archive with its size and SHA-256 (lowercase hex).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

/// How much each option adds.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(default)]
pub struct BackupSizes {
    pub data_bytes: u64,
    pub audio_bytes: u64,
    pub browser_bytes: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(default)]
pub struct LastBackup {
    pub made_at_ms: EpochMs,
    pub app: BackupApp,
    pub bytes: u64,
}

/// A restore scheduled to finish when Bruhs starts again.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PendingRestore {
    pub source: RestoreSource,
    pub created_at_ms: EpochMs,
}

/// A set of data kept from before a restore.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct PreRestoreInfo {
    pub stamp: String,
    pub made_at_ms: EpochMs,
    pub bytes: u64,
    pub has_web_part: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct StagedRestore {
    pub preview: RestorePreview,
}

/// `GET /api/backup`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct BackupOverview {
    pub supported: bool,
    pub sizes: BackupSizes,
    pub keychain_items: u32,
    pub outside_roots: Vec<String>,
    pub last: Option<LastBackup>,
    pub job: Option<BackupJob>,
    pub pending: Option<PendingRestore>,
    pub staged: Option<StagedRestore>,
    pub pre_restores: Vec<PreRestoreInfo>,
    pub web_part_waiting: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct RestoreResult {
    pub applied_at_ms: EpochMs,
    pub stamp: String,
    pub warnings: Vec<String>,
}

/// A restored backup's browser part, waiting for the web to take it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct WebPartInfo {
    pub id: String,
    pub bytes: u64,
    pub rewrites: Vec<PathRewrite>,
}

/// `GET /api/backup/restore/status`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct RestoreStatus {
    pub pending: Option<PendingRestore>,
    pub result: Option<RestoreResult>,
    pub web_part: Option<WebPartInfo>,
}

/// The id of an upload.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
#[serde(default)]
pub struct UploadId {
    pub id: String,
}

/// The switches in Make a backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptionKind {
    Keychain,
    Audio,
    Browsers,
}

/// The password as the key derivation takes it: Unicode NFC, so an "é"
/// typed as one code point on one computer and as "e" plus an accent on
/// another opens the same backup. Every create, open and web-part lock or
/// unlock derives from this, never from the typed string. Callers hold the
/// result in `Zeroizing` (this crate has no `zeroize`).
pub fn normalize_password(password: &str) -> String {
    password.nfc().collect()
}

/// A backup password, judged in its [`normalize_password`] form: at least
/// [`MIN_PASSWORD_CHARS`] characters, at most [`MAX_PASSWORD_BYTES`] bytes,
/// no control characters and not only whitespace.
pub fn validate_password(password: &str) -> Result<(), &'static str> {
    // Checked on the typed bytes first, so a huge input is never normalized.
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(PASSWORD_LONG);
    }
    let normalized = normalize_password(password);
    if normalized.len() > MAX_PASSWORD_BYTES {
        Err(PASSWORD_LONG)
    } else if normalized.chars().any(char::is_control) {
        Err(PASSWORD_CONTROL)
    } else if normalized.chars().count() < MIN_PASSWORD_CHARS {
        Err(PASSWORD_SHORT)
    } else if normalized.chars().all(char::is_whitespace) {
        Err(PASSWORD_BLANK)
    } else {
        Ok(())
    }
}

/// The error for a password typed twice, if any. The two match when their
/// [`normalize_password`] forms do, since that is what the key comes from.
pub fn password_pair_error(first: &str, second: &str) -> Option<&'static str> {
    match validate_password(first) {
        Err(message) => Some(message),
        Ok(()) if normalize_password(first) != normalize_password(second) => Some(PASSWORDS_DIFFER),
        Ok(()) => None,
    }
}

/// `"{n} {one}"` when `n` is 1, else `"{n} {other}"`.
fn plural(n: u64, one: &str, other: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { other })
}

/// The extension every backup file has.
pub const BACKUP_EXTENSION: &str = "bruhsbak";

/// The file name a backup is written under: `name` as it is when its
/// extension is already [`BACKUP_EXTENSION`], else `name` with it appended
/// (`notes` becomes `notes.bruhsbak`, `x.zip` becomes `x.zip.bruhsbak`). A
/// leading dot doesn't start an extension, as `Path::extension` reads it.
pub fn with_backup_extension(name: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 && name[dot + 1..] == *BACKUP_EXTENSION => name.to_owned(),
        _ => format!("{name}.{BACKUP_EXTENSION}"),
    }
}

/// The filled [`SERVER_ANSWERED`].
pub fn server_answered(status: u16) -> String {
    fill(SERVER_ANSWERED, &[("status", &status.to_string())])
}

/// `bruhs-backup-YYYY-MM-DD-HHMM.bruhsbak`, in UTC.
pub fn backup_file_name(made_at_ms: EpochMs) -> String {
    let (year, month, day, hour, minute) = utc_parts(made_at_ms);
    format!("bruhs-backup-{year:04}-{month:02}-{day:02}-{hour:02}{minute:02}.bruhsbak")
}

/// `YYYY-MM-DD HH:MM UTC`.
pub fn format_utc(at: EpochMs) -> String {
    let (year, month, day, hour, minute) = utc_parts(at);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

/// Year, month, day, hour and minute in UTC, from the civil calendar
/// (Howard Hinnant's `civil_from_days`), with no time-zone data.
fn utc_parts(at: EpochMs) -> (i64, i64, i64, i64, i64) {
    let minutes = at.as_millis().div_euclid(60_000);
    let days = minutes.div_euclid(1440);
    let minute_of_day = minutes.rem_euclid(1440);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day, minute_of_day / 60, minute_of_day % 60)
}

/// A job, upload or staged-restore id: 32 lowercase hex characters.
pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(is_lower_hex)
}

/// A kept set's stamp: `YYYY-MM-DD-HHMM-<4 lowercase hex>`.
pub fn valid_stamp(stamp: &str) -> bool {
    let bytes = stamp.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, &byte)| match index {
            4 | 7 | 10 | 15 => byte == b'-',
            16..=19 => is_lower_hex(byte),
            _ => byte.is_ascii_digit(),
        })
}

fn is_lower_hex(byte: u8) -> bool {
    matches!(byte, b'0'..=b'9' | b'a'..=b'f')
}

/// A size in 1000-based units: "0 B", "512 KB", "1.2 MB", "3.4 GB". Whole
/// numbers up to KB, one decimal from MB up.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let exact = u128::from(bytes);
    let kilobytes = (exact + 500) / 1000;
    if kilobytes < 1000 {
        return format!("{kilobytes} KB");
    }
    let units = [
        ("MB", 1_000_000_u128),
        ("GB", 1_000_000_000),
        ("TB", 1_000_000_000_000),
    ];
    let mut tenths = 0;
    let mut name = "TB";
    for (unit, size) in units {
        tenths = (exact * 10 + size / 2) / size;
        name = unit;
        if tenths < 10_000 {
            break;
        }
    }
    format!("{}.{} {name}", tenths / 10, tenths % 10)
}

/// Puts named values into a copy template in one pass, so a value that
/// itself holds braces (a path) is never read as a placeholder.
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let value = after.find('}').and_then(|close| {
            let name = &after[..close];
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(value);
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// What an opened backup holds, one sentence per line.
pub fn preview_lines(preview: &RestorePreview) -> Vec<String> {
    let counts = preview.counts;
    let mut lines = vec![
        fill(
            PREVIEW_MADE,
            &[
                ("date", &format_utc(preview.made_at_ms)),
                ("app", preview.made_on.label()),
                ("version", &preview.app_version),
            ],
        ),
        fill(
            PREVIEW_COUNTS,
            &[
                (
                    "bruhs",
                    &plural(u64::from(counts.bruhs), COUNT_BRUH, COUNT_BRUHS),
                ),
                (
                    "entries",
                    &plural(
                        u64::from(counts.vault_entries),
                        COUNT_VAULT_ENTRY,
                        COUNT_VAULT_ENTRIES,
                    ),
                ),
                (
                    "items",
                    &plural(
                        u64::from(counts.keychain_items),
                        COUNT_KEYCHAIN_ITEM,
                        COUNT_KEYCHAIN_ITEMS,
                    ),
                ),
                ("files", &plural(counts.files, COUNT_FILE, COUNT_FILES)),
                ("size", &format_bytes(preview.bytes)),
            ],
        ),
    ];
    lines.extend(preview.rewrites.iter().map(|rewrite| {
        fill(
            PREVIEW_REWRITE,
            &[("from", &rewrite.from), ("to", &rewrite.to)],
        )
    }));
    if !preview.outside_home.is_empty() {
        lines.push(fill(
            PREVIEW_OUTSIDE,
            &[("list", &preview.outside_home.join(", "))],
        ));
    }
    lines.push(
        if preview.has_web_part {
            PREVIEW_WEB
        } else {
            PREVIEW_NO_WEB
        }
        .to_string(),
    );
    if !preview.has_keychain {
        lines.push(PREVIEW_NO_KEYCHAIN.to_string());
    }
    if !preview.has_audio {
        lines.push(PREVIEW_NO_AUDIO.to_string());
    }
    if !preview.has_browser_profiles {
        lines.push(PREVIEW_NO_BROWSERS.to_string());
    }
    if preview.skipped_links > 0 {
        let template = if preview.skipped_links == 1 {
            PREVIEW_SKIPPED_LINK
        } else {
            PREVIEW_SKIPPED_LINKS
        };
        lines.push(fill(template, &[("n", &preview.skipped_links.to_string())]));
    }
    lines
}

/// The progress line of a running job; `None` once it has finished.
pub fn job_line(state: &JobState) -> Option<String> {
    let JobState::Running {
        done_bytes,
        total_bytes,
        step,
    } = state
    else {
        return None;
    };
    let template = match step {
        JobStep::Collecting => JOB_COLLECTING,
        JobStep::Keychain => JOB_KEYCHAIN,
        JobStep::Writing => JOB_WRITING,
        JobStep::Checking => JOB_CHECKING,
        JobStep::Preparing => JOB_PREPARING,
        JobStep::Other => JOB_WORKING,
    };
    Some(fill(
        template,
        &[
            ("done", &format_bytes(*done_bytes)),
            ("total", &format_bytes(*total_bytes)),
        ],
    ))
}

/// A kept set in the Kept from earlier restores group.
pub fn pre_restore_line(info: &PreRestoreInfo) -> String {
    fill(
        PRE_RESTORE_LINE,
        &[
            ("date", &format_utc(info.made_at_ms)),
            ("size", &format_bytes(info.bytes)),
        ],
    )
}

/// The line under the title.
pub fn last_backup_line(last: &LastBackup) -> String {
    fill(
        LAST_BACKUP_LINE,
        &[
            ("date", &format_utc(last.made_at_ms)),
            ("app", last.app.label()),
            ("size", &format_bytes(last.bytes)),
        ],
    )
}

/// The confirm before a kept set is deleted.
pub fn delete_pre_restore_confirm(info: &PreRestoreInfo) -> String {
    fill(
        DELETE_PRE_RESTORE_CONFIRM,
        &[("date", &format_utc(info.made_at_ms))],
    )
}

/// The confirm before a kept set is put back.
pub fn put_back_confirm(info: &PreRestoreInfo) -> String {
    fill(PUT_BACK_CONFIRM, &[("date", &format_utc(info.made_at_ms))])
}

/// A switch's label. `amount` is the item count for
/// [`OptionKind::Keychain`] and a byte size for the others.
pub fn option_label(kind: OptionKind, amount: u64) -> String {
    match kind {
        OptionKind::Keychain => fill(KEYCHAIN_OPTION, &[("n", &amount.to_string())]),
        OptionKind::Audio => fill(AUDIO_OPTION, &[("size", &format_bytes(amount))]),
        OptionKind::Browsers => fill(BROWSERS_OPTION, &[("size", &format_bytes(amount))]),
    }
}

/// The line once a backup is made: ready to download on the web, saved on
/// the desktop.
pub fn created_line(file: &BackupFile, app: BackupApp) -> String {
    let template = match app {
        BackupApp::Web => CREATED_WEB,
        BackupApp::Desktop | BackupApp::Other => CREATED_DESKTOP,
    };
    fill(
        template,
        &[("name", &file.name), ("size", &format_bytes(file.bytes))],
    )
}

/// The note for data under a path override outside the roots (D23), if
/// there is any.
pub fn outside_roots_line(outside: &[String]) -> Option<String> {
    (!outside.is_empty()).then(|| fill(OUTSIDE_ROOTS, &[("list", &outside.join(", "))]))
}

/// The filled [`NO_SPACE`]: `needed` more bytes must be freed.
pub fn no_space(needed: u64) -> String {
    fill(NO_SPACE, &[("size", &format_bytes(needed))])
}

/// The warning for owner files and folders a backup left out by name, if
/// it left any out.
pub fn skipped_names_line(skipped: u32) -> Option<String> {
    counted(skipped, SKIPPED_NAME, SKIPPED_NAMES)
}

/// `one` or `other` filled with `n`, or `None` for zero.
fn counted(n: u32, one: &str, other: &str) -> Option<String> {
    let template = match n {
        0 => return None,
        1 => one,
        _ => other,
    };
    Some(fill(template, &[("n", &n.to_string())]))
}

/// The warning for files that changed while a backup read them, if any.
pub fn changed_files_line(changed: u32) -> Option<String> {
    counted(changed, CHANGED_FILE, CHANGED_FILES)
}

/// The warning for keychain items a catalog names but the keychain
/// doesn't hold, if any.
pub fn missing_items_line(missing: u32) -> Option<String> {
    counted(missing, MISSING_ITEM, MISSING_ITEMS)
}

/// The warning for today's keychain items a restore couldn't remove, if
/// any (final review B-I2).
pub fn items_stayed_line(stayed: u32) -> Option<String> {
    counted(stayed, ITEM_STAYED, ITEMS_STAYED)
}

/// The warning for a bruh whose browser was still busy, so its profile
/// stayed out of the backup.
pub fn browser_busy_line(name: &str) -> String {
    fill(BROWSER_BUSY, &[("name", name)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn passwords_follow_the_rule() {
        assert_eq!(validate_password("short"), Err(PASSWORD_SHORT));
        assert_eq!(validate_password("éééééééééééé"), Ok(())); // 12 chars, 24 bytes
        assert_eq!(validate_password("ééééééééééé"), Err(PASSWORD_SHORT)); // 11 chars, 22 bytes
        assert_eq!(validate_password(&"a".repeat(1024)), Ok(()));
        assert_eq!(validate_password(&"a".repeat(1025)), Err(PASSWORD_LONG));
        assert_eq!(
            validate_password("twelve chars\u{7}"),
            Err(PASSWORD_CONTROL)
        );
        assert_eq!(validate_password("twelve chars\0"), Err(PASSWORD_CONTROL));
        assert_eq!(
            password_pair_error("correct horse 1", "correct horse 2"),
            Some(PASSWORDS_DIFFER)
        );
        assert_eq!(
            password_pair_error("correct horse 1", "correct horse 1"),
            None
        );
        assert_eq!(password_pair_error("short", "short"), Some(PASSWORD_SHORT));
    }

    #[test]
    fn passwords_refuse_blanks_and_compare_in_nfc() {
        assert_eq!(validate_password(&" ".repeat(12)), Err(PASSWORD_BLANK));
        assert_eq!(validate_password(&"\u{a0}".repeat(12)), Err(PASSWORD_BLANK));
        assert_eq!(validate_password(&" ".repeat(3)), Err(PASSWORD_SHORT));
        assert_eq!(validate_password(&format!("{}a", " ".repeat(11))), Ok(()));

        let composed = "\u{e9}".repeat(12);
        let decomposed = "e\u{301}".repeat(12);
        assert_ne!(composed, decomposed);
        assert_eq!(normalize_password(&decomposed), composed);
        assert_eq!(normalize_password(&composed), composed);
        assert_eq!(validate_password(&decomposed), Ok(()));
        // 12 code points typed, but 6 characters once composed.
        assert_eq!(
            validate_password(&"e\u{301}".repeat(6)),
            Err(PASSWORD_SHORT)
        );
        assert_eq!(password_pair_error(&composed, &decomposed), None);
    }

    fn preview_with(counts: u64, skipped_links: u32) -> RestorePreview {
        let small = u32::try_from(counts).unwrap();
        RestorePreview {
            counts: BackupCounts {
                bruhs: small,
                vault_entries: small,
                keychain_items: small,
                files: counts,
            },
            bytes: 0,
            skipped_links,
            ..RestorePreview::default()
        }
    }

    #[test]
    fn counts_and_links_agree_with_their_number() {
        assert_eq!(
            preview_lines(&preview_with(0, 0))[1],
            "0 bruhs, 0 vault entries, 0 keychain items, 0 files, 0 B."
        );
        assert_eq!(
            preview_lines(&preview_with(1, 1))[1],
            "1 bruh, 1 vault entry, 1 keychain item, 1 file, 0 B."
        );
        assert_eq!(
            preview_lines(&preview_with(2, 2))[1],
            "2 bruhs, 2 vault entries, 2 keychain items, 2 files, 0 B."
        );
        let link_line = |links| {
            preview_lines(&preview_with(0, links))
                .into_iter()
                .find(|line| line.contains("left out"))
        };
        assert_eq!(link_line(0), None);
        assert_eq!(link_line(1).as_deref(), Some("1 link was left out."));
        assert_eq!(link_line(2).as_deref(), Some("2 links were left out."));
    }

    #[test]
    fn older_apps_read_newer_answers_but_requests_stay_strict() {
        assert_eq!(
            serde_json::from_str::<JobStep>(r#""compressing""#).unwrap(),
            JobStep::Other
        );
        assert_eq!(
            serde_json::from_str::<BackupApp>(r#""mobile""#).unwrap(),
            BackupApp::Other
        );
        let overview: BackupOverview = serde_json::from_value(json!({
            "job": {"id": "a", "state": {"state": "paused", "at": 1}},
            "last": {"made_at_ms": 0, "app": "mobile", "bytes": 1},
            "pending": {"source": {"type": "cloud", "url": "x"}, "created_at_ms": 0},
        }))
        .unwrap();
        let job = overview.job.unwrap();
        assert_eq!(job.state, JobState::Other);
        assert_eq!(job_line(&job.state), None);
        assert_eq!(overview.last.unwrap().app, BackupApp::Other);
        assert_eq!(overview.pending.unwrap().source, RestoreSource::Other);
        assert_eq!(
            job_line(&JobState::Running {
                done_bytes: 1_000,
                total_bytes: 2_000,
                step: JobStep::Other
            })
            .as_deref(),
            Some("Working: 1 KB of 2 KB")
        );

        assert!(serde_json::from_str::<ApplyRestore>(
            r#"{"source":{"type":"cloud"},"replace":true}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<CreateBackup>(r#"{"password":"p","app":"mobile"}"#).is_err()
        );
        assert!(
            serde_json::from_str::<CreateBackup>(r#"{"password":"p","app":"desktop"}"#).is_ok()
        );
    }

    #[test]
    fn wire_shapes_are_stable_and_bodies_are_strict() {
        assert_eq!(
            BackupOptions::default(),
            BackupOptions {
                keychain: true,
                audio: true,
                browser_profiles: false
            }
        );
        let apply: ApplyRestore =
            serde_json::from_str(r#"{"source":{"type":"staged","id":"ab"},"replace":true}"#)
                .unwrap();
        assert!(!apply.from_desktop);
        assert!(serde_json::from_str::<ApplyRestore>(
            r#"{"source":{"type":"staged","id":"ab"},"replace":true,"x":1}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<CreateBackup>(r#"{"password":"p","app":"web","extra":1}"#)
                .is_err()
        );
        let create: CreateBackup =
            serde_json::from_str(r#"{"password":"p","app":"web","options":{"audio":false}}"#)
                .unwrap();
        assert_eq!(
            create.options,
            BackupOptions {
                audio: false,
                ..BackupOptions::default()
            }
        );
        assert!(serde_json::from_str::<OpenBackup>(
            r#"{"source":{"type":"upload","id":"ab"},"password":"p","extra":1}"#
        )
        .is_err());
        assert_eq!(
            serde_json::to_value(RestoreSource::PreRestore { stamp: "s".into() }).unwrap(),
            json!({"type": "pre_restore", "stamp": "s"})
        );
        assert_eq!(
            serde_json::to_value(JobState::Running {
                done_bytes: 1,
                total_bytes: 2,
                step: JobStep::Collecting
            })
            .unwrap(),
            json!({"state": "running", "done_bytes": 1, "total_bytes": 2, "step": "collecting"})
        );
        let overview: BackupOverview = serde_json::from_str("{}").unwrap();
        assert_eq!(overview, BackupOverview::default());

        let manifest: BackupManifest = serde_json::from_value(json!({"format":1,"app_version":"0.1.1","made_at_ms":0,"made_on":"web",
            "reactive_protocol":CREW_PROTOCOL,"os":"macos","home":"/Users/a","core_root":"/c","desktop_root":"/d","files":[],"future":1}))
        .unwrap();
        assert!(manifest.readable().is_ok());
        assert_eq!(manifest.options, BackupOptions::default());
        assert_eq!(manifest.skipped_names, 0);
        let newer = BackupManifest {
            reactive_protocol: CREW_PROTOCOL + 1,
            ..manifest.clone()
        };
        assert_eq!(newer.readable(), Err(NEWER_BACKUP));
        let newer_format = BackupManifest {
            format: MANIFEST_FORMAT + 1,
            ..manifest
        };
        assert_eq!(newer_format.readable(), Err(NEWER_BACKUP));
    }

    #[test]
    fn the_final_name_has_the_extension_once() {
        for (name, final_name) in [
            ("notes", "notes.bruhsbak"),
            ("notes.bruhsbak", "notes.bruhsbak"),
            ("x.zip", "x.zip.bruhsbak"),
            (".bruhsbak", ".bruhsbak.bruhsbak"),
            ("notes.BRUHSBAK", "notes.BRUHSBAK.bruhsbak"),
            ("notes.", "notes..bruhsbak"),
        ] {
            assert_eq!(with_backup_extension(name), final_name, "{name}");
            // The same rule `Path::extension` reads.
            let has = std::path::Path::new(name)
                .extension()
                .is_some_and(|extension| extension == BACKUP_EXTENSION);
            assert_eq!(has, with_backup_extension(name) == name, "{name}");
        }
        assert_eq!(server_answered(502), "The server answered HTTP 502.");
    }

    #[test]
    fn names_ids_and_sizes_read_plainly() {
        assert_eq!(
            backup_file_name(EpochMs::try_from(1_795_185_000_000_i64).unwrap()),
            "bruhs-backup-2026-11-20-1430.bruhsbak"
        );
        assert_eq!(format_utc(EpochMs::UNIX_EPOCH), "1970-01-01 00:00 UTC");
        assert_eq!(
            format_utc(EpochMs::try_from(951_782_400_000_i64).unwrap()),
            "2000-02-29 00:00 UTC"
        );
        assert_eq!(
            format_utc(EpochMs::try_from(-60_000_i64).unwrap()),
            "1969-12-31 23:59 UTC"
        );
        assert!(valid_id(&"a1".repeat(16)) && !valid_id("../x") && !valid_id(&"A1".repeat(16)));
        assert!(!valid_id(&"a1".repeat(15)) && !valid_id(&"g1".repeat(16)));
        assert!(valid_stamp("2026-11-20-1430-ab12") && !valid_stamp("2026-11-20-1430-ab12/.."));
        assert!(!valid_stamp("2026-11-20-1430-AB12") && !valid_stamp("2026/11/20-1430-ab12"));
        assert_eq!(
            (
                format_bytes(0),
                format_bytes(512_000),
                format_bytes(1_234_567),
                format_bytes(3_400_000_000)
            ),
            (
                "0 B".into(),
                "512 KB".into(),
                "1.2 MB".into(),
                "3.4 GB".into()
            )
        );
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(999_999), "1.0 MB");
        assert_eq!(format_bytes(999_960_000), "1.0 GB");
        assert_eq!(format_bytes(u64::MAX), "18446744.1 TB");
    }

    #[test]
    fn previews_and_jobs_read_as_lines() {
        let preview = RestorePreview {
            id: "a".repeat(32),
            made_at_ms: EpochMs::try_from(1_795_185_000_000_i64).unwrap(),
            made_on: BackupApp::Desktop,
            app_version: "0.1.1".into(),
            source_home: "/home/ada".into(),
            rewrites: vec![PathRewrite {
                from: "/home/ada".into(),
                to: "/Users/ada".into(),
            }],
            outside_home: vec!["/Volumes/Work".into()],
            counts: BackupCounts {
                bruhs: 2,
                vault_entries: 3,
                keychain_items: 4,
                files: 120,
            },
            bytes: 1_234_567,
            has_web_part: false,
            has_keychain: true,
            has_audio: true,
            has_browser_profiles: false,
            skipped_links: 0,
        };
        let lines = preview_lines(&preview);
        assert_eq!(
            lines[0],
            "Made 2026-11-20 14:30 UTC on the desktop app, Bruhs 0.1.1."
        );
        assert!(lines.contains(
            &"2 bruhs, 3 vault entries, 4 keychain items, 120 files, 1.2 MB.".to_string()
        ));
        assert!(lines.contains(&"Paths under /home/ada become /Users/ada.".to_string()));
        assert!(lines.contains(
            &"These folders are outside the old home and stay as they are: /Volumes/Work."
                .to_string()
        ));
        assert!(lines.contains(&"No browser data in this backup.".to_string()));
        assert!(lines.contains(&PREVIEW_NO_BROWSERS.to_string()));
        assert!(!lines.contains(&PREVIEW_NO_KEYCHAIN.to_string()));
        assert!(!lines.contains(&PREVIEW_NO_AUDIO.to_string()));

        let bare = RestorePreview {
            rewrites: vec![PathRewrite {
                from: "/home/{to}".into(),
                to: "/Users/x".into(),
            }],
            outside_home: Vec::new(),
            has_web_part: true,
            has_keychain: false,
            has_audio: false,
            has_browser_profiles: true,
            skipped_links: 3,
            ..preview
        };
        let lines = preview_lines(&bare);
        assert_eq!(
            lines[1..],
            [
                "2 bruhs, 3 vault entries, 4 keychain items, 120 files, 1.2 MB.",
                "Paths under /home/{to} become /Users/x.",
                PREVIEW_WEB,
                PREVIEW_NO_KEYCHAIN,
                PREVIEW_NO_AUDIO,
                "3 links were left out.",
            ]
        );

        assert_eq!(
            job_line(&JobState::Running {
                done_bytes: 1_000_000,
                total_bytes: 2_000_000,
                step: JobStep::Writing
            })
            .as_deref(),
            Some("Writing the backup: 1.0 MB of 2.0 MB")
        );
        assert_eq!(
            job_line(&JobState::Running {
                done_bytes: 0,
                total_bytes: 0,
                step: JobStep::Keychain
            })
            .as_deref(),
            Some(JOB_KEYCHAIN)
        );
        assert_eq!(
            job_line(&JobState::Failed {
                message: DAMAGED.into()
            }),
            None
        );
        assert_eq!(
            option_label(OptionKind::Audio, 1_200_000_000),
            "Meeting audio (1.2 GB)"
        );
        assert_eq!(
            option_label(OptionKind::Keychain, 7),
            "Saved logins, keys and app tokens (7)"
        );
        assert_eq!(
            option_label(OptionKind::Browsers, 512_000),
            "Bruh browser sign-ins (512 KB)"
        );

        let kept = PreRestoreInfo {
            stamp: "2026-11-20-1430-ab12".into(),
            made_at_ms: EpochMs::try_from(1_795_185_000_000_i64).unwrap(),
            bytes: 3_400_000_000,
            has_web_part: false,
        };
        assert_eq!(
            pre_restore_line(&kept),
            "From before the restore on 2026-11-20 14:30 UTC: 3.4 GB."
        );
        assert_eq!(
            delete_pre_restore_confirm(&kept),
            "Delete the data from before the restore on 2026-11-20 14:30 UTC? It can't be brought back."
        );
        assert_eq!(
            put_back_confirm(&kept),
            "Put back the data from before the restore on 2026-11-20 14:30 UTC? Today's data is kept aside the same way."
        );
        assert_eq!(
            last_backup_line(&LastBackup {
                made_at_ms: kept.made_at_ms,
                app: BackupApp::Web,
                bytes: 512_000
            }),
            "Last backup: 2026-11-20 14:30 UTC from the web app, 512 KB."
        );
    }

    #[test]
    fn backup_warnings_fill_their_values() {
        assert_eq!(skipped_names_line(0), None);
        assert_eq!(
            skipped_names_line(1).as_deref(),
            Some("1 file or folder with a name a backup can't hold was left out.")
        );
        assert_eq!(
            skipped_names_line(3).as_deref(),
            Some("3 files or folders with names a backup can't hold were left out.")
        );
        assert_eq!(changed_files_line(0), None);
        assert_eq!(
            changed_files_line(1).as_deref(),
            Some("1 file changed while Bruhs read it, so its copy may be incomplete.")
        );
        assert_eq!(
            changed_files_line(2).as_deref(),
            Some("2 files changed while Bruhs read them, so their copies may be incomplete.")
        );
        assert_eq!(missing_items_line(0), None);
        assert_eq!(
            missing_items_line(1).as_deref(),
            Some("1 keychain item wasn't in your keychain, so it isn't in this backup.")
        );
        assert_eq!(
            missing_items_line(4).as_deref(),
            Some("4 keychain items weren't in your keychain, so they aren't in this backup.")
        );
        assert_eq!(
            browser_busy_line("Scout {n}"),
            "Scout {n}'s browser was busy, so its sign-ins aren't in this backup."
        );
        assert_eq!(
            no_space(1_500_000_000),
            "There isn't enough free disk space. Free 1.5 GB and try again."
        );
    }

    #[test]
    fn made_and_outside_lines_fill_their_values() {
        let file = BackupFile {
            name: "bruhs-backup-2026-11-20-0930.bruhsbak".into(),
            bytes: 1_500_000_000,
            path: None,
            download: None,
            warnings: Vec::new(),
        };
        assert_eq!(
            created_line(&file, BackupApp::Web),
            "Backup ready: bruhs-backup-2026-11-20-0930.bruhsbak, 1.5 GB."
        );
        assert_eq!(
            created_line(&file, BackupApp::Desktop),
            "Saved bruhs-backup-2026-11-20-0930.bruhsbak, 1.5 GB."
        );
        assert_eq!(outside_roots_line(&[]), None);
        assert_eq!(
            outside_roots_line(&["/data/{x}".into(), "/srv/meetings.db".into()]).as_deref(),
            Some("Some data lives outside Bruhs' folders and isn't in a backup: /data/{x}, /srv/meetings.db.")
        );
    }
}
