//! Text reads selected by a live question link. The same confined file
//! handles as mobile diffs prevent a path from crossing the working folder.
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};

use bruhs_models::mobile::{text_slice, MobileProjectLink, MobileTextPage, TEXT_BYTES};
use sha2::{Digest, Sha256};

use super::{MobileError, MobileResult, REQUEST_TIMEOUT};

pub async fn read(
    root: PathBuf,
    link: MobileProjectLink,
    offset: usize,
    revision: Option<String>,
) -> MobileResult<MobileTextPage> {
    tokio::time::timeout(
        REQUEST_TIMEOUT,
        tokio::task::spawn_blocking(move || page(&root, link, offset, revision.as_deref())),
    )
    .await
    .map_err(|_| MobileError::Unavailable)?
    .map_err(|_| MobileError::Unavailable)?
}

fn page(
    root: &Path,
    link: MobileProjectLink,
    offset: usize,
    revision: Option<&str>,
) -> MobileResult<MobileTextPage> {
    let path = Path::new(&link.path);
    let root = root.canonicalize().map_err(|_| unavailable())?;
    let path = if path.is_absolute() {
        path.strip_prefix(&root).map_err(|_| unavailable())?
    } else {
        path
    };
    if path.components().any(|component| match component {
        Component::Normal(name) => {
            let name = name.to_string_lossy().to_ascii_lowercase();
            matches!(
                name.as_str(),
                ".git" | ".ssh" | ".gnupg" | ".aws" | ".netrc" | ".npmrc"
            ) || name.starts_with(".env")
                || name.ends_with(".pem")
                || name.ends_with(".key")
        }
        Component::CurDir => false,
        _ => true,
    }) {
        return Err(unavailable());
    }
    let path = path.strip_prefix(".").unwrap_or(path);
    let bytes = path.to_str().ok_or_else(unavailable)?.as_bytes();
    let mut input = super::diff::confined_file(&root, bytes).map_err(|_| unavailable())?;
    let limit = crate::git::DIFF_LIMIT_BYTES;
    let mut data = Vec::new();
    input
        .by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut data)
        .map_err(|_| unavailable())?;
    let source_truncated = data.len() > limit;
    data.truncate(limit);
    if data.contains(&0) {
        return Err(unavailable());
    }
    let text = match std::str::from_utf8(&data) {
        Ok(text) => text,
        Err(error) if source_truncated && error.error_len().is_none() => {
            std::str::from_utf8(&data[..error.valid_up_to()]).map_err(|_| unavailable())?
        }
        Err(_) => return Err(unavailable()),
    };
    let actual = format!("{:x}", Sha256::digest(text.as_bytes()));
    if (offset > 0 && revision.is_none()) || revision.is_some_and(|expected| expected != actual) {
        return Err(MobileError::Conflict(
            "This file changed. Reload the first page.".into(),
        ));
    }
    let offset = if offset == 0 {
        let preceding = link.line.unwrap_or(1).saturating_sub(1);
        let lines = text
            .split_inclusive('\n')
            .take(preceding)
            .map(str::len)
            .sum::<usize>();
        if lines < text.len() {
            lines
        } else {
            0
        }
    } else {
        offset
    };
    let (text, next_offset) = text_slice(text, offset, TEXT_BYTES)
        .map_err(|_| MobileError::Conflict("This file changed. Reload the first page.".into()))?;
    Ok(MobileTextPage {
        text: text.to_owned(),
        offset,
        next_offset,
        revision: actual,
        source_truncated,
    })
}

fn unavailable() -> MobileError {
    MobileError::Invalid(
        "This link cannot be previewed as a project text file on the phone.".into(),
    )
}
