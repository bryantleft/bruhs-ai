//! Capture a user-selected window into the initiating thread's draft.
use super::*;
#[cfg(target_os = "macos")]
mod shortcut;

#[derive(Default)]
pub(super) struct CaptureBinding {
    #[cfg(target_os = "macos")]
    registration: Option<(shortcut::Shortcut, Task<()>)>,
    key: Option<String>,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum CaptureSound {
    Click,
    #[default]
    Whoosh,
    Off,
}
impl CaptureSound {
    const OPTIONS: &'static [(&'static str, &'static str)] =
        &[("click", "Click"), ("whoosh", "Whoosh"), ("off", "Off")];

    pub(super) fn from_key(value: &str) -> Option<Self> {
        match value {
            "click" => Some(Self::Click),
            "whoosh" => Some(Self::Whoosh),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::Whoosh => "whoosh",
            Self::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    pub sound: CaptureSound,
    include_text: bool,
    global: bool,
    disable_flash: bool,
    disable_animations: bool,
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
struct WindowTarget {
    window_id: u32,
    owner_pid: i32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn bruhs_front_snapshot_target(target: *mut WindowTarget) -> bool;
    fn bruhs_snapshot_control_text(target: WindowTarget) -> *mut std::ffi::c_char;
    fn bruhs_snapshot_feedback(
        target: WindowTarget,
        flash: bool,
        animate: bool,
        bytes: *const u8,
        count: usize,
    );
}

struct Capture {
    target: Option<WindowTarget>,
    image: Vec<u8>,
    text: Result<String, String>,
}

#[cfg(target_os = "macos")]
fn visible_text(path: &std::path::Path, target: Option<WindowTarget>) -> Result<String, String> {
    use std::ffi::{c_char, CStr, CString};
    unsafe extern "C" {
        fn bruhs_snapshot_visible_text(path: *const c_char) -> *mut c_char;
        fn bruhs_free_snapshot_text(value: *mut c_char);
    }
    let path = CString::new(path.as_os_str().as_encoded_bytes()).map_err(|e| e.to_string())?;
    // The native bridge returns an owned UTF-8 copy, independent of the autorelease pool.
    unsafe {
        // A direct capture has a verified window identity. Picker output only
        // identifies the selected pixels, so use Vision on those pixels.
        if let Some(target) = target {
            let ptr = bruhs_snapshot_control_text(target);
            if !ptr.is_null() {
                let text = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                bruhs_free_snapshot_text(ptr);
                if !text.trim().is_empty() {
                    return Ok(text);
                }
            }
        }
        let ptr = bruhs_snapshot_visible_text(path.as_ptr());
        if ptr.is_null() {
            return Err("The image was captured, but text recognition failed.".into());
        }
        let text = CStr::from_ptr(ptr).to_string_lossy().into_owned();
        bruhs_free_snapshot_text(ptr);
        Ok(text)
    }
}

#[cfg(target_os = "macos")]
fn pick_window(
    prefs: Preferences,
    target: Option<WindowTarget>,
) -> Result<Option<Capture>, String> {
    use std::process::Command;
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("selected-window.png");
    let mut picker = Command::new("/usr/sbin/screencapture");
    picker.args(["-o", "-t", "png"]);
    if let Some(target) = target {
        picker.arg(format!("-l{}", target.window_id));
    } else {
        picker.args(["-i", "-w"]);
    }
    if prefs.sound != CaptureSound::Click {
        picker.arg("-x");
    }
    let output = picker.arg(&path).output().map_err(|e| e.to_string())?;
    if !path.is_file() {
        let error = String::from_utf8_lossy(&output.stderr);
        let error = error.trim();
        if target.is_none() && (error.is_empty() || error.eq_ignore_ascii_case("user canceled")) {
            return Ok(None);
        }
        return Err(if error.is_empty() {
            "The window closed or macOS denied screen capture.".to_owned()
        } else {
            error.to_owned()
        });
    }
    if !output.status.success() {
        return Err("The system could not finish capturing the selected window.".into());
    }
    if path.metadata().map_err(|e| e.to_string())?.len()
        > bruhs_models::logic::attachments::MAX_IMAGE_BYTES
    {
        return Err(
            "The captured window exceeds the 10 MB image limit. Try a smaller window.".into(),
        );
    }
    let image = std::fs::read(&path).map_err(|e| e.to_string())?;
    if !image.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The capture did not produce a PNG image.".into());
    }
    if prefs.sound == CaptureSound::Whoosh {
        let _ = Command::new("/usr/bin/afplay")
            .arg("/System/Library/Sounds/Blow.aiff")
            .status();
    }
    let text = if prefs.include_text {
        visible_text(&path, target)
    } else {
        Ok(String::new())
    };
    Ok(Some(Capture {
        target,
        image,
        text,
    }))
}

#[cfg(not(target_os = "macos"))]
fn pick_window(_: Preferences, _: Option<WindowTarget>) -> Result<Option<Capture>, String> {
    Err("Window capture is available on macOS.".into())
}

impl Workspace {
    pub(super) fn sync_capture_shortcut(&mut self, cx: &mut Context<'_, Self>) {
        let rule = self
            .keybindings
            .iter()
            .find(|r| r.command == "snapshot.capture" && !r.key.is_empty() && r.when.is_none());
        let key = self
            .snapshots
            .global
            .then(|| rule.map(|r| r.key.clone()).unwrap_or_default());
        if self.capture_binding.key == key {
            return;
        }
        self.capture_binding = CaptureBinding {
            key: key.clone(),
            ..Default::default()
        };
        #[cfg(target_os = "macos")]
        if let Some(key) = key {
            match shortcut::Shortcut::register(&key) {
                Ok((registration, rx)) => {
                    let window = self.workflow_window;
                    let listen = cx.spawn(async move |this, cx| {
                        while rx.recv().await.is_ok() {
                            let _ = window.update(cx, |_, window, cx| {
                                this.update(cx, |this, cx| {
                                    let mut target = WindowTarget::default();
                                    // Resolve before changing the bruhs window or its focus.
                                    if unsafe { bruhs_front_snapshot_target(&mut target) } {
                                        this.start_window_capture(Some(target), window, cx);
                                    } else {
                                        this.meetings_hub.update(cx, |hub, cx| {
                                            hub.show_notice(
                                                "No capturable window is focused.",
                                                true,
                                                cx,
                                            )
                                        });
                                    }
                                })
                            });
                        }
                    });
                    self.capture_binding.registration = Some((registration, listen));
                }
                Err(message) => self.capture_binding.error = Some(message.into()),
            }
        }
        cx.notify();
    }

    pub(super) fn capture_window(
        &mut self,
        _: &CaptureWindow,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.start_window_capture(None, window, cx);
    }

    fn start_window_capture(
        &mut self,
        target: Option<WindowTarget>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.picking_window {
            return;
        }
        let Some(thread_id) = self.active_thread().map(|t| t.id.clone()) else {
            self.meetings_hub.update(cx, |hub, cx| {
                hub.show_notice("Open a thread to attach a window capture.", false, cx)
            });
            return;
        };
        self.open_chat_view(&OpenChatView, window, cx);
        self.focus_chat_input(&FocusChatInput, window, cx);
        self.picking_window = true;
        let prefs = self.snapshots;
        let work = cx.background_spawn(async move { pick_window(prefs, target) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| this.finish_capture(&thread_id, result, cx));
        })
        .detach();
    }

    fn finish_capture(
        &mut self,
        thread_id: &str,
        result: Result<Option<Capture>, String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.picking_window = false;
        cx.notify();
        let notice = match result {
            Ok(None) => return,
            Err(error) => error,
            Ok(Some(capture)) => {
                let Some(thread) = self.threads.iter().find(|thread| thread.id == thread_id) else {
                    self.meetings_hub.update(cx, |hub, cx| {
                        hub.show_notice(
                            "Capture discarded because the original thread was closed.",
                            false,
                            cx,
                        )
                    });
                    return;
                };
                #[cfg(target_os = "macos")]
                if self.active_thread_id.as_deref() == Some(thread_id) {
                    // The system picker withholds the selected window's location.
                    unsafe {
                        bruhs_snapshot_feedback(
                            capture.target.unwrap_or_default(),
                            capture.target.is_some() && !self.snapshots.disable_flash,
                            !self.snapshots.disable_animations,
                            capture.image.as_ptr(),
                            capture.image.len(),
                        );
                    }
                }
                thread.chat.update(cx, |chat, cx| {
                    chat.enqueue_named_bytes("window.png".into(), capture.image, cx);
                    if let Ok(text) = &capture.text {
                        if !text.trim().is_empty() {
                            chat.enqueue_named_bytes(
                                "window-text.txt".into(),
                                text.as_bytes().to_vec(),
                                cx,
                            );
                        }
                    }
                });
                match capture.text {
                    Err(error) => error,
                    Ok(_) => return,
                }
            }
        };
        self.meetings_hub
            .update(cx, |hub, cx| hub.show_notice(notice, true, cx));
    }

    pub(super) fn render_snapshot_rows(&self, cx: &Context<'_, Self>) -> Vec<AnyElement> {
        let supported = cfg!(target_os = "macos");
        let mut rows = vec![self.settings_row(
            "desktop-settings-row-snapshot-window",
            "Capture a window",
            if supported {
                "Choose a window with the system picker and attach it to your current draft."
            } else {
                "Window capture is available on macOS."
            },
            on_press_view(
                web_button(
                    "snapshot-pick",
                    WebButtonVariant::Outline,
                    WebButtonSize::Small,
                    ButtonState::disabled_if(
                        !supported || self.picking_window || self.active_thread().is_none(),
                    ),
                )
                .child("Choose window"),
                cx,
                |this, window, cx| this.capture_window(&CaptureWindow, window, cx),
            )
            .into_any_element(),
        )];
        if !supported {
            return rows;
        }
        if let Some((index, rule)) = self
            .keybindings
            .iter()
            .enumerate()
            .find(|(_, r)| r.command == "snapshot.capture")
        {
            rows.push(
                self.settings_row(
                    "desktop-settings-row-snapshot-shortcut",
                    "Capture shortcut",
                    "Choose a key combination for the window picker.",
                    on_press_view(
                        web_button(
                            "snapshot-shortcut",
                            WebButtonVariant::Outline,
                            WebButtonSize::Small,
                            ButtonState::Enabled,
                        )
                        .child(if rule.key.is_empty() {
                            "Set shortcut".into()
                        } else {
                            rule.key.clone()
                        }),
                        cx,
                        move |this, window, cx| {
                            this.set_settings_section(SettingsSection::Keyboard, cx);
                            this.keybinding_filter.update(cx, |field, cx| {
                                field.set_value("snapshot.capture", window, cx)
                            });
                            this.select_keybinding(index, window, cx);
                        },
                    )
                    .into_any_element(),
                ),
            );
        }
        rows.push(self.settings_row(
            "desktop-settings-row-snapshot-global",
            "System-wide shortcut",
            self.capture_binding.error.clone().unwrap_or_else(|| {
                "Capture the front window from any app with the first unconditional capture shortcut.".into()
            }),
            settings_ui::switch(
                "snapshot-global",
                self.snapshots.global,
                false,
                "System-wide capture shortcut",
                cx,
                |this, _, cx| {
                    this.snapshots.global = !this.snapshots.global;
                    this.sync_capture_shortcut(cx);
                    this.persist_and_notify(cx);
                },
            ),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-snapshot-text", "Include app text",
            "Attach window labels and controls when macOS grants Accessibility access. The picker uses on-device text recognition.",
            settings_ui::switch("snapshot-include-text",self.snapshots.include_text,false,"Include app text",cx,|this,_,cx|{
                this.snapshots.include_text = !this.snapshots.include_text;
                this.persist_and_notify(cx);
            }),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-snapshot-sound",
            "Capture sound",
            "Sound after capturing a window.",
            settings_ui::control_column(self.choice_control(
                "settings-capture-sound",
                "Capture sound",
                self.snapshots.sound.key(),
                CaptureSound::OPTIONS,
                cx,
            )),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-snapshot-flash",
            "Capture flash",
            "Briefly outline the captured window when using the system-wide shortcut.",
            settings_ui::switch(
                "snapshot-flash",
                !self.snapshots.disable_flash,
                false,
                "Capture flash",
                cx,
                |this, _, cx| {
                    this.snapshots.disable_flash = !this.snapshots.disable_flash;
                    this.persist_and_notify(cx);
                },
            ),
        ));
        rows.push(self.settings_row(
            "desktop-settings-row-snapshot-animations", "Capture animations", "Animate the captured thumbnail into the draft. Follows the macOS Reduce Motion preference.",
            settings_ui::switch("snapshot-animations", !self.snapshots.disable_animations, false, "Capture animations", cx, |this,_,cx|{
                this.snapshots.disable_animations = !this.snapshots.disable_animations;
                this.persist_and_notify(cx);
            }),
        ));
        rows
    }
}
