//! App-owned motion follows the OS preference without polling or reading any
//! desktop content. Unknown/unsupported preferences use static feedback.
use gpui::{App, Global};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Preference {
    #[default]
    Reduced,
    Full,
}

impl Global for Preference {}

pub fn reduce_motion(cx: &App) -> bool {
    cx.try_global::<Preference>().copied().unwrap_or_default() == Preference::Reduced
}

pub fn init(cx: &mut App) {
    if cx.try_global::<Preference>().is_some() {
        return;
    }
    set_reduced_motion(true, cx);
    #[cfg(all(target_os = "macos", not(test)))]
    macos::install(cx);
}

pub(crate) fn set_reduced_motion(reduced: bool, cx: &mut App) {
    let preference = if reduced {
        Preference::Reduced
    } else {
        Preference::Full
    };
    if cx.try_global::<Preference>() != Some(&preference) {
        cx.set_global(preference);
        // A preference change must also replace an already-running animation.
        cx.refresh_windows();
    }
}

#[cfg(any(test, target_os = "macos"))]
fn listen(
    cx: &mut App,
    changes: async_channel::Receiver<()>,
    probe: impl Fn() -> bool + 'static,
) -> gpui::Task<()> {
    cx.spawn(async move |cx| {
        while changes.recv().await.is_ok() {
            cx.update(|cx| set_reduced_motion(probe(), cx));
        }
    })
}

#[cfg(all(target_os = "macos", not(test)))]
mod macos {
    use super::*;
    use block2::RcBlock;
    use objc2::{rc::Retained, runtime::ProtocolObject, MainThreadMarker};
    use objc2_app_kit::{NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification};
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
    use std::ptr::NonNull;

    struct Monitor {
        center: Retained<NSNotificationCenter>,
        token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
        _task: gpui::Task<()>,
    }

    impl Global for Monitor {}

    impl Drop for Monitor {
        fn drop(&mut self) {
            // SAFETY: This token came from this exact notification center.
            unsafe { self.center.removeObserver((*self.token).as_ref()) };
        }
    }

    pub(super) fn install(cx: &mut App) {
        if cx.try_global::<Monitor>().is_some() || MainThreadMarker::new().is_none() {
            return;
        }
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();
        // Notifications are invalidations, not queued preference values. A burst
        // needs only one pending read of the current setting on the UI thread.
        let (sender, receiver) = async_channel::bounded(1);
        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            let _ = sender.try_send(());
        });
        // SAFETY: no object/queue filter; the sendable callback only touches a
        // bounded thread-safe channel and never retains notification contents.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification),
                None,
                None,
                &block,
            )
        };
        // Subscribe before the initial read so a concurrent change is not lost.
        set_reduced_motion(workspace.accessibilityDisplayShouldReduceMotion(), cx);
        let task = listen(cx, receiver, || {
            NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
        });
        cx.set_global(Monitor {
            center,
            token,
            _task: task,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, rc::Rc};

    #[gpui::test]
    fn missing_preference_uses_static_feedback(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| assert!(reduce_motion(cx)));
    }

    #[gpui::test]
    fn preference_changes_are_coalesced_and_read_on_the_ui_thread(cx: &mut gpui::TestAppContext) {
        let setting = Rc::new(Cell::new(false));
        let probe_setting = setting.clone();
        let (sender, receiver) = async_channel::bounded(1);
        let task = cx.update(|cx| {
            init(cx); // Test builds never install the macOS observer or getter.
            listen(cx, receiver, move || probe_setting.get())
        });
        sender.try_send(()).unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(!reduce_motion(cx)));
        sender.try_send(()).unwrap();
        assert!(sender.try_send(()).is_err());
        setting.set(true);
        cx.run_until_parked();
        cx.update(|cx| assert!(reduce_motion(cx)));
        setting.set(false);
        sender.try_send(()).unwrap();
        cx.run_until_parked();
        cx.update(|cx| assert!(!reduce_motion(cx)));
        drop(task);
        cx.run_until_parked();
        assert!(
            sender.is_closed(),
            "dropping the monitor must release its receiver"
        );
    }
}
