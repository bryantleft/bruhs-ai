//! An editable host keymap. Disk and UI editors compare the same revision before writing.
use super::*;
use bruhs_models::logic::keybindings::Keybinding;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Default)]
pub(super) struct KeymapFile {
    path: Option<PathBuf>,
    revision: Option<[u8; 32]>,
    busy: bool,
    dirty: bool,
    open_requested: bool,
    watching: bool,
    pub(super) error: Option<String>,
    conflict: bool,
}
struct Contents {
    revision: [u8; 32],
    rules: Vec<Keybinding>,
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
    Ok(ResultState::Data(Contents {
        revision,
        rules: super::keybindings_ui::rules_from_saved(rules, 1),
    }))
}
enum ResultState {
    NoFile,
    Unchanged,
    Data(Contents),
    Conflict,
}
fn sync(
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
    let mut bytes = serde_json::to_vec_pretty(&rules)?;
    bytes.push(b'\n');
    let mut out = tempfile::NamedTempFile::new_in(parent)?;
    out.write_all(&bytes)?;
    out.as_file().sync_all()?;
    out.persist(path)?;
    Ok(ResultState::Data(Contents {
        revision: Sha256::digest(bytes).into(),
        rules,
    }))
}
impl KeymapFile {
    pub(super) fn new() -> Self {
        #[cfg(not(test))]
        let path = crate::paths::desktop_state_file().map(|p| p.with_file_name("keybindings.json"));
        #[cfg(test)]
        let path = None;
        Self {
            path,
            ..Default::default()
        }
    }
}
impl Workspace {
    pub(super) fn keymap_changed(&mut self, cx: &mut Context<'_, Self>) {
        self.sync_keybindings(cx);
        if self.keymap_file.revision.is_some() || self.keymap_file.busy {
            self.keymap_file.dirty = true;
            self.refresh_keymap_file(cx);
        }
    }
    pub(super) fn refresh_keymap_file(&mut self, cx: &mut Context<'_, Self>) {
        let state = &mut self.keymap_file;
        if state.busy || state.conflict {
            return;
        }
        let Some(path) = state.path.clone() else {
            return;
        };
        let create = state.open_requested;
        let revision = state.revision;
        let dirty = state.dirty;
        let local = self.keybindings.clone();
        let write = (dirty || create && revision.is_none()).then(|| local.clone());
        state.busy = true;
        let location = path.clone();
        let job = cx.background_spawn(async move { sync(&location, revision, write, create) });
        cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |this, cx| {
                this.keymap_file.busy = false;
                let open = this.keymap_file.open_requested;
                let newer = this.keybindings != local;
                match result {
                    Ok(ResultState::Unchanged) => {
                        if open {
                            this.keymap_file.open_requested = false;
                            cx.open_with_system(&path);
                        }
                        if newer {
                            this.refresh_keymap_file(cx);
                        }
                        if this.keymap_file.error.take().is_none() {
                            return;
                        }
                    }
                    Ok(ResultState::Data(data)) => {
                        let changed = Some(data.revision) != revision;
                        this.keymap_file.error = None;
                        this.keymap_file.revision = Some(data.revision);
                        if !newer {
                            if this.keybindings != data.rules {
                                this.keybindings = data.rules;
                                this.keybinding_index = 0;
                                this.keybinding_new_for = None;
                                this.keybinding_edit_open = false;
                                this.keybinding_recording = false;
                                this.sync_keybindings(cx);
                                this.persist(cx);
                            }
                            this.keymap_file.dirty = false;
                        } else if changed && !dirty && data.rules != local {
                            this.keymap_file.conflict = true;
                        }
                        if open {
                            this.keymap_file.open_requested = false;
                            cx.open_with_system(&path);
                        }
                        this.watch_keymap_file(cx);
                        if newer {
                            this.refresh_keymap_file(cx);
                        }
                    }
                    Ok(ResultState::Conflict) => {
                        this.keymap_file.conflict = true;
                        this.keymap_file.open_requested = false;
                    }
                    Ok(ResultState::NoFile) => {
                        this.keymap_file.revision = None;
                        if open {
                            this.refresh_keymap_file(cx);
                        }
                    }
                    Err(e) => {
                        this.keymap_file.error = Some(e.to_string());
                        this.keymap_file.open_requested = false;
                        if path.is_file() {
                            this.watch_keymap_file(cx);
                        }
                        if open && path.is_file() {
                            cx.open_with_system(&path);
                        }
                    }
                }
                if this.settings_section == SettingsSection::Keyboard && this.settings_visible() {
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn watch_keymap_file(&mut self, cx: &mut Context<'_, Self>) {
        if self.keymap_file.watching {
            return;
        }
        self.keymap_file.watching = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                if this
                    .update(cx, |this, cx| this.refresh_keymap_file(cx))
                    .is_err()
                {
                    return;
                }
            }
        })
        .detach();
    }
    pub(super) fn keymap_file_buttons(&self, cx: &Context<'_, Self>) -> AnyElement {
        let button = web_button(
            "settings-keymap-file",
            WebButtonVariant::Outline,
            WebButtonSize::Small,
            ButtonState::Enabled,
        )
        .aria_label("Open keybindings.json")
        .child("Open keybindings.json");
        let mut controls = div()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(on_press_view(button, cx, |this, _, cx| {
                if this.keymap_file.conflict {
                    if let Some(path)=&this.keymap_file.path {cx.open_with_system(path);}
                    return;
                }
                this.keymap_file.open_requested = true;
                this.refresh_keymap_file(cx);
            }));
        if self.keymap_file.conflict {
            let reload = web_button(
                "keymap-file-reload",
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .aria_label("Load keybindings from disk")
            .child("Load from disk");
            controls = controls
                .child("Keybindings changed on disk. Load the file to resolve the conflict.")
                .child(on_press_view(reload, cx, |this, _, cx| {
                    this.keymap_file.conflict = false;
                    this.keymap_file.revision = None;
                    this.keymap_file.dirty = false;
                    this.refresh_keymap_file(cx);
                }));
        }
        controls
            .when_some(self.keymap_file.error.clone(), |el, e| el.child(e))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{Keybinding, ResultState, read, sync};
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
