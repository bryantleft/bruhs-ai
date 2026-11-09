//! Pasted text the agent reads from disk. The bytes stay out of the prompt.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

use bruhs_models::logic::composer_text::{
    paste_disk_name, safe_paste_filename, TextAttachment, MAX_TEXT_ATTACHMENT_BYTES,
};

/// Write each body under the temp directory's `bruhs-pastes` folder.
/// The returned pairs are the label and the absolute path. A body that
/// cannot be written is omitted.
pub fn write_paste_files(items: &[TextAttachment]) -> Vec<(String, String)> {
    static SEQ: AtomicU64 = AtomicU64::new(1);
    let dir = std::env::temp_dir().join("bruhs-pastes");
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let Ok(root) = dir.canonicalize() else {
        return Vec::new();
    };
    let mut written = Vec::new();
    for item in items {
        if item.text.is_empty() || item.text.len() > MAX_TEXT_ATTACHMENT_BYTES {
            continue;
        }
        let unique = paste_unique_id(SEQ.fetch_add(1, Ordering::Relaxed));
        let file_name = paste_disk_name(&unique, &item.name);
        if file_name.contains(['/', '\\']) {
            continue;
        }
        let path = dir.join(&file_name);
        if !path.starts_with(&dir) {
            continue;
        }
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(_) => continue,
        };
        if file.write_all(item.text.as_bytes()).is_err() {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let Ok(canonical) = path.canonicalize() else {
            let _ = std::fs::remove_file(&path);
            continue;
        };
        if !canonical.starts_with(&root) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        written.push((
            paste_label(&item.name),
            canonical.to_string_lossy().into_owned(),
        ));
    }
    written
}

fn paste_unique_id(seq: u64) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{nanos:x}-{seq:x}")
}

fn paste_label(name: &str) -> String {
    let safe = safe_paste_filename(name);
    if safe == name {
        name.to_string()
    } else {
        safe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pasted_text_is_written_inside_the_temp_directory() {
        let files = write_paste_files(&[
            TextAttachment {
                name: "../secret".into(),
                text: "hello".into(),
            },
            TextAttachment {
                name: "notes.txt".into(),
                text: String::new(),
            },
        ]);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "paste.txt");
        let path = std::path::Path::new(&files[0].1);
        let root = std::env::temp_dir()
            .join("bruhs-pastes")
            .canonicalize()
            .unwrap();
        assert!(path.starts_with(&root));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "hello");
        let _ = std::fs::remove_file(path);
    }
}
