//! A delayed reset that a newer one replaces: scheduling cancels the pending
//! one, so a flash shown after another is not cleared by the older timer.

use gloo_timers::callback::Timeout;
use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct ResetTimer(StoredValue<Option<Timeout>, LocalStorage>);

impl ResetTimer {
    pub fn new() -> Self {
        Self(StoredValue::new_local(None))
    }

    /// Run `reset` after `ms`, cancelling any reset still pending.
    pub fn after(self, ms: u32, reset: impl FnOnce() + 'static) {
        // Dropping the previous `Timeout` clears it (a no-op once it fired).
        self.0.try_set_value(Some(Timeout::new(ms, reset)));
    }

    /// Cancel the pending reset, if any.
    pub fn cancel(self) {
        self.0.try_set_value(None);
    }
}

impl Default for ResetTimer {
    fn default() -> Self {
        Self::new()
    }
}
