//! Local agents moving in: OpenClaw workspaces and Hermes Agent homes
//! found in their default places or in a picked folder, each read as a
//! persona that can become a bruh (D10, D11).
//!
//! Only the persona files are opened, by name (`files`): never a config
//! file, a secret or anything through a link. Nothing here writes, and
//! nothing logs a name, a path or a persona's text.

use super::ImportError;
use bruhs_models::imports::{
    AgentScan, PersonaCandidate, FOLDER_MISSING, IMPORT_INVALID, MAX_PERSONAS,
};
use bruhs_models::logic::portable::ImportSource;
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

mod files;
mod hermes;
mod openclaw;
#[cfg(test)]
mod tests;

/// Names a scan reads from one folder; past it the rest are left out.
const MAX_LISTED: usize = 10_000;

/// Where the local agents keep their folders.
#[derive(Debug, Clone)]
pub struct AgentRoots {
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
    /// This account's home and the agents' own override variables, an
    /// empty one meaning unset.
    pub fn from_env() -> Self {
        let home = directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_default();
        let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        Self {
            home,
            openclaw_state: var("OPENCLAW_STATE_DIR").map(PathBuf::from),
            openclaw_workspace: var("OPENCLAW_WORKSPACE_DIR").map(PathBuf::from),
            openclaw_profile: var("OPENCLAW_PROFILE").and_then(|value| value.into_string().ok()),
            hermes_home: var("HERMES_HOME").map(PathBuf::from),
        }
    }

    /// The default places under `home`, with no overrides (tests).
    pub fn under(home: &Path) -> Self {
        Self {
            home: home.to_path_buf(),
            openclaw_state: None,
            openclaw_workspace: None,
            openclaw_profile: None,
            hermes_home: None,
        }
    }

    /// `path` as the owner reads it: `~/…` under the home.
    fn shown(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if !self.home.as_os_str().is_empty() => {
                if rest.as_os_str().is_empty() {
                    "~".to_string()
                } else {
                    format!("~/{}", rest.display())
                }
            }
            _ => path.display().to_string(),
        }
    }

    /// A typed or picked folder: `~` and `~/…` read against the home. It
    /// must be absolute and a real folder, not a link.
    fn picked(&self, path: &Path) -> Result<PathBuf, ImportError> {
        let text = path.to_string_lossy();
        let path = if text == "~" {
            self.home.clone()
        } else if let Some(rest) = text.strip_prefix("~/") {
            self.home.join(rest)
        } else {
            path.to_path_buf()
        };
        let folder =
            path.is_absolute() && std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir());
        if folder {
            Ok(path)
        } else {
            Err(ImportError::Refused(FOLDER_MISSING))
        }
    }
}

/// The personas `source` has: from `picked` when given, else from its
/// default places in D10's order. Each folder is answered once, and at
/// most [`MAX_PERSONAS`]. `looked_in` names each place, `~`-relative.
///
/// # Errors
///
/// [`ImportError::Refused`] with `FOLDER_MISSING` when `picked` isn't an
/// absolute, existing folder, or with `IMPORT_INVALID` for a source that
/// isn't a local agent.
pub fn scan(
    roots: &AgentRoots,
    source: ImportSource,
    picked: Option<&Path>,
) -> Result<AgentScan, ImportError> {
    let mut found = Found {
        home: roots.home.clone(),
        ..Found::default()
    };
    match (source, picked) {
        (ImportSource::OpenClaw, Some(picked)) => {
            let picked = roots.picked(picked)?;
            found.looked_in.push(roots.shown(&picked));
            if !found.openclaw(&picked) {
                found.openclaw_state(&picked);
            }
        }
        (ImportSource::OpenClaw, None) => openclaw_defaults(roots, &mut found),
        (ImportSource::Hermes, picked) => {
            let home = match picked {
                Some(picked) => roots.picked(picked)?,
                None => roots
                    .hermes_home
                    .clone()
                    .unwrap_or_else(|| roots.home.join(".hermes")),
            };
            found.looked_in.push(roots.shown(&home));
            if picked.is_none() {
                found
                    .looked_in
                    .push(format!("{}/*", roots.shown(&home.join("profiles"))));
            }
            found.hermes(&home);
        }
        _ => return Err(ImportError::Refused(IMPORT_INVALID)),
    }
    Ok(AgentScan {
        personas: found.personas,
        looked_in: found.looked_in,
    })
}

/// D10's OpenClaw places, in order.
fn openclaw_defaults(roots: &AgentRoots, found: &mut Found) {
    let home = &roots.home;
    if let Some(workspace) = &roots.openclaw_workspace {
        found.looked_in.push(roots.shown(workspace));
        found.openclaw(workspace);
    }
    let state = roots
        .openclaw_state
        .clone()
        .unwrap_or_else(|| home.join(".openclaw"));
    found.looked_in.push(roots.shown(&state.join("workspace")));
    found
        .looked_in
        .push(format!("{}-*", roots.shown(&state.join("workspace"))));
    found.openclaw_state(&state);
    let profile = roots
        .openclaw_profile
        .as_deref()
        .filter(|profile| !profile.contains(['/', '\\']) && !files::never_open(profile));
    if let Some(profile) = profile {
        let workspace = home.join(format!(".openclaw-{profile}")).join("workspace");
        found.looked_in.push(roots.shown(&workspace));
        found.openclaw(&workspace);
    }
    found.looked_in.push(format!(
        "{}-*/workspace",
        roots.shown(&home.join(".openclaw"))
    ));
    for name in real_folders(home, |name| name.starts_with(".openclaw-")) {
        found.openclaw(&home.join(name).join("workspace"));
    }
    for legacy in [
        ".clawdbot/workspace",
        ".moltbot/workspace",
        "clawd",
        "openclaw",
    ] {
        let workspace = home.join(legacy);
        found.looked_in.push(roots.shown(&workspace));
        found.openclaw(&workspace);
    }
}

/// The real folders directly in `dir` whose names `keep` takes, sorted.
fn real_folders(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    files::list_names(dir, MAX_LISTED)
        .into_iter()
        .filter(|(name, folder)| *folder && keep(name))
        .map(|(name, _)| name)
        .collect()
}

/// The personas a scan has found so far.
#[derive(Default)]
struct Found {
    /// Never-opened names are checked below it.
    home: PathBuf,
    personas: Vec<PersonaCandidate>,
    looked_in: Vec<String>,
}

impl Found {
    /// Adds `persona` unless its folder is already in, or the scan is full.
    fn add(&mut self, persona: Option<PersonaCandidate>) -> bool {
        let Some(persona) = persona else {
            return false;
        };
        if self.personas.len() < MAX_PERSONAS
            && !self.personas.iter().any(|known| known.id == persona.id)
        {
            self.personas.push(persona);
        }
        true
    }

    /// `dir` as an OpenClaw workspace; whether it is one.
    fn openclaw(&mut self, dir: &Path) -> bool {
        !self.never_opened(dir) && self.add(openclaw::persona(dir))
    }

    /// `state`'s `workspace`, then each `workspace-*`.
    fn openclaw_state(&mut self, state: &Path) {
        self.openclaw(&state.join("workspace"));
        for name in real_folders(state, |name| name.starts_with("workspace-")) {
            self.openclaw(&state.join(name));
        }
    }

    /// The Hermes home `home`, then each of its profiles.
    fn hermes(&mut self, home: &Path) {
        if self.never_opened(home) {
            return;
        }
        self.add(hermes::persona(home, None));
        for name in real_folders(&home.join("profiles"), |_| true) {
            self.add(hermes::persona(home, Some(&name)));
        }
    }

    /// Whether a folder under `dir`'s path below the home is named like a
    /// secret (a picked `credentials` folder, say), so nothing in it is
    /// opened.
    fn never_opened(&self, dir: &Path) -> bool {
        dir.strip_prefix(&self.home)
            .unwrap_or(dir)
            .components()
            .any(|part| match part {
                std::path::Component::Normal(name) => files::never_open(&name.to_string_lossy()),
                _ => false,
            })
    }
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
