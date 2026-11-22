//! The desktop and paired environments share one revision-checked shortcut file.
use bruhs_models::logic::keybindings::{rules_from_saved, Keybinding, KeybindingEdit};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};

pub fn validate_rules(rules: &[Keybinding]) -> io::Result<()> {
    if rules.len() > 256
        || rules.iter().any(|r| {
            r.command.trim().is_empty()
                || r.command.len() > 256
                || r.key.len() > 256
                || r.when.as_ref().is_some_and(|s| s.len() > 1024)
        })
    {
        return Err(io::Error::other(
            "Use up to 256 shortcut rules with a command name.",
        ));
    }
    Ok(())
}
pub struct Contents {
    pub revision: [u8; 32],
    pub rules: Vec<Keybinding>,
}
fn read(path: &Path, known: Option<[u8; 32]>) -> io::Result<ResultState> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(ResultState::NoFile),
        Err(e) => return Err(e),
    };
    if !meta.is_file() || meta.len() > 128 * 1024 {
        return Err(io::Error::other(
            "Use a regular keybindings file smaller than 128 KB.",
        ));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    fs::File::open(path)?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 128 * 1024 {
        return Err(io::Error::other("The keybindings file exceeds 128 KB."));
    }
    let revision = Sha256::digest(&bytes).into();
    if known == Some(revision) {
        return Ok(ResultState::Unchanged);
    }
    let rules: Vec<Keybinding> = serde_json::from_slice(&bytes).map_err(|e| {
        io::Error::other(format!(
            "Check keybindings.json at line {}, column {}.",
            e.line(),
            e.column()
        ))
    })?;
    validate_rules(&rules)?;
    Ok(ResultState::Data(Contents {
        revision,
        rules: rules_from_saved(rules, 1),
    }))
}
pub enum ResultState {
    NoFile,
    Unchanged,
    Data(Contents),
    Conflict,
}
pub fn sync(
    path: &Path,
    revision: Option<[u8; 32]>,
    save: Option<Vec<Keybinding>>,
    create: bool,
) -> io::Result<ResultState> {
    if !create && save.is_none() {
        return read(path, revision);
    }
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("The settings folder is unavailable."))?;
    fs::create_dir_all(parent)?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("lock"))?;
    lock.lock()?;
    let disk = read(path, None)?;
    let current = match &disk {
        ResultState::Data(contents) => Some(contents.revision),
        _ => None,
    };
    if create && current.is_some() && revision.is_none() {
        return Ok(disk);
    }
    if current != revision {
        return Ok(ResultState::Conflict);
    }
    let Some(rules) = save else {
        return Ok(disk);
    };
    validate_rules(&rules)?;
    let mut bytes = serde_json::to_vec_pretty(&rules)?;
    bytes.push(b'\n');
    if bytes.len() > 128 * 1024 {
        return Err(io::Error::other("The keybindings file exceeds 128 KB."));
    }
    let mut out = tempfile::NamedTempFile::new_in(parent)?;
    out.write_all(&bytes)?;
    out.as_file().sync_all()?;
    out.persist(path)?;
    Ok(ResultState::Data(Contents {
        revision: Sha256::digest(bytes).into(),
        rules,
    }))
}

/// A host can receive its first edit before its desktop has created the JSON file.
fn current(state: &Path) -> io::Result<(Option<[u8; 32]>, Vec<Keybinding>)> {
    if let ResultState::Data(file) = read(&state.with_file_name("keybindings.json"), None)? {
        return Ok((Some(file.revision), file.rules));
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Preferences {
        keybindings: Vec<Keybinding>,
        keybinding_schema: u8,
    }
    let prefs: Preferences = match fs::File::open(state) {
        Ok(f) => serde_json::from_reader(io::BufReader::new(f))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Preferences::default(),
        Err(e) => return Err(e),
    };
    Ok((
        None,
        rules_from_saved(prefs.keybindings, prefs.keybinding_schema),
    ))
}

pub fn environment_rules(
    state: &Path,
    edit: Option<&KeybindingEdit>,
) -> io::Result<Vec<Keybinding>> {
    let (revision, mut rules) = current(state)?;
    if let Some(edit) = edit {
        edit.apply(&mut rules)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        match sync(
            &state.with_file_name("keybindings.json"),
            revision,
            Some(rules),
            false,
        )? {
            ResultState::Data(file) => return Ok(file.rules),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Shortcuts changed on the host. Retry the edit.",
                ))
            }
        }
    }
    Ok(rules)
}

/// Open only this host’s keybindings file in its configured desktop editor.
pub fn open_environment_file(state: &Path) -> io::Result<()> {
    let path = state.with_file_name("keybindings.json");
    if !path.try_exists()? {
        let (revision, rules) = current(state)?;
        if revision.is_none() {
            sync(&path, None, Some(rules), true)?;
        }
    } else if !path.is_file() {
        return Err(io::Error::other(
            "The keybindings path is not a regular file.",
        ));
    }
    #[derive(Deserialize, Default)]
    struct Editor {
        editor_id: Option<String>,
    }
    let prefs: Editor = match fs::File::open(state) {
        Ok(f) => serde_json::from_reader(io::BufReader::new(f))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Editor::default(),
        Err(e) => return Err(e),
    };
    let editor = prefs
        .editor_id
        .as_deref()
        .unwrap_or(bruhs_models::logic::editor::DEFAULT_EDITOR_ID);
    crate::editor::open_in_editor(editor, &path, None).map_err(io::Error::other)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{read, sync, Keybinding, ResultState};
    #[test]
    fn an_empty_file_keeps_all_shipped_keys_and_removal_survives_reload() {
        use bruhs_models::logic::keybindings::{default_keybindings, KeybindingEdit};
        let folder = tempfile::tempdir().unwrap();
        let state = folder.path().join("desktop-state.json");
        std::fs::write(folder.path().join("keybindings.json"), "[]").unwrap();
        let rules = super::environment_rules(&state, None).unwrap();
        assert_eq!(rules, default_keybindings());
        let first = rules
            .iter()
            .find(|r| r.command == "branch.menu")
            .unwrap()
            .clone();
        let second = Keybinding {
            key: "alt+8".into(),
            ..first.clone()
        };
        let add = KeybindingEdit::Upsert {
            rule: second.clone(),
            replace: None,
        };
        super::environment_rules(&state, Some(&add)).unwrap();
        for rule in [first, second] {
            let remove = KeybindingEdit::Remove { rule };
            for _ in 0..2 {
                super::environment_rules(&state, Some(&remove)).unwrap();
            }
        }
        let restored = super::environment_rules(&state, None).unwrap();
        let branches = restored
            .iter()
            .filter(|r| r.command == "branch.menu")
            .collect::<Vec<_>>();
        assert_eq!(branches.len(), 1);
        assert!(branches[0].key.is_empty());
    }

    #[test]
    fn external_edits_survive_races_and_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("keybindings.json");
        let rule = |key: &str| Keybinding {
            command: "settings.open".into(),
            key: key.into(),
            when: None,
        };
        let ResultState::Data(first) = sync(&file, None, Some(vec![rule("ctrl+k")]), true).unwrap()
        else {
            panic!("new file")
        };
        assert!(matches!(
            read(&file, Some(first.revision)).unwrap(),
            ResultState::Unchanged
        ));
        let external = serde_json::to_vec(&vec![rule("ctrl+l")]).unwrap();
        std::fs::write(&file, &external).unwrap();
        assert!(matches!(
            sync(
                &file,
                Some(first.revision),
                Some(vec![rule("ctrl+p")]),
                false
            )
            .unwrap(),
            ResultState::Conflict
        ));
        assert_eq!(std::fs::read(&file).unwrap(), external);
        std::fs::write(&file, b"[").unwrap();
        assert!(sync(&file, Some(first.revision), Some(vec![]), true).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), b"[");
    }
}
