//! A paired host can receive a rename while its desktop is closed. Keep the edits
//! beside the workspace until its normal guarded save has incorporated them.
use super::{Checkout, saved_projects};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

pub type Titles = BTreeMap<String, Option<String>>;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    pub expected: Titles,
    pub title: Option<String>,
}

fn sidecar(state: &Path, suffix: &str) -> PathBuf {
    let mut name = state.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn pending(state: &Path) -> io::Result<Titles> {
    match fs::File::open(sidecar(state, ".project-names.json")) {
        Ok(file) => Ok(serde_json::from_reader(io::BufReader::new(file))?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Titles::new()),
        Err(e) => Err(e),
    }
}

fn write(state: &Path, names: &Titles) -> io::Result<()> {
    let path = sidecar(state, ".project-names.json");
    if names.is_empty() {
        fs::remove_file(path)?;
    } else {
        let mut file = tempfile::NamedTempFile::new_in(state.parent().unwrap_or(Path::new(".")))?;
        serde_json::to_writer(&mut file, names)?;
        file.flush()?;
        file.as_file().sync_all()?;
        file.persist(path)?;
    }
    #[cfg(unix)]
    fs::File::open(state.parent().unwrap_or(Path::new(".")))?.sync_all()?;
    Ok(())
}

pub(super) fn overlay(names: &Titles, projects: &mut [Checkout]) {
    for p in projects {
        if let Some(title) = names.get(&p.path) {
            p.title.clone_from(title);
        }
        if let Some(title) = &p.title {
            p.name.clone_from(title);
        }
    }
}

impl Rename {
    pub fn validate(&self) -> io::Result<()> {
        if self.expected.is_empty() || self.expected.keys().any(|p| !Path::new(p).is_absolute()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose a saved checkout.",
            ));
        }
        if self.title.as_deref().is_some_and(|s| {
            s.trim().is_empty() || s.trim() != s || s.chars().any(char::is_control)
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Enter a single-line project name.",
            ));
        }
        Ok(())
    }
}

fn names_lock(state: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(sidecar(state, ".project-names.lock"))?;
    file.lock()?;
    Ok(file)
}

/// Only the desktop may also edit a checkout that it has just created in memory.
pub fn rename(state: &Path, edit: &Rename, local: bool) -> io::Result<()> {
    edit.validate()?;
    let parent = state.parent().unwrap_or(Path::new("."));
    if local {
        fs::create_dir_all(parent)?;
    }
    let _lock = names_lock(state)?;
    let projects = saved_projects(state.to_owned())?;
    let disk: BTreeMap<_, _> = projects.into_iter().map(|p| (p.path, p.title)).collect();
    let mut names = pending(state)?;
    for (path, expected) in &edit.expected {
        let Some(saved) = disk.get(path) else {
            if local {
                continue;
            }
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "This checkout is no longer saved on the host.",
            ));
        };
        let current = names.get(path).unwrap_or(saved);
        if current != expected && current != &edit.title {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "The project name changed. Review the current name and retry.",
            ));
        }
    }
    let old = names.clone();
    for path in edit.expected.keys().filter(|p| disk.contains_key(*p)) {
        names.insert(path.clone(), edit.title.clone());
    }
    if names != old {
        write(state, &names)?;
    }
    Ok(())
}

/// Called under the workspace lock after the snapshot has reached disk.
/// A newer pending title survives an older desktop save.
pub fn acknowledge(state: &Path, saved: impl Fn(&str) -> Option<Option<String>>) -> io::Result<()> {
    if !sidecar(state, ".project-names.json").try_exists()? {
        return Ok(());
    }
    let _lock = names_lock(state)?;
    let mut names = pending(state)?;
    let before = names.len();
    names.retain(|path, title| saved(path).is_some_and(|on_disk| on_disk != *title));
    if names.len() != before {
        write(state, &names)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_workspace_save_preserves_a_newer_pending_rename() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("desktop.json");
        let checkout = root.path().join("repo").to_str().unwrap().to_owned();
        let snapshot = |title: Option<&str>| {
            let titles: BTreeMap<_, _> = title.into_iter().map(|t| (&checkout, t)).collect();
            fs::write(
                &state,
                serde_json::to_vec(&serde_json::json!({"project_list":{
                    "entries":[{"path":checkout,"name":"repo"}],"titles":titles
                }}))
                .unwrap(),
            )
            .unwrap();
        };
        snapshot(None);
        let edit = |expected: Option<&str>, title: &str| Rename {
            expected: [(checkout.clone(), expected.map(str::to_owned))].into(),
            title: Some(title.to_owned()),
        };
        rename(&state, &edit(None, "First"), false).unwrap();
        rename(&state, &edit(Some("First"), "Second"), false).unwrap();
        snapshot(Some("First"));
        acknowledge(&state, |_| Some(Some("First".into()))).unwrap();
        assert_eq!(
            pending(&state).unwrap()[&checkout].as_deref(),
            Some("Second")
        );
        snapshot(Some("Second"));
        acknowledge(&state, |_| Some(Some("Second".into()))).unwrap();
        assert!(pending(&state).unwrap().is_empty());
    }
}
