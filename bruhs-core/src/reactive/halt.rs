//! Halting the crew before a restore (D15, ruling F19): the worker stops
//! and starts nothing more, and the runs it was running end as a server
//! stop leaves them. Unlike `StopAll`, nothing is paused, no grant is
//! deleted and no approval is denied: the live data is about to become a
//! kept pre-restore set, and putting it back must give the owner the crew
//! they had.

use std::sync::{Mutex, PoisonError};
#[cfg(unix)]
use std::time::{Duration, Instant};

use tokio::task::{AbortHandle, JoinHandle};

#[cfg(unix)]
use super::{store, ReactiveError, ReactiveRuntime};

/// How long a halt waits for the stopped worker's lanes to drop.
#[cfg(unix)]
const STOP_WAIT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const STOP_POLL: Duration = Duration::from_millis(10);
/// How long a resume waits for a worker the halt stopped to let go of the
/// runtime (final review B minor): past [`STOP_WAIT`], a lane still
/// dropping its provider turn holds it.
#[cfg(unix)]
const RESUME_WAIT: Duration = Duration::from_secs(30);
#[cfg(unix)]
const RESUME_POLL: Duration = Duration::from_millis(50);

/// The running worker, so a restore can stop it.
#[derive(Debug, Default)]
pub(super) struct WorkerSlot(Mutex<Option<AbortHandle>>);

impl WorkerSlot {
    /// Remembers the worker `start` just spawned.
    pub(super) fn keep(&self, worker: &JoinHandle<()>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(worker.abort_handle());
    }

    #[cfg(unix)]
    fn take(&self) -> Option<AbortHandle> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

#[cfg(unix)]
impl ReactiveRuntime {
    /// Stops the worker the way a server stop does: its lanes, and the
    /// provider turns they run, are dropped, nothing more is claimed, and
    /// every run left running is marked interrupted. Bruhs stay as they
    /// are, with their grants and open approvals (ruling F19). `start`
    /// may run the worker again, as when a restore can't go ahead.
    pub async fn halt_for_restore(&self) -> Result<(), ReactiveError> {
        if let Some(worker) = self.worker.take() {
            worker.abort();
            let until = Instant::now() + STOP_WAIT;
            while !worker.is_finished() && Instant::now() < until {
                tokio::time::sleep(STOP_POLL).await;
            }
        }
        // Queued behind any call a lane already made: a lane whose write
        // was in the blocking pool may still end its run in its real
        // state, which is harmless; every run left running is marked.
        self.call(store::restore::interrupt_running).await?;
        self.changed();
        Ok(())
    }

    /// Runs the worker again after a restore that was called off, and asks
    /// the sync loop for a pass so the bruhs' browsers come back. A worker
    /// the halt stopped may not have let go yet: the start is tried again
    /// until it has, for up to `RESUME_WAIT` (final review B minor).
    pub async fn resume_after_restore(
        &self,
        app: std::sync::Arc<crate::AppState>,
    ) -> Result<(), ReactiveError> {
        let until = Instant::now() + RESUME_WAIT;
        let started = loop {
            match self.start(std::sync::Arc::clone(&app)) {
                Err(ReactiveError::Conflict(_)) if Instant::now() < until => {
                    tokio::time::sleep(RESUME_POLL).await;
                }
                started => break started.map(drop),
            }
        };
        self.changed();
        started
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::sync::Arc;

    use bruhs_models::crew::BruhStatus;
    use bruhs_models::reactive::{ReactiveAgent, ReactiveChange, ReactiveCommand};

    use super::ReactiveRuntime;

    /// Ruling F19: a halt stops the worker, ends the run in flight as a
    /// server stop does and leaves the bruh unpaused with its grant; the
    /// worker can run again after.
    #[tokio::test]
    async fn a_halt_stops_the_worker_and_keeps_the_crew() {
        let directory = tempfile::tempdir().unwrap();
        let db = directory.path().join("reactive.sqlite3");
        let runtime = ReactiveRuntime::open(&db).await.unwrap();
        let work = directory.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        runtime
            .command(ReactiveCommand::Apply {
                change: ReactiveChange::SaveAgent {
                    agent: ReactiveAgent {
                        id: "bruh-1".into(),
                        name: "Bruh".into(),
                        instructions: "Prepare drafts for review.".into(),
                        memory: String::new(),
                        model: "claude-sonnet-4-6".into(),
                        project_path: work.to_string_lossy().into_owned(),
                        enabled: true,
                    },
                },
            })
            .await
            .unwrap();
        runtime
            .command(ReactiveCommand::Chat {
                agent_id: "bruh-1".into(),
                message: "Draft the notes".into(),
                run_id: None,
            })
            .await
            .unwrap();
        {
            // In flight, as a lane leaves it, so the worker claims nothing.
            let db = rusqlite::Connection::open(&db).unwrap();
            db.execute(
                "UPDATE reactive_runs SET status='running', payload=json_set(payload,'$.status','running')",
                [],
            )
            .unwrap();
            db.execute(
                "INSERT INTO reactive_approval_grants(agent_id,fingerprint,created) VALUES('bruh-1','fp',1)",
                [],
            )
            .unwrap();
        }
        let app = Arc::new(
            crate::AppState::builder()
                .secret_store(crate::secrets::SecretStore::in_memory())
                .connections(crate::connections::Connections::in_memory())
                .build(),
        );
        let worker = runtime.start(Arc::clone(&app)).unwrap();
        assert!(runtime.start(Arc::clone(&app)).is_err(), "one worker");

        runtime.halt_for_restore().await.unwrap();
        assert!(worker.is_finished(), "the worker stopped");
        let snapshot = runtime.snapshot().await.unwrap();
        assert!(snapshot
            .bruhs
            .iter()
            .all(|bruh| bruh.status != BruhStatus::Paused));
        let db = rusqlite::Connection::open(&db).unwrap();
        let (status, grants): (String, i64) = db
            .query_row(
                "SELECT (SELECT status FROM reactive_runs), (SELECT count(*) FROM reactive_approval_grants)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((status.as_str(), grants), ("interrupted", 1));
        // A restore that can't go ahead runs the worker again.
        let again = runtime.start(app).unwrap();
        again.abort();
    }

    /// Final review B minor: a resume waits for a stopped worker that
    /// hasn't let go of the runtime yet, then runs it again.
    #[tokio::test]
    async fn a_resume_waits_for_the_stopped_worker() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = ReactiveRuntime::open(&directory.path().join("reactive.sqlite3"))
            .await
            .unwrap();
        let app = Arc::new(
            crate::AppState::builder()
                .secret_store(crate::secrets::SecretStore::in_memory())
                .connections(crate::connections::Connections::in_memory())
                .build(),
        );
        // A lane of the stopped worker still drops its turn, past the
        // halt's wait.
        runtime
            .started
            .store(true, std::sync::atomic::Ordering::Release);
        let started = Arc::clone(&runtime.started);
        let release = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            started.store(false, std::sync::atomic::Ordering::Release);
        });
        runtime
            .resume_after_restore(Arc::clone(&app))
            .await
            .unwrap();
        release.await.unwrap();
        assert!(runtime.start(app).is_err(), "the worker runs again");
        runtime.halt_for_restore().await.unwrap();
    }
}
