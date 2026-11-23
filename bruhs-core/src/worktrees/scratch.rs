//! Projectless chat folders. A separate lease file identifies folders bruhs
//! provisioned and keeps an active turn and cleanup mutually exclusive.
use super::{pending_path, program_in, programs, KeepWhen};
use std::future::Future;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn root() -> io::Result<PathBuf> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|dirs| dirs.data_dir().join("scratch"))
        .ok_or_else(|| io::Error::other("No storage folder for scratch chats."))
}

fn create_dir(path: &Path, recursive: bool) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

fn present(path: &Path) -> io::Result<bool> {
    match plain_dir(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn plain_dir(path: &Path) -> io::Result<()> {
    if path.symlink_metadata()?.is_dir() {
        Ok(())
    } else {
        Err(io::Error::other(
            "The scratch folder was moved or replaced.",
        ))
    }
}

fn marker(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("Missing scratch folder parent."))?;
    plain_dir(parent)?;
    let leases = parent.join(".leases");
    plain_dir(&leases)?;
    Ok(leases.join(
        path.file_name()
            .ok_or_else(|| io::Error::other("Missing scratch folder name."))?,
    ))
}

pub fn create(id: &str) -> io::Result<PathBuf> {
    let id = uuid::Uuid::parse_str(id).map_err(io::Error::other)?;
    let root = root()?;
    create_dir(&root, true)?;
    plain_dir(&root)?;
    create_dir(&root.join(".leases"), true)?;
    let path = root.join(id.to_string());
    let mark = marker(&path)?;
    let lease = fs::File::create_new(&mark)?;
    lease.lock()?;
    if let Err(e) = create_dir(&path, false) {
        drop(lease);
        fs::remove_file(mark)?;
        return Err(e);
    }
    Ok(path)
}

pub fn is_managed_path(path: &Path) -> bool {
    path.parent()
        .is_some_and(|parent| root().is_ok_and(|root| root == parent))
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok_and(|id| id.to_string() == name))
}

fn lease(path: &Path) -> io::Result<fs::File> {
    if !is_managed_path(path) {
        return Err(io::Error::other("Outside the managed scratch folder."));
    }
    let mark = marker(path)?;
    if !mark.symlink_metadata()?.is_file() {
        return Err(io::Error::other("Invalid scratch folder ownership record."));
    }
    fs::OpenOptions::new().read(true).write(true).open(mark)
}

/// Hold through the provider turn, including its startup. Reopening a retained
/// conversation gets a fresh folder after cleanup, or its original after a crash.
pub(crate) async fn for_turn(path: &Path) -> Result<Option<fs::File>, String> {
    if !is_managed_path(path) {
        return Ok(None);
    }
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let file = lease(&path).map_err(|e| e.to_string())?;
        file.lock_shared().map_err(|e| e.to_string())?;
        // A deleted thread may have retired this record while the turn was waiting.
        if !marker(&path).map_err(|e| e.to_string())?.is_file() {
            return Err("The scratch chat was deleted.".into());
        }
        match plain_dir(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let pending = pending_path(&path)?;
                match plain_dir(&pending) {
                    Ok(()) => fs::rename(pending, &path),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => create_dir(&path, false),
                    Err(e) => Err(e),
                }
                .or_else(|e| {
                    if plain_dir(&path).is_ok() {
                        Ok(())
                    } else {
                        Err(e)
                    }
                })
                .map_err(|e| e.to_string())?;
            }
            other => other.map_err(|e| e.to_string())?,
        }
        Ok(Some(file))
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn inspect(path: &Path, keep: KeepWhen) -> Result<(), String> {
    plain_dir(path).map_err(|e| e.to_string())?;
    if let Some(name) = program_in(&programs().await?, path) {
        return Err(format!("{name} uses the scratch folder."));
    }
    let root = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut dirs = vec![root.clone()];
        #[cfg(unix)]
        let device = {
            use std::os::unix::fs::MetadataExt;
            root.symlink_metadata().map_err(|e| e.to_string())?.dev()
        };
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let name = entry.file_name();
                if name == ".git" {
                    return Err("The scratch folder contains a Git checkout.".into());
                }
                if dir == root && keep != KeepWhen::TrackedChanges && name != "node_modules" {
                    return Err("The scratch folder has local files.".into());
                }
                let meta = entry.path().symlink_metadata().map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if meta.dev() != device {
                        return Err(
                            "The scratch folder contains another mounted filesystem.".into()
                        );
                    }
                }
                if meta.is_dir() {
                    dirs.push(entry.path());
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Removal {
    RetainChat,
    DeleteChat,
}

pub(super) async fn remove<F: Future<Output = Result<(), String>> + Send>(
    path: &Path,
    keep: KeepWhen,
    mode: Removal,
    validate: impl Fn() -> F + Send + Sync,
) -> Result<(), String> {
    let pending = pending_path(path)?;
    let file = match lease(path) {
        Ok(file) => file,
        Err(e)
            if mode == Removal::DeleteChat
                && e.kind() == io::ErrorKind::NotFound
                && is_managed_path(path)
                && matches!(present(path), Ok(false))
                && matches!(present(&pending), Ok(false)) =>
        {
            return validate().await
        }
        Err(e) => return Err(e.to_string()),
    };
    file.try_lock()
        .map_err(|_| "The scratch folder is in use.".to_owned())?;
    validate().await?;
    let retire = || -> Result<(), String> {
        if mode == Removal::DeleteChat {
            fs::remove_file(marker(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    match (present(path), present(&pending)) {
        (Ok(false), Ok(true)) => fs::rename(&pending, path).map_err(|e| e.to_string())?,
        (Ok(true), Ok(false)) => {}
        (Ok(false), Ok(false)) if mode == Removal::DeleteChat => return retire(),
        _ => return Err("The scratch folder could not be verified.".into()),
    }
    inspect(path, keep).await?;
    fs::rename(path, &pending).map_err(|e| e.to_string())?;
    let result = async {
        inspect(&pending, keep).await?;
        validate().await?;
        let disposable = super::staging_path(path)?;
        fs::rename(&pending, &disposable).map_err(|e| e.to_string())?;
        super::delete_in_background(disposable);
        Ok(())
    }
    .await;
    if let Err(reason) = result {
        if path.symlink_metadata().is_ok() || fs::rename(&pending, path).is_err() {
            return Err(format!(
                "{reason} Original files were kept at {}.",
                pending.display()
            ));
        }
        return Err(reason);
    }
    retire()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn scratch_containment_file_policy_and_rollback() {
        let name = "worktrees::scratch::tests::scratch_containment_file_policy_and_rollback";
        if std::env::var_os("BRUHS_SCRATCH_TEST_HOME").is_some() {
            tokio::runtime::Runtime::new().unwrap().block_on(exercise());
            return;
        }
        let home = tempfile::tempdir().unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name])
            .env("HOME", home.path())
            .env("XDG_DATA_HOME", home.path().join("data"))
            .env("BRUHS_SCRATCH_TEST_HOME", home.path())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[expect(clippy::unwrap_used, reason = "test helper")]
    async fn exercise() {
        let home = PathBuf::from(std::env::var_os("BRUHS_SCRATCH_TEST_HOME").unwrap());
        assert_eq!(std::env::var_os("HOME").unwrap(), home.as_os_str());
        let scratch = create(&uuid::Uuid::new_v4().to_string()).unwrap();
        let check = || std::future::ready(Ok(()));
        let remove = |keep| remove(&scratch, keep, Removal::RetainChat, check);
        let held = for_turn(&scratch).await.unwrap().unwrap();
        assert!(remove(KeepWhen::TrackedChanges).await.is_err());
        drop(held);

        fs::write(scratch.join("draft.txt"), "work").unwrap();
        assert!(remove(KeepWhen::UncommittedChanges).await.is_err());
        let checks = std::sync::atomic::AtomicUsize::new(0);
        assert!(super::remove(
            &scratch,
            KeepWhen::TrackedChanges,
            Removal::RetainChat,
            || {
                std::future::ready(
                    match checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                        0 => Ok(()),
                        _ => Err("settings changed".into()),
                    },
                )
            }
        )
        .await
        .is_err());
        assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            fs::read_to_string(scratch.join("draft.txt")).unwrap(),
            "work"
        );
        let inner = scratch.join("nested/.git");
        fs::create_dir_all(&inner).unwrap();
        assert!(remove(KeepWhen::TrackedChanges).await.is_err());
        fs::remove_dir_all(scratch.join("nested")).unwrap();
        let outside = home.join("project");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "file").unwrap();
        symlink(&outside, scratch.join("linked")).unwrap();
        remove(KeepWhen::TrackedChanges).await.unwrap();
        assert!(!scratch.exists());
        assert!(outside.join("keep").is_file());
        assert!(for_turn(&outside).await.unwrap().is_none());
        let held = for_turn(&scratch).await.unwrap().unwrap();
        assert!(scratch.is_dir() && !scratch.join("draft.txt").exists());
        drop(held);
        fs::rename(&scratch, pending_path(&scratch).unwrap()).unwrap();
        fs::write(
            pending_path(&scratch).unwrap().join("recovered"),
            "interrupted",
        )
        .unwrap();
        drop(for_turn(&scratch).await.unwrap());
        assert!(scratch.join("recovered").is_file());
        fs::remove_dir_all(&scratch).unwrap();
        symlink(&outside, &scratch).unwrap();
        assert!(remove(KeepWhen::TrackedChanges).await.is_err());
        assert!(for_turn(&scratch).await.is_err());
        assert!(outside.join("keep").is_file());
        fs::remove_file(&scratch).unwrap();
        for _ in 0..2 {
            super::remove(
                &scratch,
                KeepWhen::TrackedChanges,
                Removal::DeleteChat,
                check,
            )
            .await
            .unwrap();
        }
        assert!(for_turn(&scratch).await.is_err());
    }
}
