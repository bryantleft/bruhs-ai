//! AppKit receives command-key releases before the focused GPUI view sees them.
use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventModifierFlags, NSEventType};
use std::{cell::Cell, ptr::NonNull, rc::Rc};

pub(super) struct Hold {
    monitor: Retained<AnyObject>,
    released: Rc<Cell<bool>>,
}
impl Hold {
    pub(super) fn begin() -> Option<Self> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        // A queued GPUI dispatch can run after AppKit has cleared its current event.
        let code = app
            .currentEvent()
            .filter(|event| event.r#type() == NSEventType::KeyDown)
            .map(|event| event.keyCode());
        let released = Rc::new(Cell::new(false));
        let up = released.clone();
        let callback = RcBlock::new(move |raw: NonNull<NSEvent>| {
            // SAFETY: AppKit owns the NSEvent for the duration of the monitor callback.
            let e = unsafe { raw.as_ref() };
            let kind = e.r#type();
            if (kind == NSEventType::FlagsChanged
                && !e.modifierFlags().contains(NSEventModifierFlags::Command))
                || (kind == NSEventType::KeyUp && code.is_none_or(|q| q == e.keyCode()))
                || (kind == NSEventType::KeyDown && !e.isARepeat())
            {
                up.set(true);
            }
            raw.as_ptr()
        });
        let mask = NSEventMask::KeyDown | NSEventMask::KeyUp | NSEventMask::FlagsChanged;
        // SAFETY: Called on the main thread (verified by MainThreadMarker) with a valid RcBlock.
        let monitor =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &callback) }?;
        Some(Self { monitor, released })
    }
    pub(super) fn is_down(&self) -> bool {
        !self.released.get()
    }
}
impl Drop for Hold {
    fn drop(&mut self) {
        // SAFETY: `self.monitor` was returned by `addLocalMonitorForEventsMatchingMask_handler` and is removed once.
        unsafe { NSEvent::removeMonitor(&self.monitor) }
    }
}
