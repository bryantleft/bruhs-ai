//! An editable host keymap. Disk and UI editors compare the same revision before writing.
use super::*;
use bruhs_core::keybinding_file::{sync, ResultState};
use std::{path::PathBuf, time::Duration};

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
        if self.keymap_file.path.is_some() {
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
        let create = state.open_requested && !state.dirty;
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
                                if this.remote_file_button_state().is_none() {
                                    this.keybinding_index = 0;
                                    this.keybinding_when_open = None;
                                    this.keybinding_edit_open = false;
                                    this.keybinding_recording = None;
                                }
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
                        this.watch_keymap_file(cx);
                        if open || this.keymap_file.dirty {
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
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            if this
                .update(cx, |this, cx| this.refresh_keymap_file(cx))
                .is_err()
            {
                return;
            }
        })
        .detach();
    }
    pub(super) fn keymap_file_buttons(&self, cx: &Context<'_, Self>) -> AnyElement {
        let button = web_button(
            "settings-keymap-file",
            WebButtonVariant::Outline,
            WebButtonSize::IconSmall,
            ButtonState::disabled_if(self.remote_file_button_state() == Some(false)),
        )
        .debug_selector(|| "settings-keymap-file".into())
        .aria_label("Open keybindings.json")
        .tooltip(|w, cx| Tooltip::new("Open keybindings.json").build(w, cx))
        .child(super::updates_ui::icon_svg(
            assets::FILE_TEXT_ICON_PATH,
            15.,
            theme::rambutan_300(),
        ));
        let mut controls = div()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(on_press_view(button, cx, |this, _, cx| {
                if this.open_selected_shortcut_file(cx) {
                    return;
                }
                if this.keymap_file.conflict {
                    if let Some(path) = &this.keymap_file.path {
                        cx.open_with_system(path);
                    }
                    return;
                }
                this.keymap_file.open_requested = true;
                this.refresh_keymap_file(cx);
            }));
        if self.remote_file_button_state().is_some() {
            return controls.into_any_element();
        }
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
