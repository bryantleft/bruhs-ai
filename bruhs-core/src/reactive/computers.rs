//! Keeps the local tools host's bruh computers in step with the store, and
//! removes the `<home>/bruhs/<id>` tree of every deleted bruh once its
//! browser has closed.
//!
//! Removal is deliberately narrow: only a real directory (never a link)
//! directly under a real `<home>/bruhs`, whose name is a valid id that no
//! agent has, and whose `.bruhs-owned` marker names it. Everything else
//! stays, so a custom work folder or anything a person put there is safe.

use super::{store, ReactiveError, ReactiveRuntime};
use crate::local_tools::{BruhComputer, LocalToolsHost};
use bruhs_models::reactive::ReactiveAgent;
use rusqlite::Connection;
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc, OnceLock, Weak},
};
use tokio::sync::{watch, Mutex};

/// The file in each `bruhs/<id>` tree Bruhs made, holding exactly `<id>`.
pub(super) const OWNED_MARKER: &str = ".bruhs-owned";
/// The folder under the runtime's home that holds every bruh's tree.
pub(super) const TREES: &str = "bruhs";
/// A marker larger than this is not one Bruhs wrote.
const MARKER_LIMIT: u64 = 1024;

/// A bruh tree's browser profile and its cache, which no bruh run may read.
pub(super) const BROWSER_FOLDERS: [&str; 2] = ["browser", "browser-cache"];

/// The folders every bruh run is kept out of: each bruh's browser profile
/// and cache under `home`, and by name the bruh `id`'s own. The profile
/// holds the owner's logins from Open in a window, and the folder's 0700
/// mode doesn't keep out the bruh's own process.
pub(super) fn hidden_folders(home: &Path, id: &str) -> Vec<PathBuf> {
    let own = bruh_tree(home, id).ok();
    let every = home.join(TREES).join("*");
    own.into_iter()
        .chain([every])
        .flat_map(|tree| BROWSER_FOLDERS.map(|folder| tree.join(folder)))
        .collect()
}

/// `<home>/bruhs/<id>`, for an id that can't leave that folder.
pub(super) fn bruh_tree(home: &Path, id: &str) -> Result<PathBuf, ReactiveError> {
    bruhs_models::ids::validate_id(id).map_err(|_| {
        ReactiveError::Invalid(
            "Use a nonempty ID without path separators or control characters.".into(),
        )
    })?;
    Ok(home.join(TREES).join(id))
}

/// Create `directory` private to the owner when missing. An existing one
/// must be a real directory, not a link.
fn private_directory(directory: &Path) -> Result<(), ReactiveError> {
    let refused = || ReactiveError::Storage("Could not create the bruh's folder.".into());
    match std::fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refused()),
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            match std::fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.is_dir() => Ok(()),
                _ => Err(refused()),
            }
        }
        Err(_) => Err(refused()),
    }
}

/// Make `<home>/bruhs/<id>` if needed and write its marker, so a later
/// sweep may remove it once the bruh is gone. `<home>/bruhs` may be a link
/// (the sweep then skips everything); the tree itself may not.
pub(super) fn mark_tree(home: &Path, id: &str) -> Result<(), ReactiveError> {
    let tree = bruh_tree(home, id)?;
    let root = home.join(TREES);
    if !root.is_dir() {
        private_directory(&root)?;
    }
    private_directory(&tree)?;
    let marker = tree.join(OWNED_MARKER);
    let refused = || ReactiveError::Storage("Could not mark the bruh's folder.".into());
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.is_file() => {
            if metadata.len() <= MARKER_LIMIT
                && std::fs::read(&marker).is_ok_and(|text| text == id.as_bytes())
            {
                return Ok(());
            }
            // Replaced, never rewritten in place: a hard-linked marker
            // would carry the write to the file it shares.
            std::fs::remove_file(&marker).map_err(|_| refused())?;
        }
        Ok(_) => return Err(refused()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(refused()),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&marker).map_err(|_| refused())?;
    file.write_all(id.as_bytes()).map_err(|_| refused())
}

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Sweep {
    removed: usize,
    /// Owned trees that couldn't be removed; a later pass tries again.
    failed: usize,
}

/// Remove every owned tree under `<home>/bruhs` whose name isn't in `live`.
/// Returns how many trees went. The sync pass calls [`sweep`], which also
/// counts the trees that stayed.
#[cfg(test)]
pub(super) fn sweep_trees(home: &Path, live: &HashSet<String>) -> usize {
    sweep(home, live).removed
}

fn sweep(home: &Path, live: &HashSet<String>) -> Sweep {
    let mut outcome = Sweep::default();
    let root = home.join(TREES);
    // A missing, linked or odd `bruhs` holds nothing to remove.
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() => {}
        _ => return outcome,
    }
    // A link anywhere above it would put the trees somewhere else.
    if std::fs::canonicalize(&root).ok().as_deref() != Some(root.as_path()) {
        tracing::warn!("The bruhs folder moved behind a link; nothing was removed");
        return outcome;
    }
    let Ok(entries) = std::fs::read_dir(&root) else {
        return outcome;
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if bruhs_models::ids::validate_id(&name).is_err() || is_live(&root, &name, live) {
            continue;
        }
        let tree = root.join(&name);
        if !owned(&tree, &name) {
            continue;
        }
        match std::fs::remove_dir_all(&tree) {
            Ok(()) => outcome.removed += 1,
            Err(error) => {
                tracing::warn!(%error, "A deleted bruh's folder could not be removed");
                outcome.failed += 1;
            }
        }
    }
    outcome
}

/// Whether `name` may be a live bruh's tree: its id, or, on a disk that
/// folds case or Unicode forms, the same directory as a live id's.
fn is_live(root: &Path, name: &str, live: &HashSet<String>) -> bool {
    if live.contains(name) {
        return true;
    }
    let folded = name.to_lowercase();
    if live.iter().any(|id| id.to_lowercase() == folded) {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let Ok(this) = std::fs::symlink_metadata(root.join(name)) else {
            return true;
        };
        live.iter()
            .filter(|id| bruhs_models::ids::validate_id(id).is_ok())
            .filter_map(|id| std::fs::symlink_metadata(root.join(id)).ok())
            .any(|other| other.dev() == this.dev() && other.ino() == this.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        false
    }
}

/// A real directory, not behind a link, whose marker is a plain file naming
/// it exactly.
fn owned(tree: &Path, name: &str) -> bool {
    match std::fs::symlink_metadata(tree) {
        Ok(metadata) if metadata.is_dir() => {}
        _ => return false,
    }
    if std::fs::canonicalize(tree).ok().as_deref() != Some(tree) {
        return false;
    }
    let marker = tree.join(OWNED_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MARKER_LIMIT => {}
        _ => return false,
    }
    std::fs::read(&marker).is_ok_and(|text| text == name.as_bytes())
}

fn agents(connection: &Connection) -> Result<Vec<ReactiveAgent>, ReactiveError> {
    store::records::<ReactiveAgent>(connection, "reactive_agents")
}

/// Mark the work folders Bruhs made for live agents before this marker
/// existed, so their trees go when the bruh does.
fn mark_work_folders(home: &Path, agents: &[ReactiveAgent]) {
    for agent in agents {
        let Ok(tree) = bruh_tree(home, &agent.id) else {
            continue;
        };
        let work = tree.join("work");
        let made_here = Path::new(&agent.project_path) == work;
        let real = std::fs::symlink_metadata(&tree).is_ok_and(|metadata| metadata.is_dir());
        if made_here && real {
            if let Err(error) = mark_tree(home, &agent.id) {
                tracing::warn!(%error, "A bruh's work folder could not be marked");
            }
        }
    }
}

/// What the sync loop remembers between passes.
#[derive(Default)]
pub(super) struct Pass {
    /// The agent ids the last finished sweep saw.
    swept: Option<HashSet<String>>,
    /// The last sweep left an owned tree behind.
    retry: bool,
}

/// One pass: hand the host the bruhs whose Computer is on, then, when the
/// set of agents changed (or on the first pass, or after a failure), remove
/// deleted bruhs' trees. Without a host it only sweeps.
pub(super) async fn sync_pass(
    runtime: &ReactiveRuntime,
    host: Option<&LocalToolsHost>,
    pass: &mut Pass,
) {
    let home = Arc::clone(&runtime.home);
    let marks = host.is_some();
    let read = runtime
        .call(move |connection| {
            let computers = store::crew::computers(connection, &home)?;
            let ids: HashSet<String> = agents(connection)?.into_iter().map(|a| a.id).collect();
            // Marked before the host can create a profile in it. A tree
            // that can't be marked gets no browser.
            let computers = computers
                .into_iter()
                .filter(|computer| {
                    !marks
                        || mark_tree(&home, &computer.agent_id)
                            .inspect_err(|error| {
                                tracing::warn!(%error, "A bruh's Computer folder could not be marked");
                            })
                            .is_ok()
                })
                .collect::<Vec<BruhComputer>>();
            Ok((computers, ids))
        })
        .await;
    let (computers, ids) = match read {
        Ok(read) => read,
        Err(error) => {
            tracing::warn!(%error, "Bruh computers could not be read");
            return;
        }
    };
    // Bruhs the host still has a slot for keep their trees this pass; a
    // deleted one's goes on the next, after `sync_bruhs` closed it.
    let synced: HashSet<String> = computers.iter().map(|c| c.agent_id.clone()).collect();
    if let Some(host) = host {
        host.sync_bruhs(computers).await;
    }
    if !pass.retry && pass.swept.as_ref() == Some(&ids) {
        return;
    }
    let home = Arc::clone(&runtime.home);
    // In one call, so no new bruh can take a name between the check and the
    // removal.
    let swept = runtime
        .call(move |connection| {
            let agents = agents(connection)?;
            mark_work_folders(&home, &agents);
            let keep: HashSet<String> = agents.into_iter().map(|a| a.id).chain(synced).collect();
            Ok(sweep(&home, &keep))
        })
        .await;
    match swept {
        Ok(sweep) => {
            pass.swept = Some(ids);
            pass.retry = sweep.failed > 0;
        }
        Err(error) => {
            tracing::warn!(%error, "Deleted bruhs' folders could not be checked");
            pass.retry = true;
        }
    }
}

/// The runtime without keeping it alive, so the sync loop ends with it.
struct WeakRuntime {
    connection: Weak<Mutex<Connection>>,
    lock: Weak<store::RuntimeLock>,
    home: Weak<PathBuf>,
    started: Weak<std::sync::atomic::AtomicBool>,
    changes: Weak<watch::Sender<u64>>,
    notifier: Weak<std::sync::RwLock<super::Notifier>>,
    computers: Weak<OnceLock<Arc<LocalToolsHost>>>,
    syncing: Weak<std::sync::atomic::AtomicBool>,
    allowances: Weak<super::approval::Allowances>,
    private_roots: Weak<Vec<PathBuf>>,
}

impl WeakRuntime {
    fn new(runtime: &ReactiveRuntime) -> Self {
        Self {
            connection: Arc::downgrade(&runtime.connection),
            lock: Arc::downgrade(&runtime._lock),
            home: Arc::downgrade(&runtime.home),
            started: Arc::downgrade(&runtime.started),
            changes: Arc::downgrade(&runtime.changes),
            notifier: Arc::downgrade(&runtime.notifier),
            computers: Arc::downgrade(&runtime.computers),
            syncing: Arc::downgrade(&runtime.syncing),
            allowances: Arc::downgrade(&runtime.allowances),
            private_roots: Arc::downgrade(&runtime.private_roots),
        }
    }

    fn upgrade(&self) -> Option<ReactiveRuntime> {
        Some(ReactiveRuntime {
            connection: self.connection.upgrade()?,
            _lock: self.lock.upgrade()?,
            home: self.home.upgrade()?,
            started: self.started.upgrade()?,
            changes: self.changes.upgrade()?,
            notifier: self.notifier.upgrade()?,
            computers: self.computers.upgrade()?,
            syncing: self.syncing.upgrade()?,
            allowances: self.allowances.upgrade()?,
            private_roots: self.private_roots.upgrade()?,
        })
    }
}

/// Start the runtime's one sync loop, unless it already runs: a pass now
/// and after every change. It ends when the runtime is dropped.
pub(super) fn spawn_sync(runtime: &ReactiveRuntime) {
    if runtime.syncing.swap(true, Ordering::AcqRel) {
        return;
    }
    /// Lets a later `start` run the loop again if this one panicked.
    struct Running(Weak<std::sync::atomic::AtomicBool>);
    impl Drop for Running {
        fn drop(&mut self) {
            if let Some(syncing) = self.0.upgrade() {
                syncing.store(false, Ordering::Release);
            }
        }
    }
    let running = Running(Arc::downgrade(&runtime.syncing));
    let mut changes = runtime.watch_changes();
    let runtime = WeakRuntime::new(runtime);
    tokio::spawn(async move {
        let _running = running;
        let mut pass = Pass::default();
        loop {
            changes.borrow_and_update();
            let Some(live) = runtime.upgrade() else {
                return;
            };
            let host = live.computers();
            sync_pass(&live, host.as_deref(), &mut pass).await;
            drop(live);
            if changes.changed().await.is_err() {
                return;
            }
        }
    });
}
