//! Only a provider-accepted, live turn can leave an interrupted-run marker.
//! The sender owns the file lock. Dropping the process releases it without
//! clearing the marker, whereas Stop and normal completion retire it first.
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use uuid::Uuid;

#[derive(Clone)]
pub(super) struct Run {
    pub id: Uuid,
    pub depth: u8,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    lock: Weak<File>,
    session: Option<String>,
    waiting: std::collections::BTreeSet<String>,
    marked: bool,
    retired: bool,
}

impl State {
    fn write(&mut self, id: Uuid, allowed: bool) {
        if allowed == self.marked {
            return;
        }
        let Some(lock) = self.lock.upgrade() else {
            return;
        };
        let mut file = lock.as_ref();
        let result = if allowed {
            file.seek(SeekFrom::Start(0))
                .and_then(|_| file.write_all(id.as_bytes()))
        } else {
            file.set_len(0)
        };
        if let Err(error) = result.and_then(|()| file.sync_data()) {
            tracing::warn!(%error, "Could not update interrupted-turn marker");
        } else {
            self.marked = allowed;
        }
    }
}

impl Run {
    pub fn new(depth: u8) -> Self {
        Self {
            id: Uuid::new_v4(),
            depth,
            state: Default::default(),
        }
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
    pub fn attach(&self, lock: &Arc<File>) -> bool {
        let mut state = self.lock();
        if state.retired {
            return false;
        }
        state.lock = Arc::downgrade(lock);
        true
    }
    pub fn marked_for(&self, session: Option<&str>) -> bool {
        let state = self.lock();
        state.marked && !state.retired && state.session.as_deref() == session
    }
    pub fn confirm(&self, session: &str) {
        let mut state = self.lock();
        state.session = Some(session.to_owned());
        let allowed = state.waiting.is_empty() && !state.retired;
        state.write(self.id, allowed);
    }
    pub fn wait_for(&self, request: &str) {
        let mut state = self.lock();
        state.waiting.insert(request.to_owned());
        state.write(self.id, false);
    }
    pub fn answered(&self, request: &str) {
        let mut state = self.lock();
        state.waiting.remove(request);
        let allowed = state.session.is_some() && state.waiting.is_empty() && !state.retired;
        state.write(self.id, allowed);
    }
    pub fn retire(&self) {
        let mut state = self.lock();
        state.write(self.id, false);
        state.retired = true;
    }
}

pub(super) struct Continuation {
    pub session: String,
    pub depth: u8,
    pub claim: Option<File>,
}

#[cfg(test)]
mod tests {
    use super::Run;
    use std::sync::Arc;

    #[test]
    fn recovery_marker_tracks_acceptance_outstanding_requests_and_stop() {
        let path = tempfile::NamedTempFile::new().unwrap();
        let file = Arc::new(path.reopen().unwrap());
        let run = Run::new(0);
        assert!(run.attach(&file));
        let marker = || std::fs::read(path.path()).unwrap();
        assert!(marker().is_empty());
        run.confirm("current");
        assert!(!run.marked_for(Some("previous")));
        assert_eq!(marker(), run.id.as_bytes());
        run.wait_for("permission");
        run.wait_for("secret");
        run.answered("permission");
        assert!(marker().is_empty());
        run.answered("secret");
        assert_eq!(marker(), run.id.as_bytes());
        run.retire();
        run.confirm("late init");
        assert!(marker().is_empty());
        assert!(!run.attach(&file));
    }
}
