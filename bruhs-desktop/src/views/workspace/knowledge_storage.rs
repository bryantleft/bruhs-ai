//! Knowledge attachment payloads on disk.
//!
//! Files are private to the user (0600 in a 0700 directory), size-checked
//! before they are read, and named only by validated attachment ids, so an
//! id from a tampered state file can never reach outside the directory.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Largest attachment accepted.
pub(super) const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
/// Longest file extension kept on an attachment id (`.markdown`).
const MAX_EXTENSION_CHARS: usize = 16;

#[derive(Debug, thiserror::Error)]
pub(super) enum AttachmentError {
    #[error("Could not read that attachment.")]
    Read(#[source] io::Error),
    #[error("Attachments must be 10 MB or smaller.")]
    TooLarge,
    #[error("Could not locate local app storage.")]
    NoStorage,
    #[error("Could not save the attachment: {0}")]
    Write(#[source] io::Error),
}

/// A new attachment id: a UUID plus the source file's extension.
pub(super) fn new_attachment_id(source: &Path) -> String {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| valid_extension(value))
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    format!("{}{extension}", uuid::Uuid::new_v4())
}

/// What a new id may carry as its extension (alphanumeric, short).
fn valid_extension(extension: &str) -> bool {
    !extension.is_empty()
        && extension.chars().count() <= MAX_EXTENSION_CHARS
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Whether `id` names an attachment payload this app may open or delete: a
/// hyphenated UUID, optionally followed by `.` and an extension.
///
/// Earlier versions kept the source file's extension verbatim (`.jpg_large`,
/// `.c++`, very long ones), so any extension is accepted here as long as it
/// stays one plain file name inside the attachments directory: no `/`, `\`
/// or NUL, no `..`, and no second leading `.`. New ids are stricter (see
/// [`new_attachment_id`]).
pub(super) fn is_valid_attachment_id(id: &str) -> bool {
    let (uuid, extension) = match id.split_once('.') {
        Some((uuid, extension)) => (uuid, Some(extension)),
        None => (id, None),
    };
    uuid.len() == 36
        && uuid::Uuid::try_parse(uuid).is_ok()
        && extension.is_none_or(stored_extension_is_safe)
}

/// An extension from any version: confined to a single path component.
fn stored_extension_is_safe(extension: &str) -> bool {
    !extension.is_empty()
        && !extension.starts_with('.')
        && !extension.contains("..")
        && !extension.contains(['/', '\\', '\0'])
}

/// The payload file of attachment `id` inside `dir`; `None` for an id this
/// app would never have created.
pub(super) fn attachment_path_in(dir: &Path, id: &str) -> Option<PathBuf> {
    is_valid_attachment_id(id).then(|| dir.join(id))
}

/// The payload file of attachment `id` in the app's storage.
pub(super) fn attachment_path(id: &str) -> Option<PathBuf> {
    crate::paths::knowledge_attachments_dir().and_then(|dir| attachment_path_in(&dir, id))
}

/// Copy `source` into `dir` as attachment `id`; returns its size.
pub(super) fn store_attachment(
    source: &Path,
    dir: &Path,
    id: &str,
) -> Result<u64, AttachmentError> {
    let target = attachment_path_in(dir, id).ok_or(AttachmentError::NoStorage)?;
    let file = fs::File::open(source).map_err(AttachmentError::Read)?;
    let size = file.metadata().map_err(AttachmentError::Read)?.len();
    if size > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentError::TooLarge);
    }
    // The file can grow between the check and the read; never take more.
    let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or_default());
    file.take(MAX_ATTACHMENT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(AttachmentError::Read)?;
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        return Err(AttachmentError::TooLarge);
    }
    create_private_dir(dir).map_err(AttachmentError::Write)?;
    let mut out = private_file_options()
        .open(&target)
        .map_err(AttachmentError::Write)?;
    out.write_all(&bytes)
        .and_then(|()| out.sync_all())
        .map_err(AttachmentError::Write)?;
    Ok(bytes.len() as u64)
}

/// Delete the payloads of `ids` (ids that are invalid or already gone are
/// skipped). Failures are logged: the document is already deleted.
pub(super) fn remove_attachments(dir: &Path, ids: &[String]) {
    for id in ids {
        let Some(path) = attachment_path_in(dir, id) else {
            tracing::warn!("not deleting attachment with unexpected id {id:?}");
            continue;
        };
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!("could not delete {}: {error}", path.display()),
        }
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    // An existing directory from an earlier version may be world-readable.
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)
}

fn private_file_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    // `create_new` never follows or replaces an existing file.
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";

    #[test]
    fn malformed_or_escaping_ids_are_invalid() {
        assert!(is_valid_attachment_id(ID));
        assert!(is_valid_attachment_id(&format!("{ID}.pdf")));
        for bad in [
            "",
            "..",
            "../desktop-state.json",
            "/etc/passwd",
            &format!("{ID}/../../x"),
            &format!("{ID}.p/df"),
            &format!("{ID}."),
            "0f8fad5bd9cb469fa16570867728950e",
            "not-a-uuid-at-all-not-a-uuid-at-all",
        ] {
            assert!(!is_valid_attachment_id(bad), "{bad:?}");
            assert!(attachment_path_in(Path::new("/tmp/a"), bad).is_none());
        }
    }

    #[test]
    fn ids_from_earlier_versions_keep_resolving() {
        for legacy in [
            format!("{ID}.jpg_large"),
            format!("{ID}.c++"),
            format!("{ID}.tar-1"),
            format!("{ID}.{}", "x".repeat(40)),
            format!("{ID}.tar.gz"),
            format!("{ID}.résumé"),
        ] {
            assert!(is_valid_attachment_id(&legacy), "{legacy:?}");
            assert_eq!(
                attachment_path_in(Path::new("/tmp/a"), &legacy),
                Some(Path::new("/tmp/a").join(&legacy))
            );
        }
        for bad in [
            format!("{ID}./etc/passwd"),
            format!("{ID}.x/../../y"),
            format!("{ID}.x\\..\\y"),
            format!("{ID}..x"),
            format!("{ID}.x..y"),
            format!("{ID}..."),
            format!("{ID}.x\0y"),
        ] {
            assert!(!is_valid_attachment_id(&bad), "{bad:?}");
        }
    }

    #[test]
    fn a_legacy_attachment_is_deleted_with_its_document() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("attachments");
        fs::create_dir(&dir).unwrap();
        let legacy = format!("{ID}.jpg_large");
        fs::write(dir.join(&legacy), b"payload").unwrap();

        remove_attachments(&dir, std::slice::from_ref(&legacy));

        assert!(!dir.join(&legacy).exists());
    }

    #[test]
    fn new_ids_are_valid_and_keep_a_safe_extension() {
        let id = new_attachment_id(Path::new("/x/report.pdf"));
        assert!(id.ends_with(".pdf") && is_valid_attachment_id(&id));
        let odd = new_attachment_id(Path::new("/x/notes.we!rd"));
        assert!(is_valid_attachment_id(&odd));
        assert!(!odd.contains('.'));
    }

    #[test]
    fn stored_attachments_are_private_copies() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("report.pdf");
        fs::write(&source, b"%PDF-1.7").unwrap();
        let dir = temp.path().join("attachments");
        let id = format!("{ID}.pdf");

        let size = store_attachment(&source, &dir, &id).unwrap();

        assert_eq!(size, 8);
        assert_eq!(fs::read(dir.join(&id)).unwrap(), b"%PDF-1.7");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&dir.join(&id)), 0o600);
        }
        // An id is written once; a second store cannot replace it.
        assert!(matches!(
            store_attachment(&source, &dir, &id),
            Err(AttachmentError::Write(_))
        ));
    }

    #[test]
    fn oversized_files_are_refused_before_they_are_read() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("huge.bin");
        let file = fs::File::create(&source).unwrap();
        // Sparse: the size check must not need the bytes.
        file.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();
        let dir = temp.path().join("attachments");
        assert!(matches!(
            store_attachment(&source, &dir, ID),
            Err(AttachmentError::TooLarge)
        ));
        assert!(!dir.exists(), "nothing is written for a refused file");
    }

    #[test]
    fn removal_skips_invalid_ids_and_missing_files() {
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("keep.txt");
        fs::write(&outside, b"keep").unwrap();
        let dir = temp.path().join("attachments");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join(ID), b"payload").unwrap();

        remove_attachments(
            &dir,
            &[
                ID.to_string(),
                "../keep.txt".to_string(),
                format!("{ID}.gone"),
            ],
        );

        assert!(!dir.join(ID).exists());
        assert!(outside.exists());
    }
}
