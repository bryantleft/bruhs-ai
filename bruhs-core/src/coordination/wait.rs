//! A parent waiting on a child yields its worker slot. The host retains the
//! dependency lifecycle so nested delegation also progresses on a one-core host.
use super::*;
use tokio::sync::OwnedSemaphorePermit;

pub(super) struct TaskSlot {
    permit: Option<OwnedSemaphorePermit>,
    wait_id: Option<String>,
}

pub(super) struct RunSlot {
    runtime: CoordinationRuntime,
    id: ThreadId,
}

impl RunSlot {
    pub(super) fn hold(
        runtime: CoordinationRuntime,
        id: ThreadId,
        permit: OwnedSemaphorePermit,
    ) -> Result<Self, CoordinationError> {
        {
            let mut slots = runtime.0.permits.lock().map_err(storage)?;
            if slots.contains_key(&id) {
                return Err(conflict("This task already owns a worker slot."));
            }
            slots.insert(
                id.clone(),
                TaskSlot {
                    permit: Some(permit),
                    wait_id: None,
                },
            );
        }
        Ok(Self { runtime, id })
    }
}

impl Drop for RunSlot {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.runtime.0.permits.lock() {
            slots.remove(&self.id);
        }
    }
}

struct WaitGuard {
    runtime: CoordinationRuntime,
    parent: ThreadId,
    wait_id: String,
    cancel: watch::Sender<bool>,
    armed: bool,
}

impl Drop for WaitGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let owns_wait = self.runtime.0.permits.lock().ok().is_some_and(|slots| {
            slots
                .get(&self.parent)
                .is_some_and(|slot| slot.wait_id.as_deref() == Some(&self.wait_id))
        });
        if !owns_wait {
            return;
        }
        // Stop the producer synchronously before it could resume without a slot.
        self.cancel.send_replace(true);
        let runtime = self.runtime.clone();
        let parent = self.parent.clone();
        tokio::spawn(async move {
            if let Err(error) = runtime.cancel(&parent).await {
                tracing::error!(%error, task=%parent, "could not cancel disconnected delegated wait");
                return;
            }
            let mut data = runtime.0.data.lock().await;
            if let Some(task) = data.tasks.get_mut(&parent) {
                task.error = Some("The delegated-result wait disconnected before its worker slot was restored. The task tree was stopped; review child results before resuming.".into());
            }
            let result = runtime.persist(&mut data).await;
            drop(data);
            if let Err(error) = result {
                tracing::error!(%error, task=%parent, "could not save disconnected wait failure");
            }
        });
    }
}

fn pending(task: &CoordinatedTask) -> bool {
    matches!(
        task.state,
        TaskState::Ready | TaskState::Running | TaskState::WaitingChild
    )
}

fn is_descendant(data: &Data, parent: &ThreadId, id: &ThreadId) -> bool {
    let mut cursor = id;
    let mut seen = std::collections::BTreeSet::new();
    while seen.insert(cursor) {
        let Some(task) = data.tasks.get(cursor) else {
            return false;
        };
        let Some(ancestor) = task.parent_id.as_ref() else {
            return false;
        };
        if ancestor == parent {
            return true;
        }
        cursor = ancestor;
    }
    false
}

impl CoordinationRuntime {
    /// Observe a descendant while yielding the parent's worker slot. The child
    /// observation budget matches the established capture/MCP request deadline;
    /// restoring capacity is intentionally not time-limited. HTTP callers must
    /// not impose an overall timeout: disconnecting stops this task tree.
    pub async fn wait_for_child(
        &self,
        parent_id: &ThreadId,
        id: &ThreadId,
    ) -> Result<CoordinatedTask, CoordinationError> {
        self.wait_for_child_with_budget(parent_id, id, crate::capture_client::REQUEST_TIMEOUT)
            .await
    }

    #[expect(
        clippy::significant_drop_tightening,
        reason = "TaskSlot borrows do not own permits; the slot map stays locked while capacity is yielded or restored"
    )]
    async fn wait_for_child_with_budget(
        &self,
        parent_id: &ThreadId,
        id: &ThreadId,
        budget: std::time::Duration,
    ) -> Result<CoordinatedTask, CoordinationError> {
        let mut changes = self.changes();
        let cancel = self
            .0
            .cancels
            .lock()
            .await
            .get(parent_id)
            .cloned()
            .ok_or_else(|| conflict("Only a running host-owned task can wait for a child."))?;
        let mut cancelled = cancel.subscribe();
        let wait_id = uuid::Uuid::new_v4().to_string();
        let mut guard = WaitGuard {
            runtime: self.clone(),
            parent: parent_id.clone(),
            wait_id: wait_id.clone(),
            cancel,
            armed: false,
        };
        {
            let mut data = self.0.data.lock().await;
            let parent = data
                .tasks
                .get(parent_id)
                .ok_or_else(|| not_found(parent_id))?;
            let child = data.tasks.get(id).ok_or_else(|| not_found(id))?;
            if parent.project_path != child.project_path || !is_descendant(&data, parent_id, id) {
                return Err(invalid(
                    "Only descendants in the same project can be awaited.",
                ));
            }
            if !pending(child) {
                return Ok(child.clone());
            }
            if parent.state != TaskState::Running || *cancelled.borrow() {
                return Err(conflict(
                    "The parent must be running and cannot already be waiting for a child.",
                ));
            }
            {
                let mut slots = self.0.permits.lock().map_err(storage)?;
                let slot = slots
                    .get_mut(parent_id)
                    .ok_or_else(|| conflict("The parent has no running worker slot."))?;
                if slot.wait_id.is_some() || slot.permit.is_none() {
                    return Err(conflict("This parent is already waiting for a child."));
                }
                slot.wait_id = Some(wait_id.clone());
                drop(slot.permit.take());
                guard.armed = true;
            }
            data.tasks
                .get_mut(parent_id)
                .ok_or_else(|| not_found(parent_id))?
                .state = TaskState::WaitingChild;
            self.persist(&mut data).await?;
            drop(data);
        }
        let deadline = tokio::time::sleep(budget);
        tokio::pin!(deadline);
        loop {
            if *cancelled.borrow() {
                return Err(conflict("The parent task was stopped while waiting."));
            }
            if !pending(&self.task(id).await?) {
                break;
            }
            tokio::select! {
                _ = &mut deadline => break,
                _ = cancelled.changed() => return Err(conflict("The parent task was stopped while waiting.")),
                result = changes.changed() => if result.is_err() { return Err(conflict("Task updates closed while waiting.")); },
            }
        }
        let permit = tokio::select! {
            _ = cancelled.changed() => return Err(conflict("The parent task was stopped while restoring its worker slot.")),
            permit = self.0.slots.clone().acquire_owned() => permit.map_err(storage)?,
        };
        {
            let mut slots = self.0.permits.lock().map_err(storage)?;
            let slot = slots.get_mut(parent_id).ok_or_else(|| {
                conflict("The parent task ended before its worker slot was restored.")
            })?;
            if slot.wait_id.as_deref() != Some(&wait_id) || *cancelled.borrow() {
                return Err(conflict("This delegated wait is no longer current."));
            }
            slot.permit = Some(permit);
        }
        {
            let mut data = self.0.data.lock().await;
            let parent = data
                .tasks
                .get_mut(parent_id)
                .ok_or_else(|| not_found(parent_id))?;
            if parent.state != TaskState::WaitingChild {
                return Err(conflict("The parent task changed while waiting."));
            }
            parent.state = TaskState::Running;
            self.persist(&mut data).await?;
            drop(data);
        }
        {
            let mut slots = self.0.permits.lock().map_err(storage)?;
            let slot = slots.get_mut(parent_id).ok_or_else(|| {
                conflict("The parent task ended while restoring its worker slot.")
            })?;
            slot.wait_id = None;
        }
        guard.armed = false;
        drop(guard);
        self.task(id).await
    }
}
