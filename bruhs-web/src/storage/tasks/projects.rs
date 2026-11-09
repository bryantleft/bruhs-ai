use super::*;
use crate::state::Project;

pub(super) struct ProjectGuard {
    added: BTreeSet<String>,
    deleted: BTreeSet<String>,
}

impl ProjectGuard {
    pub fn new(writes: &[Write<'_>]) -> Self {
        let mut added = BTreeSet::new();
        let mut deleted = BTreeSet::new();
        for write in writes {
            if write.kind == Kind::Project && write.draft.is_none() {
                deleted.insert(write.id.to_owned());
            }
            if let Some(Record::Task(task)) = write.draft {
                let previous = match write.expected {
                    Some(Record::Task(original)) => original.project_id.as_ref(),
                    _ => None,
                };
                if let Some(project) = &task.project_id {
                    if previous != Some(project) {
                        added.insert(project.clone());
                    }
                }
            }
        }
        Self { added, deleted }
    }

    pub fn stores(&self) -> impl Iterator<Item = &'static str> {
        (!self.added.is_empty() || !self.deleted.is_empty())
            .then_some(super::super::STORE_PROJECTS)
            .into_iter()
            .chain((!self.deleted.is_empty()).then_some(STORE_WORK_ITEMS))
    }

    /// Both sides hold the same transaction lock. A stale tab cannot link a
    /// new task after deletion, or delete a project while overlooking an
    /// external task that was not in its loaded sidebar snapshot.
    pub async fn validate(&self, tx: &LocalTransaction) -> Result<(), SaveError> {
        for id in &self.added {
            let raw = tx
                .object_store(super::super::STORE_PROJECTS)?
                .get(JsValue::from_str(id))?
                .await?
                .ok_or(SaveError::Conflict)?;
            let project: Project = from_js(raw).map_err(|_| SaveError::Corrupt)?;
            if project.id != *id {
                return Err(SaveError::Corrupt);
            }
        }
        if !self.deleted.is_empty() {
            #[derive(serde::Deserialize)]
            struct ScopedTask {
                project_id: Option<String>,
            }
            let tasks = tx
                .object_store(STORE_WORK_ITEMS)?
                .get_all(None, None)?
                .await?;
            for raw in tasks {
                let task: ScopedTask = from_js(raw).map_err(|_| SaveError::Corrupt)?;
                if task.project_id.is_some_and(|id| self.deleted.contains(&id)) {
                    return Err(SaveError::Conflict);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
