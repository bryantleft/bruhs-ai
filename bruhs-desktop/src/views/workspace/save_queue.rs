//! Fixed batching window and one writer: continuous input must not starve saves.

#[derive(Default)]
pub(super) enum SaveQueue {
    #[default]
    Idle,
    Scheduled,
    Writing {
        dirty: bool,
    },
}

impl SaveQueue {
    /// True only for the first edit, which owns the non-resetting batching timer.
    pub(super) fn request(&mut self) -> bool {
        match self {
            Self::Idle => {
                *self = Self::Scheduled;
                true
            }
            Self::Scheduled => false,
            Self::Writing { dirty } => {
                *dirty = true;
                false
            }
        }
    }

    pub(super) fn begin_write(&mut self) {
        *self = Self::Writing { dirty: false };
    }

    /// Dirty state needs another batch after success, or explicit retry on failure.
    pub(super) fn finish(&mut self) -> bool {
        let dirty = matches!(self, Self::Writing { dirty: true });
        *self = Self::Idle;
        dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_edits_share_one_timer_and_changes_during_write_require_one_followup() {
        let mut queue = SaveQueue::default();
        assert!(queue.request());
        for _ in 0..10_000 {
            assert!(!queue.request());
        }
        queue.begin_write();
        for _ in 0..10_000 {
            assert!(!queue.request());
        }
        assert!(queue.finish());
        assert!(queue.request());
        queue.begin_write();
        assert!(!queue.finish());
    }
}
