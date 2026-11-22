//! Local agents moving in: OpenClaw workspaces and Hermes Agent homes
//! found in their default places or in a picked folder, each read as a
//! persona that can become a bruh (D10, D11).
//!
//! Only the persona files are opened, by name (`files`): never a config
//! file, a secret or anything through a link. Nothing here writes, and
//! nothing logs a name, a path or a persona's text.
//!
//! **Places.** Each place D10 names by path (the picked folder, the three
//! override variables, each fixed default and the home itself) is resolved
//! once, links and all, as the owner laid it out. A link whose target is
//! missing counts as missing. The never-opened names are then checked on
//! the resolved path, below the resolved home, so a link can't lead into
//! `credentials/`. The resolved folder opens with `O_NOFOLLOW`, and no
//! link below it is followed: places found by listing (`workspace-*`,
//! `~/.openclaw-*`, `profiles/*`) must be real folders, and every file
//! opens by name through folder handles.

use super::ImportError;
use bruhs_models::imports::{
    AgentScan, PersonaCandidate, FOLDER_MISSING, FOLDER_PRIVATE, IMPORT_INVALID, MAX_PERSONAS,
};
use bruhs_models::logic::portable::ImportSource;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

mod files;
mod hermes;
mod openclaw;
#[cfg(test)]
mod tests;

/// Names a listing keeps from one folder, the first by name.
const MAX_LISTED: usize = 10_000;

/// Where the local agents keep their folders.
#[derive(Debug, Clone)]
pub struct AgentRoots {
    /// The home the default places sit in. Empty or relative: none are
    /// searched.
    pub home: PathBuf,
    /// `OPENCLAW_STATE_DIR`, else `~/.openclaw`.
    pub openclaw_state: Option<PathBuf>,
    /// `OPENCLAW_WORKSPACE_DIR`, looked in first.
    pub openclaw_workspace: Option<PathBuf>,
    /// `OPENCLAW_PROFILE`: its `~/.openclaw-<profile>/workspace`.
    pub openclaw_profile: Option<String>,
    /// `HERMES_HOME`, else `~/.hermes`.
    pub hermes_home: Option<PathBuf>,
}

impl AgentRoots {
    /// This account's home and the agents' own override variables. An
    /// empty variable is unset, and a relative one is ignored, so nothing
    /// resolves against the server's working folder.
    pub fn from_env() -> Self {
        let home = directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .filter(|home| home.is_absolute())
            .unwrap_or_default();
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let folder = |name: &str| {
            var(name)
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        Self {
            home,
            openclaw_state: folder("OPENCLAW_STATE_DIR"),
            openclaw_workspace: folder("OPENCLAW_WORKSPACE_DIR"),
            openclaw_profile: var("OPENCLAW_PROFILE").and_then(|value| value.into_string().ok()),
            hermes_home: folder("HERMES_HOME"),
        }
    }

    /// The default places under `home`, with no overrides (tests).
    pub fn under(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            ..Self::none()
        }
    }

    /// No places at all: a scan finds only a picked absolute folder.
    pub fn none() -> Self {
        Self {
            home: PathBuf::new(),
            openclaw_state: None,
            openclaw_workspace: None,
            openclaw_profile: None,
            hermes_home: None,
        }
    }

    /// The home, when there is one to search.
    fn named_home(&self) -> Option<&Path> {
        Some(self.home.as_path()).filter(|home| home.is_absolute())
    }

    /// A typed or picked path with `~` and `~/…` read against the home.
    /// `None` for one of those with no home to read it against.
    pub fn expand(&self, path: &Path) -> Option<PathBuf> {
        let text = path.to_string_lossy();
        match (text.as_ref(), text.strip_prefix("~/"), self.named_home()) {
            ("~", _, Some(home)) => Some(home.to_path_buf()),
            (_, Some(rest), Some(home)) => Some(home.join(rest)),
            ("~", _, None) | (_, Some(_), None) => None,
            _ => Some(path.to_path_buf()),
        }
    }
}

/// The personas `source` has: from `picked` when given, else from its
/// default places in D10's order. Each folder is answered once, and at
/// most [`MAX_PERSONAS`]. `looked_in` names each place as the owner would,
/// `~`-relative.
///
/// # Errors
///
/// [`ImportError::Refused`] with:
/// - `FOLDER_MISSING` when `picked` isn't an absolute, existing folder;
/// - `FOLDER_PRIVATE` when `picked`, or an override variable, leads to a
///   folder named for secrets;
/// - `IMPORT_INVALID` for a source that isn't a local agent.
pub fn scan(
    roots: &AgentRoots,
    source: ImportSource,
    picked: Option<&Path>,
) -> Result<AgentScan, ImportError> {
    let mut scan = Scan::new(roots);
    match (source, picked) {
        (ImportSource::OpenClaw, Some(picked)) => {
            let (named, folder) = scan.picked(picked)?;
            scan.looked(&named);
            if !scan.openclaw(&folder) {
                scan.openclaw_state(&folder);
            }
        }
        (ImportSource::OpenClaw, None) => scan.openclaw_defaults()?,
        (ImportSource::Hermes, Some(picked)) => {
            let (named, folder) = scan.picked(picked)?;
            scan.looked(&named);
            scan.hermes(&folder);
            // A home's own `profiles` folder, picked: its profiles.
            if folder.file_name().is_some_and(|name| name == "profiles") {
                if let Some(home) = folder.parent() {
                    scan.profiles(home);
                }
            }
        }
        (ImportSource::Hermes, None) => {
            let named = match (&roots.hermes_home, roots.named_home()) {
                (Some(home), _) => Some((home.clone(), true)),
                (None, Some(home)) => Some((home.join(".hermes"), false)),
                (None, None) => None,
            };
            if let Some((named, chosen)) = named {
                scan.looked(&named);
                scan.looked_glob(&named.join("profiles"), "/*");
                if let Some(home) = scan.place(&named, chosen)? {
                    scan.hermes(&home);
                }
            }
        }
        _ => return Err(ImportError::Refused(IMPORT_INVALID)),
    }
    Ok(AgentScan {
        personas: scan.personas,
        looked_in: scan.looked_in,
    })
}

/// Whether a path the owner picked leads somewhere never opened: a part
/// of `named` below the home, or of `resolved` below the resolved home
/// (all of either when it isn't under its home), is a name a scan never
/// opens, such as `credentials` or `secrets`. The desktop's export picker
/// checks its paths with this, as a scan checks its places (F33).
pub(crate) fn picked_private(roots: &AgentRoots, named: &Path, resolved: &Path) -> bool {
    let home = roots
        .named_home()
        .and_then(|home| std::fs::canonicalize(home).ok());
    private_below(roots.named_home(), named) || private_below(home.as_deref(), resolved)
}

/// Whether a part of `path` below `home`, or of all of it when it isn't
/// under `home`, is never opened.
fn private_below(home: Option<&Path>, path: &Path) -> bool {
    let below = home
        .and_then(|home| path.strip_prefix(home).ok())
        .unwrap_or(path);
    below.components().any(|part| match part {
        Component::Normal(name) => files::never_open(&name.to_string_lossy()),
        _ => false,
    })
}

/// One scan's state: the resolved home, and what it has found.
struct Scan<'a> {
    roots: &'a AgentRoots,
    /// The home resolved, which the never-opened names are checked below.
    home: Option<PathBuf>,
    personas: Vec<PersonaCandidate>,
    looked_in: Vec<String>,
}

/// What a named place resolved to.
enum Place {
    /// Missing, a link to nothing, or not a folder.
    Missing,
    /// It leads to a folder named for secrets.
    Private,
    Folder(PathBuf),
}

impl<'a> Scan<'a> {
    fn new(roots: &'a AgentRoots) -> Self {
        Self {
            roots,
            home: roots
                .named_home()
                .and_then(|home| std::fs::canonicalize(home).ok()),
            personas: Vec::new(),
            looked_in: Vec::new(),
        }
    }

    /// Whether the scan holds [`MAX_PERSONAS`]: nothing more is read.
    fn full(&self) -> bool {
        self.personas.len() >= MAX_PERSONAS
    }

    /// `named` resolved to a real folder.
    fn resolve(&self, named: &Path) -> Place {
        if !named.is_absolute() {
            return Place::Missing;
        }
        match std::fs::canonicalize(named) {
            Ok(folder) if std::fs::metadata(&folder).is_ok_and(|meta| meta.is_dir()) => {
                if self.private(&folder) {
                    Place::Private
                } else {
                    Place::Folder(folder)
                }
            }
            _ => Place::Missing,
        }
    }

    /// A named place: its resolved folder, or `None` when it is missing or
    /// a default that leads somewhere private. One the owner `chose` (an
    /// override) that leads somewhere private is refused out loud.
    fn place(&self, named: &Path, chose: bool) -> Result<Option<PathBuf>, ImportError> {
        match self.resolve(named) {
            Place::Folder(folder) => Ok(Some(folder)),
            Place::Private if chose => Err(ImportError::Refused(FOLDER_PRIVATE)),
            Place::Private | Place::Missing => Ok(None),
        }
    }

    /// Whether a part of `folder` below the resolved home, or of all of it
    /// when it isn't under the home, is never opened (`credentials`, say).
    fn private(&self, folder: &Path) -> bool {
        private_below(self.home.as_deref(), folder)
    }

    /// A typed or picked folder: `~` and `~/…` read against the home. It
    /// answers the named path and the resolved folder.
    fn picked(&self, path: &Path) -> Result<(PathBuf, PathBuf), ImportError> {
        let named = self
            .roots
            .expand(path)
            .ok_or(ImportError::Refused(FOLDER_MISSING))?;
        match self.resolve(&named) {
            Place::Folder(folder) => Ok((named, folder)),
            Place::Private => Err(ImportError::Refused(FOLDER_PRIVATE)),
            Place::Missing => Err(ImportError::Refused(FOLDER_MISSING)),
        }
    }

    /// Adds `path` to `looked_in`, as the owner reads it.
    fn looked(&mut self, path: &Path) {
        let shown = self.shown(path);
        self.looked_in.push(shown);
    }

    /// Adds `path` and `glob` to `looked_in`: `~/.openclaw/workspace-*`.
    fn looked_glob(&mut self, path: &Path, glob: &str) {
        let shown = format!("{}{glob}", self.shown(path));
        self.looked_in.push(shown);
    }

    /// `path` as `~/…` when it is under the home, as named or resolved.
    fn shown(&self, path: &Path) -> String {
        let homes = [self.roots.named_home(), self.home.as_deref()];
        for home in homes.into_iter().flatten() {
            if let Ok(rest) = path.strip_prefix(home) {
                return if rest.as_os_str().is_empty() {
                    "~".to_string()
                } else {
                    format!("~/{}", rest.display())
                };
            }
        }
        path.display().to_string()
    }

    /// Adds `persona` unless its folder is already in.
    fn add(&mut self, persona: Option<PersonaCandidate>) -> bool {
        let Some(persona) = persona else {
            return false;
        };
        if !self.full() && !self.personas.iter().any(|known| known.id == persona.id) {
            self.personas.push(persona);
        }
        true
    }

    /// The resolved `folder` as an OpenClaw workspace; whether it is one.
    fn openclaw(&mut self, folder: &Path) -> bool {
        !self.full() && self.add(openclaw::persona(folder))
    }

    /// The named place `named` as an OpenClaw workspace.
    fn openclaw_named(&mut self, named: &Path, chose: bool) -> Result<(), ImportError> {
        self.looked(named);
        if let Some(folder) = self.place(named, chose)? {
            self.openclaw(&folder);
        }
        Ok(())
    }

    /// The resolved `state`'s `workspace`, then each real `workspace-*`.
    fn openclaw_state(&mut self, state: &Path) {
        if let Place::Folder(workspace) = self.resolve(&state.join("workspace")) {
            self.openclaw(&workspace);
        }
        for name in real_folders(state, |name| name.starts_with("workspace-")) {
            if self.full() {
                return;
            }
            self.openclaw(&state.join(name));
        }
    }

    /// D10's OpenClaw places, in order.
    fn openclaw_defaults(&mut self) -> Result<(), ImportError> {
        let roots = self.roots;
        if let Some(workspace) = &roots.openclaw_workspace {
            self.openclaw_named(workspace, true)?;
        }
        let home = roots.named_home().map(Path::to_path_buf);
        let state = match (&roots.openclaw_state, &home) {
            (Some(state), _) => Some((state.clone(), true)),
            (None, Some(home)) => Some((home.join(".openclaw"), false)),
            (None, None) => None,
        };
        if let Some((state, chose)) = state {
            self.openclaw_named(&state.join("workspace"), chose)?;
            self.looked_glob(&state.join("workspace"), "-*");
            if let Some(state) = self.place(&state, chose)? {
                for name in real_folders(&state, |name| name.starts_with("workspace-")) {
                    if self.full() {
                        break;
                    }
                    self.openclaw(&state.join(name));
                }
            }
        }
        let Some(home) = home else {
            return Ok(());
        };
        let profile = roots
            .openclaw_profile
            .as_deref()
            .filter(|profile| !profile.is_empty() && !profile.contains(['/', '\\']));
        if let Some(profile) = profile {
            let named = home.join(format!(".openclaw-{profile}")).join("workspace");
            self.openclaw_named(&named, false)?;
        }
        self.looked_glob(&home.join(".openclaw"), "-*/workspace");
        if let Some(resolved) = self.home.clone() {
            for name in real_folders(&resolved, |name| name.starts_with(".openclaw-")) {
                if self.full() {
                    break;
                }
                let named = resolved.join(name).join("workspace");
                if let Place::Folder(workspace) = self.resolve(&named) {
                    self.openclaw(&workspace);
                }
            }
        }
        for legacy in [
            ".clawdbot/workspace",
            ".moltbot/workspace",
            "clawd",
            "openclaw",
        ] {
            self.openclaw_named(&home.join(legacy), false)?;
        }
        Ok(())
    }

    /// The resolved Hermes home `home`, then each of its profiles.
    fn hermes(&mut self, home: &Path) {
        if !self.full() {
            self.add(hermes::persona(home, None));
        }
        self.profiles(home);
    }

    /// Each real folder in `home`'s `profiles`, as a Hermes profile.
    fn profiles(&mut self, home: &Path) {
        let profiles = files::list_names(&home.join("profiles"), MAX_LISTED);
        for name in profiles
            .into_iter()
            .filter(|(_, folder)| *folder)
            .map(|(name, _)| name)
        {
            if self.full() {
                return;
            }
            self.add(hermes::persona(home, Some(&name)));
        }
    }
}

/// The real folders directly in `dir` whose names `keep` takes, sorted.
fn real_folders(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    files::select(dir, MAX_LISTED, keep, files::Pick::First)
        .0
        .into_iter()
        .filter(|(_, kind)| *kind == files::Kind::Folder)
        .map(|(name, _)| name)
        .collect()
}

/// A persona's id: `p-` and 16 hex characters of the SHA-256 of its
/// canonical folder.
fn persona_id(folder: &Path) -> String {
    let canonical = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    let mut id = String::with_capacity(18);
    id.push_str("p-");
    for byte in &digest[..8] {
        let _ = write!(id, "{byte:02x}");
    }
    id
}
