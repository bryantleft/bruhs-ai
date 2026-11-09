//! Files the user attached, and files the viewer opens from a path.
//!
//! Uploads land under the temp directory's `bruhs-attachments` folder. A
//! turn may only point the agent at files written there. Opening a path is
//! read-only and may leave the project: the viewer follows the file the
//! agent named.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use bruhs_models::logic::attachments::{
    admit, kind_for_name, mime_for, upload_disk_name, upload_label, FileKind, UploadedFile,
    COULD_NOT_READ, FILE_TOO_LARGE, MAX_FILE_BYTES, TEXT_PREVIEW_BYTES, UPLOAD_DID_NOT_FINISH,
};

const ATTACHMENT_DIR: &str = "bruhs-attachments";

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
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    admit(0, 0, name, len)?;
    let root = attachment_root().ok_or(UPLOAD_DID_NOT_FINISH)?;
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let unique = upload_unique_id(SEQ.fetch_add(1, Ordering::Relaxed));
    let file_name = upload_disk_name(&unique, name);
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
    let Some(root) = attachment_root() else {
        return false;
    };
    let Ok(canonical) = std::fs::canonicalize(path) else {
        return false;
    };
    canonical.starts_with(&root) && canonical.is_file()
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
    let dir = std::env::temp_dir().join(ATTACHMENT_DIR);
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
        let root = std::env::temp_dir()
            .join(ATTACHMENT_DIR)
            .canonicalize()
            .unwrap();
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
