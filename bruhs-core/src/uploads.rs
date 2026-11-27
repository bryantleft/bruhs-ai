//! Files the user attached, and files the viewer opens from a path.
//!
//! Uploads live in the private app data directory. Legacy temp uploads
//! remain readable so old conversations can be captured durably. A
//! turn may only point the agent at files written there. Opening a path is
//! read-only and may leave the project: the viewer follows the file the
//! agent named.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bruhs_models::logic::attachments::{
    admit, kind_for_name, mime_for, upload_disk_name, upload_label, web_question_upload_prefix,
    FileKind, UploadedFile, COULD_NOT_READ, FILE_TOO_LARGE, MAX_FILE_BYTES, TEXT_PREVIEW_BYTES,
    UPLOAD_DID_NOT_FINISH,
};

pub mod browser_captures;

const ATTACHMENT_DIR: &str = "bruhs-attachments";
const DESKTOP_QUESTION_PREFIX: &str = "desktop-answer-";
const LINKED_THREAD_PREFIX: &str = "linked-thread-";

/// The shared output location for media in chats without a project. Merely
/// naming it does no I/O; the agent creates the directory before writing.
pub fn media_output_directory() -> std::io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("BRUHS_ATTACHMENTS_DIR") {
        if path.is_empty() {
            return Err(std::io::Error::other(
                "BRUHS_ATTACHMENTS_DIR must not be empty.",
            ));
        }
        return Ok(path.into());
    }
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_local_dir().join("attachments"))
        .ok_or_else(|| std::io::Error::other("The private app data directory is unavailable."))
}

/// Why a preview path could not be opened.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    BadPath,
    NoProject,
    NotFound,
    NotAFile,
    TooLarge,
    Read,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl OpenError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::BadPath => "That path is empty.",
            Self::NoProject => "Open a project to read a relative file.",
            Self::NotFound => "That file is not on this machine.",
            Self::NotAFile => "That path is not a file.",
            Self::TooLarge => FILE_TOO_LARGE,
            Self::Read => COULD_NOT_READ,
        }
    }
}

/// Bytes the viewer can show. Text and HTML stay strings. Media stays bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct OpenedFile {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    pub mime: &'static str,
    pub truncated: bool,
    pub text: Option<String>,
    pub bytes: Option<Vec<u8>>,
}

/// Write `bytes` under the attachment directory and return the absolute path.
pub fn write_upload(name: &str, bytes: &[u8]) -> Result<UploadedFile, &'static str> {
    let root = attachment_root().ok_or(UPLOAD_DID_NOT_FINISH)?;
    write_in(root, name, bytes)
}

pub(crate) fn write_desktop_question_upload(
    name: &str,
    bytes: &[u8],
) -> Result<UploadedFile, &'static str> {
    let root = attachment_root().ok_or(UPLOAD_DID_NOT_FINISH)?;
    write_in_with_prefix(root, name, bytes, DESKTOP_QUESTION_PREFIX)
}

pub(crate) fn write_web_question_upload(
    thread: &bruhs_models::ids::ThreadId,
    name: &str,
    bytes: &[u8],
) -> Result<UploadedFile, &'static str> {
    let root = attachment_root().ok_or(UPLOAD_DID_NOT_FINISH)?;
    write_in_with_prefix(
        root,
        name,
        bytes,
        &web_question_upload_prefix(thread.as_str()),
    )
}

pub fn linked_thread_upload_prefix(grant_id: &str, host_thread_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(grant_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(host_thread_id.as_bytes());
    format!("{LINKED_THREAD_PREFIX}{:x}-", hasher.finalize())
}

/// Write one binary or text attachment scoped to a linked grant and host thread.
/// Returns an opaque filename key so the remote client never sees host paths.
pub fn write_linked_thread_upload(
    grant_id: &str,
    host_thread_id: &str,
    name: &str,
    bytes: &[u8],
) -> Result<bruhs_models::coordination::linked_thread::LinkedAttachment, &'static str> {
    let root = attachment_root().ok_or(UPLOAD_DID_NOT_FINISH)?;
    let prefix = linked_thread_upload_prefix(grant_id, host_thread_id);
    let uploaded = write_in_with_prefix(root, name, bytes, &prefix)?;
    let id = Path::new(&uploaded.path)
        .file_name()
        .and_then(|file| file.to_str())
        .ok_or(UPLOAD_DID_NOT_FINISH)?
        .to_owned();
    Ok(
        bruhs_models::coordination::linked_thread::LinkedAttachment {
            id,
            name: uploaded.name,
            bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        },
    )
}

/// Resolve and validate attachments previously uploaded for this grant and host thread.
pub fn resolve_linked_thread_uploads(
    grant_id: &str,
    host_thread_id: &str,
    attachments: &[bruhs_models::coordination::linked_thread::LinkedAttachment],
) -> Result<Vec<UploadedFile>, &'static str> {
    use bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE;
    use std::collections::BTreeSet;
    if attachments.is_empty() {
        return Ok(Vec::new());
    }
    if attachments.len() > MAX_FILES_PER_MESSAGE {
        return Err(bruhs_models::logic::attachments::TOO_MANY_FILES);
    }
    let root = existing_attachment_root()
        .ok()
        .flatten()
        .ok_or("Attached files must be uploaded to this host first.")?;
    let prefix = linked_thread_upload_prefix(grant_id, host_thread_id);
    let mut seen = BTreeSet::new();
    let mut image_bytes = 0u64;
    let mut resolved = Vec::with_capacity(attachments.len());
    for (count, attachment) in attachments.iter().enumerate() {
        if !attachment.id.starts_with(&prefix)
            || attachment.id.contains(['/', '\\'])
            || attachment.id.len() > 255
            || !attachment
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            || !seen.insert(attachment.id.as_str())
        {
            return Err("Attached files must belong to this linked conversation.");
        }
        let path = root.join(&attachment.id);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|_| "An attached file is missing on this host.")?;
        if !meta.is_file() {
            return Err("Attached files must be regular uploads on this host.");
        }
        let canonical = path
            .canonicalize()
            .map_err(|_| "An attached file is missing on this host.")?;
        if canonical.parent() != Some(root.as_path()) || !canonical.is_file() {
            return Err("Attached files must stay inside this host's attachment folder.");
        }
        let label = upload_label(&attachment.name);
        let kind = admit(count, image_bytes, &label, meta.len())?;
        if kind == FileKind::Image {
            image_bytes = image_bytes.saturating_add(meta.len());
        }
        resolved.push(UploadedFile {
            name: label,
            path: canonical.to_string_lossy().into_owned(),
        });
    }
    Ok(resolved)
}

/// Retire browser answer files issued for this exact thread. Validate the
/// whole batch before unlinking anything so a malformed request is a no-op.
pub fn retire_web_question_uploads(
    request: &bruhs_models::logic::attachments::RetireQuestionFiles,
) -> std::io::Result<()> {
    use bruhs_models::logic::attachments::MAX_FILES_PER_MESSAGE;
    let prefix = web_question_upload_prefix(request.thread_id.as_str());
    if request.files.len() > MAX_FILES_PER_MESSAGE
        || request.files.iter().any(|name| {
            !name.starts_with(&prefix)
                || name.contains(['/', '\\'])
                || name.len() > 255
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Invalid answer file for this thread.",
        ));
    }
    let Some(root) = existing_attachment_root()? else {
        return Ok(());
    };
    for name in &request.files {
        remove_regular_upload(&root.join(name))?;
    }
    Ok(())
}

pub fn is_desktop_question_upload(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(DESKTOP_QUESTION_PREFIX))
}

/// Retire only desktop-owned answer files directly under the upload directory.
pub fn remove_desktop_question_upload(path: &str) -> std::io::Result<()> {
    let path = Path::new(path);
    if !path.is_absolute() || !path.to_str().is_some_and(is_desktop_question_upload) {
        return Ok(());
    }
    let Some(root) = existing_attachment_root()? else {
        return Ok(());
    };
    if path.parent() != Some(root.as_path()) {
        return Ok(());
    }
    remove_regular_upload(path)
}

fn existing_attachment_root() -> std::io::Result<Option<PathBuf>> {
    match media_output_directory()?.canonicalize() {
        Ok(root) => Ok(Some(root)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn remove_regular_upload(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => std::fs::remove_file(path),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_in(root: PathBuf, name: &str, bytes: &[u8]) -> Result<UploadedFile, &'static str> {
    write_in_with_prefix(root, name, bytes, "")
}

fn write_in_with_prefix(
    root: PathBuf,
    name: &str,
    bytes: &[u8],
    prefix: &str,
) -> Result<UploadedFile, &'static str> {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    admit(0, 0, name, len)?;
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let unique = upload_unique_id(SEQ.fetch_add(1, Ordering::Relaxed));
    let mut file_name = upload_disk_name(&format!("{prefix}{unique}"), name);
    // The receipt carries the full display name. Leave room for the owner
    // marker and retain the extension the viewer uses to recognize the file.
    if !prefix.is_empty() && file_name.len() > 255 {
        let extension = file_name
            .rsplit_once('.')
            .map(|(_, extension)| extension)
            .filter(|extension| extension.len() <= 16)
            .map(|extension| format!(".{extension}"))
            .unwrap_or_default();
        file_name.truncate(255 - extension.len());
        file_name.push_str(&extension);
    }
    if file_name.contains(['/', '\\']) {
        return Err(UPLOAD_DID_NOT_FINISH);
    }
    let path = root.join(&file_name);
    if !path.starts_with(&root) {
        return Err(UPLOAD_DID_NOT_FINISH);
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|_| UPLOAD_DID_NOT_FINISH)?;
    if file.write_all(bytes).is_err() {
        let _ = std::fs::remove_file(&path);
        return Err(UPLOAD_DID_NOT_FINISH);
    }
    let canonical = path.canonicalize().map_err(|_| {
        let _ = std::fs::remove_file(&path);
        UPLOAD_DID_NOT_FINISH
    })?;
    if !canonical.starts_with(&root) || !canonical.is_file() {
        let _ = std::fs::remove_file(&path);
        return Err(UPLOAD_DID_NOT_FINISH);
    }
    Ok(UploadedFile {
        name: upload_label(name),
        path: canonical.to_string_lossy().into_owned(),
    })
}

/// `true` when `path` is a regular file inside the attachment directory.
pub fn is_upload_path(path: &str) -> bool {
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return false;
    };
    if !canonical.is_file() {
        return false;
    }
    attachment_root().is_some_and(|root| canonical.starts_with(root))
        || std::env::temp_dir()
            .join(ATTACHMENT_DIR)
            .canonicalize()
            .is_ok_and(|root| canonical.starts_with(root))
}

/// Every path must be an upload this app wrote.
pub fn require_uploaded(files: &[UploadedFile]) -> Result<(), &'static str> {
    for file in files {
        if !is_upload_path(&file.path) {
            return Err("Attached files must be uploads from this app.");
        }
    }
    Ok(())
}

/// Open `path` for the viewer. A relative path joins `project`. An absolute
/// path is used as given, including outside the project.
pub fn open_for_viewer(path: &str, project: Option<&Path>) -> Result<OpenedFile, OpenError> {
    let raw = path.trim();
    if raw.is_empty() {
        return Err(OpenError::BadPath);
    }
    let joined = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        let project = project.ok_or(OpenError::NoProject)?;
        project.join(raw)
    };
    let canonical = std::fs::canonicalize(&joined).map_err(|_| OpenError::NotFound)?;
    if !canonical.is_file() {
        return Err(OpenError::NotAFile);
    }
    let len = std::fs::metadata(&canonical)
        .map_err(|_| OpenError::NotFound)?
        .len();
    if len > MAX_FILE_BYTES {
        return Err(OpenError::TooLarge);
    }
    let name = canonical
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| upload_label(raw));
    let kind = kind_for_name(&name);
    let mime = mime_for(kind, &name);
    let path = canonical.to_string_lossy().into_owned();
    if matches!(kind, FileKind::Text | FileKind::Html) {
        let (text, truncated) = read_text_preview(&canonical)?;
        return Ok(OpenedFile {
            name,
            path,
            kind,
            mime,
            truncated,
            text: Some(text),
            bytes: None,
        });
    }
    if kind == FileKind::Other {
        return Ok(OpenedFile {
            name,
            path,
            kind,
            mime,
            truncated: false,
            text: None,
            bytes: None,
        });
    }
    let bytes = std::fs::read(&canonical).map_err(|_| OpenError::Read)?;
    Ok(OpenedFile {
        name,
        path,
        kind,
        mime,
        truncated: false,
        text: None,
        bytes: Some(bytes),
    })
}

fn read_text_preview(path: &Path) -> Result<(String, bool), OpenError> {
    let mut file = std::fs::File::open(path).map_err(|_| OpenError::Read)?;
    let mut limited = (&mut file).take(u64::try_from(TEXT_PREVIEW_BYTES).unwrap_or(u64::MAX) + 1);
    let mut buf = Vec::new();
    limited.read_to_end(&mut buf).map_err(|_| OpenError::Read)?;
    let truncated = buf.len() > TEXT_PREVIEW_BYTES;
    if truncated {
        buf.truncate(TEXT_PREVIEW_BYTES);
    }
    Ok((String::from_utf8_lossy(&buf).into_owned(), truncated))
}

fn attachment_root() -> Option<PathBuf> {
    let dir = media_output_directory().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    dir.canonicalize().ok()
}

fn upload_unique_id(seq: u64) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{nanos:x}-{seq:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_stay_inside_the_attachment_directory() {
        let uploaded = write_upload("../secret.png", b"png").unwrap();
        assert_eq!(uploaded.name, "secret.png");
        let path = Path::new(&uploaded.path);
        let root = media_output_directory().unwrap().canonicalize().unwrap();
        assert!(path.starts_with(&root));
        assert!(is_upload_path(&uploaded.path));
        assert!(require_uploaded(std::slice::from_ref(&uploaded)).is_ok());
        assert_eq!(std::fs::read(path).unwrap(), b"png");
        assert!(require_uploaded(&[UploadedFile {
            name: "x".into(),
            path: "/etc/passwd".into(),
        }])
        .is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn heic_and_oversize_images_are_not_written() {
        assert!(write_upload("shot.heic", b"heic").is_err());
        assert!(write_upload("empty.txt", b"").is_err());
    }

    #[test]
    fn linked_thread_uploads_scope_binary_files_to_grant_and_thread() {
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01];
        let uploaded = write_linked_thread_upload("grant-a", "thread-1", "../shot.png", &png)
            .expect("binary png should upload");
        assert_eq!(uploaded.name, "shot.png");
        assert_eq!(uploaded.bytes, png.len() as u64);
        assert!(!uploaded.id.contains('/') && !uploaded.id.contains('\\'));
        let resolved =
            resolve_linked_thread_uploads("grant-a", "thread-1", std::slice::from_ref(&uploaded))
                .expect("owner grant and thread should resolve");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "shot.png");
        assert_eq!(std::fs::read(&resolved[0].path).unwrap(), png);
        assert!(resolve_linked_thread_uploads(
            "grant-b",
            "thread-1",
            std::slice::from_ref(&uploaded)
        )
        .is_err());
        assert!(resolve_linked_thread_uploads(
            "grant-a",
            "thread-2",
            std::slice::from_ref(&uploaded)
        )
        .is_err());
        assert!(resolve_linked_thread_uploads(
            "grant-a",
            "thread-1",
            &[uploaded.clone(), uploaded],
        )
        .is_err());
        let _ = std::fs::remove_file(&resolved[0].path);
    }

    #[test]
    fn the_viewer_reads_project_files_and_files_outside_it() {
        let root = std::env::temp_dir().join(format!("bruhs-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let docs = root.join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("a.md"), b"hello").unwrap();
        let outside =
            std::env::temp_dir().join(format!("bruhs-outside-{}.txt", std::process::id()));
        std::fs::write(&outside, b"out").unwrap();

        let opened = open_for_viewer("docs/a.md", Some(&root)).unwrap();
        assert_eq!(opened.kind, FileKind::Text);
        assert_eq!(opened.text.as_deref(), Some("hello"));
        assert!(!opened.truncated);

        let opened = open_for_viewer(outside.to_str().unwrap(), Some(&root)).unwrap();
        assert_eq!(opened.text.as_deref(), Some("out"));

        assert_eq!(
            open_for_viewer("docs/missing.md", Some(&root)).unwrap_err(),
            OpenError::NotFound
        );
        assert_eq!(
            open_for_viewer("docs", Some(&root)).unwrap_err(),
            OpenError::NotAFile
        );
        assert_eq!(
            open_for_viewer("docs/a.md", None).unwrap_err(),
            OpenError::NoProject
        );

        let big = root.join("big.txt");
        std::fs::write(&big, vec![b'a'; TEXT_PREVIEW_BYTES + 8]).unwrap();
        let opened = open_for_viewer(big.to_str().unwrap(), None).unwrap();
        assert!(opened.truncated);
        assert_eq!(opened.text.unwrap().len(), TEXT_PREVIEW_BYTES);

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }
}
