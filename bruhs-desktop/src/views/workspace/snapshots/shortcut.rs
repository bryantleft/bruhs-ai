//! An OS-owned hotkey registration. No keyboard event tap or input monitoring.
use async_channel::{Receiver, Sender};
use bruhs_models::logic::keybindings::parse_chord;
use std::ffi::{c_char, c_void, CString};
use std::ptr::NonNull;

unsafe extern "C" {
    fn bruhs_register_snapshot_shortcut(
        key: *const c_char,
        flags: u32,
        notify: extern "C" fn(*mut c_void),
        context: *mut c_void,
    ) -> *mut c_void;
    fn bruhs_release_snapshot_shortcut(handle: *mut c_void);
}

pub(super) struct Shortcut {
    handle: NonNull<c_void>,
    _sender: Box<Sender<()>>,
}
impl Drop for Shortcut {
    fn drop(&mut self) {
        // Unregister on the GPUI thread before releasing the callback's boxed sender.
        unsafe {
            bruhs_release_snapshot_shortcut(self.handle.as_ptr());
        }
    }
}

extern "C" fn pressed(context: *mut c_void) {
    let sender = unsafe { &*context.cast::<Sender<()>>() };
    let _ = sender.try_send(());
}

impl Shortcut {
    pub fn register(key: &str) -> Result<(Self, Receiver<()>), &'static str> {
        let chord = parse_chord(key, true).ok_or("Record a shortcut first.")?;
        if !(chord.ctrl || chord.command || chord.alt) {
            return Err("Include Command, Control or Option for a system-wide shortcut.");
        }
        let key = CString::new(chord.key).map_err(|_| "Unsupported shortcut.")?;
        let (tx, rx) = async_channel::bounded(1);
        let mut sender = Box::new(tx);
        let flags = u32::from(chord.command)
            | u32::from(chord.ctrl) << 1
            | u32::from(chord.alt) << 2
            | u32::from(chord.shift) << 3;
        let raw = unsafe {
            bruhs_register_snapshot_shortcut(
                key.as_ptr(),
                flags,
                pressed,
                (&raw mut *sender).cast(),
            )
        };
        let handle = NonNull::new(raw)
            .ok_or("macOS could not register this shortcut. Choose another combination.")?;
        Ok((
            Self {
                handle,
                _sender: sender,
            },
            rx,
        ))
    }
}
