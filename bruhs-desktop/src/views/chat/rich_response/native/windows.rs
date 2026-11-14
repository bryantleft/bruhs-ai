//! WebView2 construction pumps Win32 messages. Keep that pump outside GPUI's
//! paint/update borrow so ordinary foreground tasks can safely run within it.

use gpui::{App, EntityId, Task, Window};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, WindowHandle};

struct ParentWindow {
    handle: RawWindowHandle,
    active: Rc<Cell<bool>>,
}

impl HasWindowHandle for ParentWindow {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        if !self.active.get() {
            return Err(HandleError::Unavailable);
        }
        // SAFETY: this is the GPUI parent's Win32 handle on its owning thread.
        // Its pre-close observer cancels this host before destroying the parent.
        // Wry consumes the borrowed handle at the start of child construction;
        // Win32 rejects creation if the parent closes during its message pump.
        Ok(unsafe { WindowHandle::borrow_raw(self.handle) })
    }
}

pub(super) struct PendingWebView {
    active: Rc<Cell<bool>>,
    ready: Rc<RefCell<Option<Result<wry::WebView, String>>>>,
    _task: Task<()>,
}

impl PendingWebView {
    pub(super) fn start(
        builder: wry::WebViewBuilder<'static>,
        window: &Window,
        app: &App,
        entity: EntityId,
    ) -> Result<Self, String> {
        let handle = HasWindowHandle::window_handle(window)
            .map_err(|error| error.to_string())?
            .as_raw();
        let active = Rc::new(Cell::new(true));
        let parent = ParentWindow {
            handle,
            active: Rc::clone(&active),
        };
        let ready = Rc::new(RefCell::new(None));
        let completion = Rc::clone(&ready);
        let task = app.spawn(async move |async_app| {
            // Deliberately outside app.update(): WebView2 pumps messages while
            // creating its environment/controller, including GPUI save tasks.
            let result = builder
                .build_as_child(&parent)
                .map_err(super::platform_error);
            if parent.active.get() {
                *completion.borrow_mut() = Some(result);
                async_app.update(|current_app| current_app.notify(entity));
            }
        });
        Ok(Self {
            active,
            ready,
            _task: task,
        })
    }

    pub(super) fn take_ready(&self) -> Option<Result<wry::WebView, String>> {
        self.ready.borrow_mut().take()
    }
}

impl Drop for PendingWebView {
    fn drop(&mut self) {
        self.active.set(false);
        // A completed but not-yet-painted child also belongs to this host.
        self.ready.borrow_mut().take();
    }
}
