//! Owner-selected storage for new worktrees. Old locations remain available for restore.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    marker: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Registry {
    active: Option<PathBuf>,
    roots: Vec<Entry>,
}
const MARKER: &str = ".bruhs-location";

fn default_root() -> io::Result<PathBuf> {
    directories::ProjectDirs::from("dev", "bruhs", "bruhs-ai")
        .map(|d| d.data_dir().join("worktrees"))
        .ok_or_else(|| io::Error::other("No data folder for worktrees."))
}
fn registry_file(default: &Path) -> PathBuf {
    default.with_file_name("worktree-locations.json")
}
fn read(default: &Path) -> io::Result<Registry> {
    match fs::read(registry_file(default)) {
        Ok(data) => Ok(serde_json::from_slice(&data)?),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(e) => Err(e),
    }
}
fn verify(e: &Entry) -> bool {
    let marker=e.path.join(MARKER);
    e.path.canonicalize().is_ok_and(|canonical|canonical==e.path)
        && fs::symlink_metadata(&marker).is_ok_and(|meta|meta.is_file() && meta.len()==e.marker.len() as u64)
        && fs::read_to_string(marker).is_ok_and(|text|text==e.marker)
}
/// The owner-selected parent, including when a removable volume is disconnected.
pub fn selected_parent() -> io::Result<Option<PathBuf>> {
    Ok(read(&default_root()?)?.active.and_then(|p|p.parent().map(Path::to_owned)))
}
pub fn active() -> io::Result<PathBuf> {
    active_in(&default_root()?)
}
fn active_in(default: &Path) -> io::Result<PathBuf> {
    let reg = read(default)?;
    match reg.active {
        None => Ok(default.to_owned()),
        Some(p) if reg.roots.iter().any(|e| e.path == p && verify(e)) => Ok(p),
        Some(_) => Err(io::Error::other(
            "The selected worktree location is unavailable. Reconnect it or choose another folder in Settings.",
        )),
    }
}
pub fn all() -> io::Result<Vec<PathBuf>> {
    all_in(&default_root()?)
}
fn all_in(default: &Path) -> io::Result<Vec<PathBuf>> {
    let mut roots = vec![default.to_owned()];
    roots.extend(
        read(default)?
            .roots
            .iter()
            .filter(|e| verify(e))
            .map(|e| e.path.clone()),
    );
    Ok(roots)
}
pub fn for_path(path: &Path) -> io::Result<Option<PathBuf>> {
    Ok(all()?.into_iter().find(|root| super::inside(root, path)))
}
pub fn choose(parent: Option<&Path>) -> io::Result<PathBuf> {
    choose_in(&default_root()?, parent)
}
fn choose_in(default: &Path, parent: Option<&Path>) -> io::Result<PathBuf> {
    let config = registry_file(default);
    let data = config
        .parent()
        .ok_or_else(|| io::Error::other("Worktree settings folder unavailable"))?;
    fs::create_dir_all(data)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.with_extension("lock"))?;
    lock.lock()?;
    let mut reg = read(default)?;
    let selected = if let Some(parent) = parent {
        let parent = parent.canonicalize()?;
        if !parent.is_dir() {
            return Err(io::Error::other("Select a folder."));
        }
        let path = parent.join("bruhs-worktrees");
        if let Some(entry) = reg.roots.iter().find(|e| e.path == path) {
            if !verify(entry) {
                return Err(io::Error::other(
                    "The worktree folder has moved or belongs to a different installation.",
                ));
            }
        } else {
            let overlaps = |root: &Path| path.starts_with(root) || root.starts_with(&path);
            if overlaps(&super::resolve(default)) || reg.roots.iter().any(|e| overlaps(&e.path)) {
                return Err(io::Error::other(
                    "Choose a folder outside existing worktree storage.",
                ));
            }
            let marker = uuid::Uuid::new_v4().to_string();
            fs::create_dir(&path).map_err(|e| {
                io::Error::new(e.kind(), format!("Cannot create {}: {e}", path.display()))
            })?;
            fs::write(path.join(MARKER), &marker)?;
            reg.roots.push(Entry {
                path: path.clone(),
                marker,
            });
        }
        Some(path)
    } else {
        None
    };
    reg.active = selected.clone();
    let mut temp = tempfile::NamedTempFile::new_in(data)?;
    serde_json::to_writer(&mut temp, &reg)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(config)?;
    Ok(selected.unwrap_or_else(|| default.to_owned()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_locations_preserves_old_checkouts_and_rejects_unowned_folders() {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("data/worktrees");
        let target = fixture.path().join("custom");
        fs::create_dir(&target).unwrap();
        let chosen = choose_in(&original, Some(&target)).unwrap();
        assert_eq!(active_in(&original).unwrap(), chosen);
        assert!(choose_in(&original, Some(&chosen)).is_err());
        choose_in(&original, None).unwrap();
        assert_eq!(active_in(&original).unwrap(), original);
        assert!(all_in(&original).unwrap().contains(&chosen));
        choose_in(&original, Some(&target)).unwrap();
        fs::write(chosen.join(MARKER), "foreign").unwrap();
        assert!(active_in(&original).is_err());
        assert!(!all_in(&original).unwrap().contains(&chosen));
        assert!(choose_in(&original, Some(&target)).is_err());
    }
}
