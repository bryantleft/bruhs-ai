//! Browser screenshots have their own retention boundary inside uploads.
use super::{media_output_directory, write_in, UploadedFile, UPLOAD_DID_NOT_FINISH};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub const MAX_DAYS: u16 = 3650;
const FOLDER: &str = "browser-captures";

fn directory(root: &Path, create: bool) -> io::Result<PathBuf> {
    let path = root.join(FOLDER);
    if create {
        fs::create_dir_all(root)?;
        match fs::create_dir(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
    }
    if !fs::symlink_metadata(&path)?.is_dir() {
        return Err(io::Error::other(
            "Browser capture storage must be a directory on this host.",
        ));
    }
    let canonical = path.canonicalize()?;
    if canonical != root.canonicalize()?.join(FOLDER) {
        return Err(io::Error::other("The browser capture folder changed."));
    }
    Ok(canonical)
}

pub fn write(name: &str, bytes: &[u8]) -> Result<UploadedFile, &'static str> {
    let root = media_output_directory().map_err(|_| UPLOAD_DID_NOT_FINISH)?;
    write_in(
        directory(&root, true).map_err(|_| UPLOAD_DID_NOT_FINISH)?,
        name,
        bytes,
    )
}

fn policy_path() -> io::Result<PathBuf> {
    let root = media_output_directory()?;
    Ok(root.join(".browser-capture-retention.json"))
}

/// Zero keeps captures indefinitely. Existing installations have no policy file.
pub fn days() -> io::Result<u16> {
    match fs::read(policy_path()?) {
        Ok(bytes) => {
            let value: u16 = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if value > MAX_DAYS {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid browser capture retention period.",
                ));
            }
            Ok(value)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

pub fn set_days(days: u16) -> io::Result<u16> {
    if days > MAX_DAYS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Choose 1 to 3650 days, or turn off automatic deletion.",
        ));
    }
    use std::io::Write;
    let path = policy_path()?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing browser storage folder."))?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    writeln!(file, "{days}")?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(days)
}

pub fn prune() -> io::Result<usize> {
    prune_in(&media_output_directory()?, days()?, SystemTime::now())
}

fn prune_in(root: &Path, days: u16, now: SystemTime) -> io::Result<usize> {
    if days == 0 {
        return Ok(0);
    }
    let folder = match directory(root, false) {
        Ok(folder) => folder,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let age = Duration::from_secs(u64::from(days) * 86400);
    let mut count = 0;
    for file in fs::read_dir(folder)? {
        let file = file?;
        let name = file.file_name();
        let recognized = name
            .to_str()
            .and_then(|s| s.strip_suffix("-verification.png"))
            .and_then(|s| s.split_once('-'))
            .is_some_and(|(time, sequence)| {
                [time, sequence]
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_hexdigit()))
            });
        if !recognized || !file.file_type()?.is_file() {
            continue;
        }
        let metadata = match fs::symlink_metadata(file.path()) {
            Ok(info) => info,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if !metadata.is_file()
            || !now
                .duration_since(metadata.modified()?)
                .is_ok_and(|elapsed| elapsed >= age)
        {
            continue;
        }
        match fs::remove_file(file.path()) {
            Ok(()) => count += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => (),
            Err(e) => return Err(e),
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_expired_browser_captures_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = directory(tmp.path(), true).unwrap();
        let old = folder.join("1234-ab-verification.png");
        let fresh = folder.join("1235-cd-verification.png");
        let legacy = tmp.path().join("1234-ab-verification.png");
        let user = folder.join("personal.png");
        for file in [&old, &fresh, &legacy, &user] {
            fs::write(file, b"fixture").unwrap();
        }
        let now = SystemTime::now();
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(now - Duration::from_secs(2 * 86400))
            .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&legacy, folder.join("123-a-verification.png")).unwrap();
        assert_eq!(prune_in(tmp.path(), 0, now).unwrap(), 0);
        assert_eq!(prune_in(tmp.path(), 1, now).unwrap(), 1);
        assert!(!old.exists() && fresh.exists() && legacy.exists() && user.exists());
        assert_eq!(prune_in(tmp.path(), 1, now).unwrap(), 0);
    }
}
