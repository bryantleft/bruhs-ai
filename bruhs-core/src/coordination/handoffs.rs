//! A prepared personal-task snapshot never starts a provider. Start records its
//! receipt in the same durable write as the run, before dispatching the worker.
use super::*;
use bruhs_models::workbench_handoff::{
    work_item_draft, WorkItemHandoff, WorkItemHandoffRequest, WorkItemHandoffSummary,
};

impl CoordinationRuntime {
    pub async fn work_item_handoffs(&self) -> Vec<WorkItemHandoffSummary> {
        let data = self.0.data.lock().await;
        let mut handoffs = data
            .work_item_handoffs
            .values()
            .map(|handoff| WorkItemHandoffSummary {
                id: handoff.id.clone(),
                item_id: handoff.request.item.id.clone(),
                title: handoff.request.item.title.clone(),
                created_at: handoff.created_at,
                task_id: handoff
                    .started
                    .as_ref()
                    .map(|started| started.task_id.clone()),
            })
            .collect::<Vec<_>>();
        drop(data);
        handoffs.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        handoffs
    }

    pub async fn prepare_work_item(
        &self,
        request: WorkItemHandoffRequest,
    ) -> Result<WorkItemHandoff, CoordinationError> {
        work_item_draft(&request).map_err(invalid)?;
        let mut data = self.0.data.lock().await;
        if let Some(existing) = data
            .work_item_handoffs
            .values()
            .find(|handoff| handoff.request == request)
        {
            return Ok(existing.clone());
        }
        let handoff = WorkItemHandoff {
            id: ThreadId::new_v4(),
            request,
            created_at: EpochMs::now(),
            started: None,
        };
        data.work_item_handoffs
            .insert(handoff.id.clone(), handoff.clone());
        self.persist(&mut data).await?;
        drop(data);
        Ok(handoff)
    }

    pub async fn work_item_handoff(
        &self,
        id: &ThreadId,
    ) -> Result<WorkItemHandoff, CoordinationError> {
        self.0
            .data
            .lock()
            .await
            .work_item_handoffs
            .get(id)
            .cloned()
            .ok_or_else(|| not_found(id))
    }

    pub async fn start_work_item(
        &self,
        id: &ThreadId,
        request: NewTask,
    ) -> Result<ThreadId, CoordinationError> {
        // Serialize the receipt check through worktree preparation and durable
        // creation. A response lost after commit cannot launch a second worker.
        let _start = self.0.handoff_starts.lock().await;
        let handoff = self.work_item_handoff(id).await?;
        if let Some(started) = handoff.started {
            if started.request != request {
                return Err(conflict("This handoff already started with different settings. Open its existing task before preparing other work."));
            }
            self.task(&started.task_id).await?;
            return Ok(started.task_id);
        }
        self.create_with_handoff(request, None, Relationship::Root, Some(handoff))
            .await
    }

    pub async fn forget_work_item_handoff(&self, id: &ThreadId) -> Result<(), CoordinationError> {
        let _start = self.0.handoff_starts.lock().await;
        let mut data = self.0.data.lock().await;
        if data.work_item_handoffs.remove(id).is_none() {
            return Err(not_found(id));
        }
        let result = self.persist(&mut data).await;
        drop(data);
        result
    }
}
