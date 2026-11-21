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
        // After the lanes: a write one of them had begun lands first.
        self.call(store::restore::interrupt_running).await?;
        self.changed();
        Ok(())
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
}
