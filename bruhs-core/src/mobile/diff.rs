//! Read-only diff projection. No textconv, external diff, fsmonitor, filters or index writes.
use super::{MobileError, MobileResult, REQUEST_TIMEOUT};
use bruhs_models::mobile::{text_slice, MobileTextPage, TEXT_BYTES};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};
#[cfg(test)]
mod tests;
const LIMIT: usize = crate::git::DIFF_LIMIT_BYTES;

pub async fn read(
    dir: &Path,
    offset: usize,
    revision: Option<&str>,
) -> MobileResult<MobileTextPage> {
    let (bytes, truncated) = tokio::time::timeout(REQUEST_TIMEOUT, collect(dir))
        .await
        .map_err(|_| MobileError::Unavailable)??;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    let truncated = truncated || text.len() > LIMIT;
    bruhs_models::logic::text::truncate_utf8(&mut text, LIMIT);
    let actual = format!("{:x}", Sha256::digest(text.as_bytes()));
    if (offset > 0 && revision.is_none()) || revision.is_some_and(|r| r != actual) {
        return Err(MobileError::Conflict(
            "The working tree changed. Reload its diff from the first page.".into(),
        ));
    }
    let (part, next_offset) = text_slice(&text, offset, TEXT_BYTES)
        .map_err(|_| MobileError::Conflict("The diff page changed. Reload it.".into()))?;
    Ok(MobileTextPage {
        text: part.to_owned(),
        offset,
        next_offset,
        revision: actual,
        source_truncated: truncated,
    })
}
fn unavailable() -> MobileError {
    MobileError::Invalid(
        "This thread's Git diff is unavailable. Check its working folder on the laptop.".into(),
    )
}
async fn pipe<R: AsyncRead + Unpin>(reader: R, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(limit + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .await?;
    Ok(bytes)
}
async fn git(
    dir: &Path,
    args: &[&str],
    cap: usize,
    metadata: Option<&Path>,
) -> MobileResult<(Vec<u8>, bool)> {
    run_git(dir, args, cap, metadata, false)
        .await
        .map(|(bytes, status)| (bytes, status == Some(0)))
}
async fn run_git(
    dir: &Path,
    args: &[&str],
    cap: usize,
    metadata: Option<&Path>,
    read_excludes_config: bool,
) -> MobileResult<(Vec<u8>, Option<i32>)> {
    if read_excludes_config
        && (args != ["config", "--path", "--get", "core.excludesFile"] || metadata.is_some())
    {
        return Err(unavailable());
    }
    let mut command = Command::new("git");
    // Inherited Git configuration cannot redirect this request to a different repository/helper.
    for (key, _) in std::env::vars_os().filter(|(key, _)| key.to_string_lossy().starts_with("GIT_"))
    {
        command.env_remove(key);
    }
    command
        .current_dir(dir)
        .args([
            "--no-pager",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.pager=cat",
            "-c",
            "core.untrackedCache=false",
        ])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if !read_excludes_config {
        command
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null");
    }
    if let Some(metadata) = metadata {
        command.env("GIT_DIR", metadata).env("GIT_WORK_TREE", dir);
    }
    let mut child = command.spawn().map_err(|_| unavailable())?;
    let stdout = child.stdout.take().ok_or_else(unavailable)?;
    let stderr = child.stderr.take().ok_or_else(unavailable)?;
    let (output, errors, status) =
        tokio::join!(pipe(stdout, cap), pipe(stderr, 16 * 1024), child.wait());
    let bytes = output.map_err(|_| unavailable())?;
    errors.map_err(|_| unavailable())?;
    let code = status.map_err(|_| unavailable())?.code();
    Ok((bytes, code))
}
async fn collect(dir: &Path) -> MobileResult<(Vec<u8>, bool)> {
    let (root, ok) = git(
        dir,
        &["rev-parse", "--show-toplevel"],
        bruhs_models::mobile::RESPONSE_BYTES,
        None,
    )
    .await?;
    let root = String::from_utf8(root).map_err(|_| unavailable())?;
    let root = root.strip_suffix('\n').ok_or_else(unavailable)?;
    if !ok || root.contains(['\r', '\n']) || !Path::new(root).is_absolute() {
        return Err(unavailable());
    }
    let dir = Path::new(root);
    // Git's clean/process filters also run during working-tree comparison. A temporary
    // repository owns configuration and the index snapshot, while objects are read through
    // alternates. Later changes to the original config/includes cannot introduce helpers.
    let (paths, ok) = git(
        dir,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
            "--git-path",
            "index",
            "--git-path",
            "info/exclude",
            "--shared-index-path",
        ],
        bruhs_models::mobile::RESPONSE_BYTES,
        None,
    )
    .await?;
    if !ok || paths.len() > bruhs_models::mobile::RESPONSE_BYTES {
        return Err(unavailable());
    }
    let paths = std::str::from_utf8(&paths)
        .map_err(|_| unavailable())?
        .lines()
        .collect::<Vec<_>>();
    if !(3..=4).contains(&paths.len()) {
        return Err(unavailable());
    }
    let objects = PathBuf::from(paths[0]);
    let index = PathBuf::from(paths[1]);
    let exclude = PathBuf::from(paths[2]);
    let shared = paths.get(3).filter(|s| !s.is_empty()).map(PathBuf::from);
    let (excludes, status) = run_git(
        dir,
        &["config", "--path", "--get", "core.excludesFile"],
        bruhs_models::mobile::RESPONSE_BYTES,
        None,
        true,
    )
    .await?;
    if !matches!(status, Some(0 | 1)) {
        return Err(unavailable());
    }
    if excludes.len() > bruhs_models::mobile::RESPONSE_BYTES {
        return Err(unavailable());
    }
    let excludes = if status == Some(0) {
        Some(
            String::from_utf8(excludes)
                .map_err(|_| unavailable())?
                .strip_suffix('\n')
                .ok_or_else(unavailable)?
                .to_owned(),
        )
    } else if excludes.is_empty() {
        None
    } else {
        return Err(unavailable());
    };
    let (format, ok) = git(dir, &["rev-parse", "--show-object-format"], 128, None).await?;
    let format = String::from_utf8(format).map_err(|_| unavailable())?;
    if !ok || !matches!(format.trim(), "sha1" | "sha256") {
        return Err(unavailable());
    }
    let (head, has_head) = git(dir, &["rev-parse", "--verify", "HEAD"], 128, None).await?;
    let head = String::from_utf8(head).map_err(|_| unavailable())?;
    if has_head
        && (head.trim().len() != if format.trim() == "sha1" { 40 } else { 64 }
            || !head.trim().bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(unavailable());
    }
    let metadata = tokio::task::spawn_blocking(move || {
        snapshot_metadata(
            &objects,
            &index,
            &exclude,
            shared.as_deref(),
            &format,
            has_head.then_some(head.trim()),
        )
    })
    .await
    .map_err(|_| MobileError::Unavailable)??;
    if let Some(excludes) = excludes {
        let config = metadata.path().join("config");
        let config = config.to_str().ok_or_else(unavailable)?;
        let (_, ok) = git(
            dir,
            &["config", "--file", config, "core.excludesFile", &excludes],
            bruhs_models::mobile::RESPONSE_BYTES,
            Some(metadata.path()),
        )
        .await?;
        if !ok {
            return Err(unavailable());
        }
    }
    let mut base = "HEAD".to_owned();
    if !has_head {
        let (empty, ok) = git(
            dir,
            &["hash-object", "-w", "-t", "tree", "--stdin"],
            128,
            Some(metadata.path()),
        )
        .await?;
        if !ok {
            return Err(unavailable());
        }
        base = String::from_utf8(empty)
            .map_err(|_| unavailable())?
            .trim()
            .to_owned();
    }
    let flags = [
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--ignore-submodules=all",
        "--no-renames",
        "--",
    ];
    let head = [&["diff", base.as_str()][..], &flags].concat();
    let (mut bytes, success) = git(dir, &head, LIMIT, Some(metadata.path())).await?;
    if bytes.len() > LIMIT {
        bytes.truncate(LIMIT);
        return Ok((bytes, true));
    }
    if !success {
        return Err(unavailable());
    }
    let (mut paths, success) = git(
        dir,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        bruhs_models::mobile::RESPONSE_BYTES,
        Some(metadata.path()),
    )
    .await?;
    if !success && paths.len() <= bruhs_models::mobile::RESPONSE_BYTES {
        return Err(unavailable());
    }
    let mut truncated = paths.len() > bruhs_models::mobile::RESPONSE_BYTES;
    if truncated {
        let end = paths.iter().rposition(|b| *b == 0).map_or(0, |i| i + 1);
        paths.truncate(end);
    }
    let source = dir.to_path_buf();
    let remaining = LIMIT - bytes.len();
    let additions = tokio::task::spawn_blocking(move || untracked(&source, &paths, remaining))
        .await
        .map_err(|_| MobileError::Unavailable)??;
    truncated |= additions.1;
    bytes.extend(additions.0);
    Ok((bytes, truncated))
}
fn snapshot_metadata(
    objects: &Path,
    index: &Path,
    exclude: &Path,
    shared: Option<&Path>,
    format: &str,
    head: Option<&str>,
) -> MobileResult<tempfile::TempDir> {
    let root = tempfile::tempdir().map_err(|_| unavailable())?;
    std::fs::create_dir_all(root.path().join("objects/info")).map_err(|_| unavailable())?;
    std::fs::create_dir(root.path().join("refs")).map_err(|_| unavailable())?;
    std::fs::create_dir(root.path().join("info")).map_err(|_| unavailable())?;
    let config = if format.trim() == "sha256" {
        "[core]\nrepositoryformatversion = 1\nbare = false\n[extensions]\nobjectFormat = sha256\n"
    } else {
        "[core]\nrepositoryformatversion = 0\nbare = false\n"
    };
    std::fs::write(root.path().join("config"), config).map_err(|_| unavailable())?;
    let object_path = objects
        .to_str()
        .filter(|s| !s.contains(['\r', '\n']))
        .ok_or_else(unavailable)?;
    std::fs::write(
        root.path().join("objects/info/alternates"),
        format!("{object_path}\n"),
    )
    .map_err(|_| unavailable())?;
    std::fs::write(
        root.path().join("HEAD"),
        head.map_or_else(
            || "ref: refs/heads/mobile-unborn\n".into(),
            |head| format!("{head}\n"),
        ),
    )
    .map_err(|_| unavailable())?;
    copy_metadata_file(exclude, &root.path().join("info/exclude"), true, true)?;
    copy_metadata_file(index, &root.path().join("index"), true, false)?;
    if let Some(shared) = shared {
        let name = shared
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(unavailable)?;
        let suffix = name.strip_prefix("sharedindex.").ok_or_else(unavailable)?;
        let expected = if format.trim() == "sha1" { 40 } else { 64 };
        if suffix.len() != expected || !suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(unavailable());
        }
        copy_metadata_file(shared, &root.path().join(name), false, false)?;
    }
    Ok(root)
}
fn copy_metadata_file(
    source: &Path,
    target: &Path,
    missing_ok: bool,
    follow: bool,
) -> MobileResult<()> {
    use std::io::{Read, Write};
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK | if follow { 0 } else { libc::O_NOFOLLOW });
    }
    #[cfg(windows)]
    if !follow {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the reparse point itself so the is_file check below rejects links.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let input = match options.open(source) {
        Ok(input) => input,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && missing_ok => return Ok(()),
        Err(_) => return Err(unavailable()),
    };
    if !input.metadata().map_err(|_| unavailable())?.is_file() {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    input
        .take(u64::try_from(LIMIT + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|_| unavailable())?;
    if bytes.len() > LIMIT {
        return Err(MobileError::Invalid("A Git index or ignore file exceeds the 2 MiB mobile preview budget. Review the diff on the laptop.".into()));
    }
    let mut output = std::fs::File::create(target).map_err(|_| unavailable())?;
    output.write_all(&bytes).map_err(|_| unavailable())?;
    Ok(())
}
fn untracked(dir: &Path, paths: &[u8], budget: usize) -> MobileResult<(Vec<u8>, bool)> {
    use std::io::Read;
    let mut result = Vec::new();
    let mut truncated = false;
    for (count, raw) in paths
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .enumerate()
    {
        if count >= crate::git::UNTRACKED_LIMIT {
            truncated = true;
            break;
        }
        let name = String::from_utf8_lossy(raw);
        let header = format!(
            "\ndiff --git a/{name} b/{name}\nnew untracked file\n--- /dev/null\n+++ b/{name}\n"
        );
        if result.len() + header.len() > budget {
            truncated = true;
            break;
        }
        result.extend(header.as_bytes());
        let mut file = match confined_file(dir, raw) {
            Ok(file) => file,
            Err(_) => {
                let note = b"[file not read by the confined preview; review on the laptop]\n";
                if result.len() + note.len() <= budget {
                    result.extend(note);
                } else {
                    truncated = true;
                }
                continue;
            }
        };
        let mut data = Vec::new();
        let cap = budget.saturating_sub(result.len());
        file.by_ref()
            .take(u64::try_from(cap + 1).unwrap_or(u64::MAX))
            .read_to_end(&mut data)
            .map_err(|_| unavailable())?;
        let binary = data.contains(&0);
        if binary {
            let note = b"[binary content not displayed]\n";
            if result.len() + note.len() <= budget {
                result.extend(note);
            } else {
                truncated = true;
            }
            continue;
        }
        for line in data.split_inclusive(|b| *b == b'\n') {
            if result.len() + 1 + line.len() > budget {
                truncated = true;
                break;
            }
            result.push(b'+');
            result.extend(line);
        }
        if data.len() > cap {
            truncated = true;
        }
        if truncated {
            break;
        }
    }
    Ok((result, truncated))
}
#[cfg(unix)]
pub(super) fn confined_file(dir: &Path, raw: &[u8]) -> std::io::Result<std::fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(raw));
    let components = path.components().collect::<Vec<_>>();
    if components.is_empty()
        || components
            .iter()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(std::io::Error::other("invalid repository path"));
    }
    let mut parent = std::fs::File::open(dir)?;
    for (index, component) in components.iter().enumerate() {
        let name = CString::new(component.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
        let last = index + 1 == components.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if last {
                libc::O_NONBLOCK
            } else {
                libc::O_DIRECTORY
            };
        // SAFETY: valid directory fd and NUL-terminated path; ownership transfers once on success.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: openat returned a new valid descriptor, transferred exactly once to File.
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        if last {
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::other("not a regular file"));
            }
            return Ok(file);
        }
        parent = file;
    }
    Err(std::io::Error::other("invalid repository path"))
}
#[cfg(not(unix))]
pub(super) fn confined_file(_dir: &Path, _raw: &[u8]) -> std::io::Result<std::fs::File> {
    // A path-only check would race junctions. Tracked diffs still work on this platform.
    Err(std::io::Error::other(
        "untracked mobile preview requires a confined file handle",
    ))
}
