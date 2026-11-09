//! In-memory registry of background tasks (backgrounded Bash commands)
//! started by agent turns. Listeners get every change as a [`TaskEvent`].
//!
//! Listeners run on the mutating thread, after every lock is released, so a
//! listener may call back into the registry (including `subscribe` /
//! `unsubscribe`). Finished tasks are kept for the UI up to
//! [`MAX_FINISHED_TASKS`]; older ones are evicted with a `Removed` event.

use crate::models::{BackgroundTask, TaskEvent, TaskKind, TaskStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::watch;

#[cfg(any(test, feature = "test-support"))]
const LOG_TAIL_MAX: usize = 64 * 1024;

/// Finished (non-running) tasks kept before the oldest are evicted.
pub const MAX_FINISHED_TASKS: usize = 200;

type TaskListener = Arc<dyn Fn(&TaskEvent) + Send + Sync>;
type TaskListeners = Vec<Option<TaskListener>>;
type TaskMap = HashMap<String, BackgroundTask>;

fn now_ms() -> i64 {
    bruhs_models::EpochMs::now().as_millis()
}

/// Evict the oldest finished tasks beyond [`MAX_FINISHED_TASKS`]; returns
/// the evicted ids. Running tasks are never evicted.
fn evict_finished(tasks: &mut TaskMap) -> Vec<String> {
    let mut finished: Vec<(i64, i64, String)> = tasks
        .values()
        .filter(|t| t.status != TaskStatus::Running)
        .map(|t| {
            (
                t.ended_at.unwrap_or(t.started_at),
                t.started_at,
                t.id.clone(),
            )
        })
        .collect();
    let excess = finished.len().saturating_sub(MAX_FINISHED_TASKS);
    if excess == 0 {
        return Vec::new();
    }
    finished.sort();
    finished
        .into_iter()
        .take(excess)
        .map(|(_, _, id)| {
            tasks.remove(&id);
            id
        })
        .collect()
}

/// Fields [`BackgroundTaskRegistry::update`] sets; `None` leaves a field as
/// it is.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TaskUpdate {
    pub status: Option<TaskStatus>,
    pub ended_at: Option<i64>,
    pub exit_code: Option<i32>,
}

#[cfg(any(test, feature = "test-support"))]
impl TaskUpdate {
    /// The task exited with `exit_code` at `ended_at`.
    pub fn exited(ended_at: i64, exit_code: i32) -> Self {
        Self {
            status: Some(TaskStatus::Exited),
            ended_at: Some(ended_at),
            exit_code: Some(exit_code),
        }
    }
}

/// Why [`BackgroundTaskRegistry::remove`] kept a task.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RemoveTaskError {
    #[error("no background task {0}")]
    NotFound(String),
    /// Running tasks are removed only once they finish.
    #[error("background task {0} is still running")]
    StillRunning(String),
}

pub struct BackgroundTaskRegistry {
    tasks: Mutex<TaskMap>,
    listeners: Mutex<TaskListeners>,
    /// Bumped after every change, for async consumers that only need to
    /// know that something changed (see [`Self::watch_changes`]).
    changes: watch::Sender<u64>,
}

impl BackgroundTaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
            listeners: Mutex::new(Vec::new()),
            changes: watch::Sender::new(0),
        }
    }

    /// A receiver that is marked changed after every change to the tasks.
    /// Unlike [`Self::subscribe`] it needs no unsubscribe: dropping the
    /// receiver is enough.
    pub fn watch_changes(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Deliver `events` to a snapshot of the listeners, with no lock held.
    fn emit_all(&self, events: &[TaskEvent]) {
        if events.is_empty() {
            return;
        }
        self.changes
            .send_modify(|version| *version = version.wrapping_add(1));
        let listeners: Vec<TaskListener> =
            self.lock_listeners().iter().flatten().cloned().collect();
        for event in events {
            for listener in &listeners {
                listener(event);
            }
        }
    }

    fn emit(&self, event: TaskEvent) {
        self.emit_all(&[event]);
    }

    fn lock_tasks(&self) -> MutexGuard<'_, TaskMap> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_listeners(&self) -> MutexGuard<'_, TaskListeners> {
        self.listeners
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn register(
        &self,
        thread_id: impl Into<String>,
        kind: TaskKind,
        label: impl Into<String>,
        meta: serde_json::Value,
    ) -> BackgroundTask {
        let task = BackgroundTask {
            id: uuid::Uuid::new_v4().to_string(),
            thread_id: thread_id.into(),
            parent_task_id: None,
            kind,
            label: label.into(),
            status: TaskStatus::Running,
            started_at: now_ms(),
            ended_at: None,
            exit_code: None,
            meta,
            log_tail: String::new(),
        };
        {
            let mut tasks = self.lock_tasks();
            tasks.insert(task.id.clone(), task.clone());
        }
        self.emit(TaskEvent::Created(task.clone()));
        task
    }

    /// Test fixture: set status / end time / exit code directly. Production
    /// code finishes tasks through `mark_stream_ended` / `mark_stream_killed`.
    #[cfg(any(test, feature = "test-support"))]
    pub fn update(&self, id: &str, update: TaskUpdate) {
        let events = {
            let mut tasks = self.lock_tasks();
            let Some(task) = tasks.get_mut(id) else {
                return;
            };
            if let Some(status) = update.status {
                task.status = status;
            }
            if update.ended_at.is_some() {
                task.ended_at = update.ended_at;
            }
            if update.exit_code.is_some() {
                task.exit_code = update.exit_code;
            }
            let mut events = vec![TaskEvent::Updated(task.clone())];
            events.extend(
                evict_finished(&mut tasks)
                    .into_iter()
                    .map(TaskEvent::Removed),
            );
            drop(tasks);
            events
        };
        self.emit_all(&events);
    }

    /// Test fixture: append to a task's log tail (capped at 64 KiB).
    #[cfg(any(test, feature = "test-support"))]
    pub fn append_log(&self, id: &str, chunk: &str) {
        let updated = {
            let mut tasks = self.lock_tasks();
            let Some(task) = tasks.get_mut(id) else {
                return;
            };
            task.log_tail.push_str(chunk);
            // If over cap, trim from the front to keep the tail
            if task.log_tail.len() > LOG_TAIL_MAX {
                let excess = task.log_tail.len() - LOG_TAIL_MAX;
                // Advance past a char boundary
                let trim_at = task
                    .log_tail
                    .char_indices()
                    .map(|(i, _)| i)
                    .find(|&i| i >= excess)
                    .unwrap_or(excess);
                task.log_tail = task.log_tail[trim_at..].to_string();
            }
            let updated = task.clone();
            drop(tasks);
            updated
        };
        self.emit(TaskEvent::Updated(updated));
    }

    /// Remove a finished task.
    ///
    /// # Errors
    ///
    /// [`RemoveTaskError::NotFound`] for an unknown id and
    /// [`RemoveTaskError::StillRunning`] for a running task (which is kept).
    pub fn remove(&self, id: &str) -> Result<(), RemoveTaskError> {
        {
            let mut tasks = self.lock_tasks();
            match tasks.get(id) {
                None => return Err(RemoveTaskError::NotFound(id.to_string())),
                Some(task) if task.status == TaskStatus::Running => {
                    return Err(RemoveTaskError::StillRunning(id.to_string()))
                }
                Some(_) => tasks.remove(id),
            };
        }
        self.emit(TaskEvent::Removed(id.to_string()));
        Ok(())
    }

    /// Move every running task `selected` picks to `status`, then evict the
    /// oldest finished tasks beyond the cap.
    fn finish_where(&self, selected: impl Fn(&BackgroundTask) -> bool, status: TaskStatus) {
        let events: Vec<TaskEvent> = {
            let mut tasks = self.lock_tasks();
            let ended_at = now_ms();
            let mut events: Vec<TaskEvent> = tasks
                .values_mut()
                .filter(|t| t.status == TaskStatus::Running && selected(t))
                .map(|t| {
                    t.status = status.clone();
                    t.ended_at = Some(ended_at);
                    TaskEvent::Updated(t.clone())
                })
                .collect();
            events.extend(
                evict_finished(&mut tasks)
                    .into_iter()
                    .map(TaskEvent::Removed),
            );
            drop(tasks);
            events
        };
        self.emit_all(&events);
    }

    /// Every running task of `thread_id` exited with its turn.
    pub fn mark_stream_ended(&self, thread_id: &str) {
        self.finish_where(|t| t.thread_id == thread_id, TaskStatus::Exited);
    }

    /// Every running task of `thread_id` was killed with its turn.
    pub fn mark_stream_killed(&self, thread_id: &str) {
        self.finish_where(|t| t.thread_id == thread_id, TaskStatus::Killed);
    }

    /// The running tasks among `ids` exited with the turn that started them.
    pub fn mark_tasks_ended(&self, ids: &[String]) {
        self.finish_where(|t| ids.contains(&t.id), TaskStatus::Exited);
    }

    /// The running tasks among `ids` were killed with the turn that started
    /// them.
    pub fn mark_tasks_killed(&self, ids: &[String]) {
        self.finish_where(|t| ids.contains(&t.id), TaskStatus::Killed);
    }

    pub fn get_all(&self) -> Vec<BackgroundTask> {
        let tasks = self.lock_tasks();
        tasks.values().cloned().collect()
    }

    pub fn get_by_thread(&self, thread_id: &str) -> Vec<BackgroundTask> {
        let tasks = self.lock_tasks();
        tasks
            .values()
            .filter(|t| t.thread_id == thread_id)
            .cloned()
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<BackgroundTask> {
        let tasks = self.lock_tasks();
        tasks.get(id).cloned()
    }

    /// Subscribe a listener; returns its index for use with `unsubscribe`.
    pub fn subscribe(&self, callback: impl Fn(&TaskEvent) + Send + Sync + 'static) -> usize {
        let mut listeners = self.lock_listeners();
        let index = listeners.len();
        listeners.push(Some(Arc::new(callback)));
        index
    }

    /// Unsubscribe by the index returned from `subscribe`.
    pub fn unsubscribe(&self, index: usize) {
        let mut listeners = self.lock_listeners();
        if let Some(slot) = listeners.get_mut(index) {
            *slot = None;
        }
    }
}

impl Default for BackgroundTaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn make_registry() -> BackgroundTaskRegistry {
        BackgroundTaskRegistry::new()
    }

    #[test]
    fn test_register() {
        let registry = make_registry();
        let events: Arc<Mutex<Vec<TaskEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        registry.subscribe(move |e| {
            events_clone.lock().unwrap().push(e.clone());
        });

        let task = registry.register("thread-1", TaskKind::Shell, "echo hello", json!({}));

        assert!(!task.id.is_empty());
        assert_eq!(task.thread_id, "thread-1");
        assert_eq!(task.label, "echo hello");
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.ended_at.is_none());
        assert!(task.exit_code.is_none());

        let stored = registry.get(&task.id).expect("task should be stored");
        assert_eq!(stored.id, task.id);

        let evts = events.lock().unwrap();
        assert_eq!(evts.len(), 1);
        match &evts[0] {
            TaskEvent::Created(t) => assert_eq!(t.id, task.id),
            _ => panic!("expected Created event"),
        }
    }

    #[test]
    fn test_mark_stream_ended() {
        let registry = make_registry();
        let events: Arc<Mutex<Vec<TaskEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        registry.subscribe(move |e| {
            events_clone.lock().unwrap().push(e.clone());
        });

        let t1 = registry.register("thread-A", TaskKind::Shell, "task 1", json!({}));
        let t2 = registry.register("thread-A", TaskKind::Shell, "task 2", json!({}));
        // Register a task on a different thread — should not be affected
        let t3 = registry.register("thread-B", TaskKind::Shell, "task 3", json!({}));

        registry.mark_stream_ended("thread-A");

        let stored1 = registry.get(&t1.id).unwrap();
        let stored2 = registry.get(&t2.id).unwrap();
        let stored3 = registry.get(&t3.id).unwrap();

        assert_eq!(stored1.status, TaskStatus::Exited);
        assert!(stored1.ended_at.is_some());
        assert_eq!(stored2.status, TaskStatus::Exited);
        assert!(stored2.ended_at.is_some());
        // Thread-B task must remain Running
        assert_eq!(stored3.status, TaskStatus::Running);

        // 2 Created + 2 Updated events for thread-A tasks, 1 Created for thread-B
        let updated_count = events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, TaskEvent::Updated(_)))
            .count();
        assert_eq!(updated_count, 2);
    }

    #[test]
    fn test_append_log() {
        let registry = make_registry();
        let task = registry.register("thread-1", TaskKind::Shell, "log test", json!({}));

        // Append something well under the limit
        registry.append_log(&task.id, "hello\n");
        let stored = registry.get(&task.id).unwrap();
        assert_eq!(stored.log_tail, "hello\n");

        // Append enough to exceed LOG_TAIL_MAX (64 KiB)
        let big_chunk = "x".repeat(LOG_TAIL_MAX);
        registry.append_log(&task.id, &big_chunk);
        let stored = registry.get(&task.id).unwrap();
        assert!(
            stored.log_tail.len() <= LOG_TAIL_MAX,
            "log_tail should be capped at LOG_TAIL_MAX, got {}",
            stored.log_tail.len()
        );
        // The tail should end with the big chunk's suffix
        assert!(stored.log_tail.ends_with('x'));
    }

    #[test]
    fn test_remove() {
        let registry = make_registry();
        let events: Arc<Mutex<Vec<TaskEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_clone = events.clone();
        registry.subscribe(move |e| {
            events_clone.lock().unwrap().push(e.clone());
        });

        let task = registry.register("thread-1", TaskKind::Shell, "remove me", json!({}));

        let mut changes = registry.watch_changes();
        changes.mark_unchanged();

        // Can't remove a running task
        assert_eq!(
            registry.remove(&task.id),
            Err(RemoveTaskError::StillRunning(task.id.clone()))
        );
        assert!(registry.get(&task.id).is_some(), "task should still exist");
        assert!(
            !changes.has_changed().unwrap(),
            "a refused removal changes nothing"
        );

        // Transition to Exited then remove
        registry.update(&task.id, TaskUpdate::exited(now_ms(), 0));
        assert_eq!(registry.remove(&task.id), Ok(()));
        assert!(registry.get(&task.id).is_none(), "task should be gone");
        assert!(changes.has_changed().unwrap(), "changes are announced");
        assert_eq!(
            registry.remove(&task.id),
            Err(RemoveTaskError::NotFound(task.id.clone()))
        );

        let evts = events.lock().unwrap();
        let removed_events: Vec<_> = evts
            .iter()
            .filter(|e| matches!(e, TaskEvent::Removed(_)))
            .collect();
        assert_eq!(removed_events.len(), 1);
        match &removed_events[0] {
            TaskEvent::Removed(id) => assert_eq!(id, &task.id),
            _ => unreachable!(),
        }
        drop(evts);
    }

    #[test]
    fn test_unsubscribe() {
        let registry = make_registry();
        let count: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
        let count_clone = count.clone();

        let idx = registry.subscribe(move |_| {
            *count_clone.lock().unwrap() += 1;
        });

        registry.register("thread-1", TaskKind::Shell, "task", json!({}));
        assert_eq!(*count.lock().unwrap(), 1);

        registry.unsubscribe(idx);
        registry.register("thread-1", TaskKind::Shell, "task2", json!({}));
        // Count must not increase after unsubscribe
        assert_eq!(*count.lock().unwrap(), 1);
    }

    #[test]
    fn test_registry_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<BackgroundTaskRegistry>();
        assert_send_sync::<Arc<BackgroundTaskRegistry>>();
    }

    /// Listeners used to run under the listener mutex, so one that touched
    /// the subscriptions deadlocked the registry.
    #[test]
    fn listeners_may_call_back_into_the_registry() {
        let registry = Arc::new(make_registry());
        let inner = Arc::clone(&registry);
        registry.subscribe(move |event| {
            if matches!(event, TaskEvent::Created(_)) {
                let index = inner.subscribe(|_| {});
                inner.unsubscribe(index);
                let _ = inner.get_all();
            }
        });
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = Arc::clone(&registry);
        std::thread::spawn(move || {
            worker.register("thread-1", TaskKind::Shell, "reentrant", json!({}));
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("register deadlocked in a re-entrant listener");
    }

    #[test]
    fn finished_tasks_beyond_the_cap_are_evicted_oldest_first() {
        let registry = make_registry();
        let removed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let removed_clone = removed.clone();
        registry.subscribe(move |e| {
            if let TaskEvent::Removed(id) = e {
                removed_clone.lock().unwrap().push(id.clone());
            }
        });

        let running = registry.register("live", TaskKind::Shell, "still running", json!({}));
        let mut finished = Vec::new();
        for i in 0..(MAX_FINISHED_TASKS + 3) {
            let task = registry.register("t", TaskKind::Shell, format!("task {i}"), json!({}));
            registry.update(&task.id, TaskUpdate::exited(i64::try_from(i).unwrap(), 0));
            finished.push(task.id);
        }

        assert_eq!(registry.get_all().len(), MAX_FINISHED_TASKS + 1);
        assert!(registry.get(&running.id).is_some(), "running tasks stay");
        assert_eq!(*removed.lock().unwrap(), finished[..3].to_vec());
        for id in &finished[..3] {
            assert!(registry.get(id).is_none());
        }

        // Finishing a whole thread evicts too.
        for _ in 0..5 {
            registry.register("bulk", TaskKind::Shell, "bulk", json!({}));
        }
        registry.mark_stream_ended("bulk");
        let finished_count = registry
            .get_all()
            .iter()
            .filter(|t| t.status != TaskStatus::Running)
            .count();
        assert_eq!(finished_count, MAX_FINISHED_TASKS);
    }
}
