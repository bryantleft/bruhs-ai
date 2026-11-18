//! Web `CopyButton`'s confirmation: after a copy, the button shows a green
//! check for 1.5s, and another copy restarts the moment.

use gpui::{AsyncApp, Context, SharedString};
use std::time::Duration;

/// How long a copy button shows its check (web: 1.5s).
pub const COPIED_VISIBLE_MS: u64 = 1_500;

/// Which copy button shows its check, if any.
#[derive(Default)]
pub struct CopyFeedback {
    shown: Option<SharedString>,
    /// Bumped by every copy, so an older timer leaves a newer check alone.
    generation: u64,
}

impl CopyFeedback {
    /// Whether the button `key` shows its check.
    pub fn is_shown(&self, key: &str) -> bool {
        self.shown.as_deref() == Some(key)
    }

    /// Show the check on `key`, held in `view` at `field`, then hide it
    /// after [`COPIED_VISIBLE_MS`] unless another copy happened since.
    pub fn show<V: 'static>(
        view: &mut V,
        field: fn(&mut V) -> &mut CopyFeedback,
        key: impl Into<SharedString>,
        cx: &mut Context<'_, V>,
    ) {
        let feedback = field(view);
        feedback.generation = feedback.generation.wrapping_add(1);
        feedback.shown = Some(key.into());
        let generation = feedback.generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(COPIED_VISIBLE_MS))
                .await;
            let _ = this.update(cx, |view, cx| {
                let feedback = field(view);
                if feedback.generation == generation {
                    feedback.shown = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{CopyFeedback, COPIED_VISIBLE_MS};
    use gpui::{AppContext as _, TestAppContext};
    use std::time::Duration;

    struct Holder(CopyFeedback);

    fn copy(holder: &gpui::Entity<Holder>, key: &'static str, cx: &mut TestAppContext) {
        holder.update(cx, |holder, cx| {
            CopyFeedback::show(holder, |holder| &mut holder.0, key, cx);
        });
    }

    fn shown(holder: &gpui::Entity<Holder>, key: &str, cx: &mut TestAppContext) -> bool {
        holder.read_with(cx, |holder, _| holder.0.is_shown(key))
    }

    #[gpui::test]
    fn a_copy_shows_its_check_for_a_moment_and_a_second_copy_restarts_it(cx: &mut TestAppContext) {
        let holder = cx.new(|_| Holder(CopyFeedback::default()));
        copy(&holder, "a", cx);
        assert!(shown(&holder, "a", cx));
        assert!(!shown(&holder, "b", cx));

        cx.executor()
            .advance_clock(Duration::from_millis(COPIED_VISIBLE_MS / 2));
        copy(&holder, "a", cx);
        cx.executor()
            .advance_clock(Duration::from_millis(COPIED_VISIBLE_MS / 2));
        cx.run_until_parked();
        assert!(
            shown(&holder, "a", cx),
            "the second copy restarts the moment"
        );

        cx.executor()
            .advance_clock(Duration::from_millis(COPIED_VISIBLE_MS / 2));
        cx.run_until_parked();
        assert!(!shown(&holder, "a", cx));

        // Copying another button moves the check there.
        copy(&holder, "a", cx);
        copy(&holder, "b", cx);
        assert!(!shown(&holder, "a", cx));
        assert!(shown(&holder, "b", cx));
    }
}
