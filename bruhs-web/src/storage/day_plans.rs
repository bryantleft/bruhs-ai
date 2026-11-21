//! Read, validate and commit a day plan and its Undo receipt atomically.

use bruhs_models::{
    prepare_day_plan_undo, prepare_day_plan_with_availability, CalendarEvent, DayPlanChangeError,
    DayPlanOwnership, DayPlanProposal, DayPlanWindow, WorkItem,
};
use idb::TransactionMode;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsValue;

use super::metadata::{self, LocalTransaction, Metadata, SaveError};
use super::{from_js, open_db, to_js, STORE_EVENTS, STORE_METADATA, STORE_WORK_ITEMS};

const KEY: &str = "bruhs.last-day-plan";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub ownership: DayPlanOwnership,
}

pub struct Commit {
    pub tasks: Vec<(WorkItem, WorkItem)>,
    pub events: Vec<(Option<CalendarEvent>, Option<CalendarEvent>)>,
    pub receipt: Option<Receipt>,
}

pub struct Snapshot {
    pub tasks: Vec<WorkItem>,
    pub events: Vec<CalendarEvent>,
    pub receipt: Option<Receipt>,
}

#[derive(Debug)]
pub enum Error {
    Storage(SaveError),
    Plan(DayPlanChangeError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(f),
            Self::Plan(error) => error.fmt(f),
        }
    }
}

impl From<idb::Error> for Error {
    fn from(error: idb::Error) -> Self {
        Self::Storage(error.into())
    }
}

impl From<SaveError> for Error {
    fn from(error: SaveError) -> Self {
        Self::Storage(error)
    }
}

impl From<DayPlanChangeError> for Error {
    fn from(error: DayPlanChangeError) -> Self {
        Self::Plan(error)
    }
}

pub async fn load() -> Result<Option<Receipt>, SaveError> {
    Ok(metadata::load_or_migrate::<Option<Receipt>>(KEY)
        .await?
        .value)
}

async fn read_records(tx: &LocalTransaction) -> Result<(Vec<WorkItem>, Vec<CalendarEvent>), Error> {
    let tasks = tx
        .object_store(STORE_WORK_ITEMS)?
        .get_all(None, None)?
        .await?
        .into_iter()
        .map(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))
        .collect::<Result<_, _>>()?;
    let events = tx
        .object_store(STORE_EVENTS)?
        .get_all(None, None)?
        .await?
        .into_iter()
        .map(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))
        .collect::<Result<_, _>>()?;
    Ok((tasks, events))
}

async fn persist(tx: &LocalTransaction, commit: &Commit, revision: u64) -> Result<(), Error> {
    // Convert every value before issuing the first write.
    let tasks = commit
        .tasks
        .iter()
        .map(|(_, after)| to_js(after))
        .collect::<Result<Vec<_>, _>>()?;
    let events = commit
        .events
        .iter()
        .map(|(_, after)| after.as_ref().map(to_js).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    let receipt = metadata::encode(&Metadata {
        key: KEY.to_owned(),
        revision,
        value: &commit.receipt,
    })?;
    for task in tasks {
        tx.object_store(STORE_WORK_ITEMS)?.put(&task, None)?.await?;
    }
    for ((before, _), value) in commit.events.iter().zip(events) {
        let store = tx.object_store(STORE_EVENTS)?;
        if let Some(value) = value {
            store.put(&value, None)?.await?;
        } else if let Some(before) = before {
            store.delete(JsValue::from_str(&before.id))?.await?;
        }
    }
    tx.object_store(STORE_METADATA)?
        .put(&receipt, None)?
        .await?;
    Ok(())
}

async fn finish<T>(tx: LocalTransaction, result: Result<T, Error>) -> Result<T, Error> {
    match result {
        Ok(commit) => metadata::finish(tx, Ok(commit)).await.map_err(Into::into),
        Err(error) => {
            let _ = metadata::finish::<()>(tx, Err(SaveError::Rejected("Day plan was not saved.")))
                .await;
            Err(error)
        }
    }
}

pub async fn snapshot() -> Result<Snapshot, Error> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_WORK_ITEMS, STORE_EVENTS, STORE_METADATA],
        TransactionMode::ReadOnly,
    )?);
    let result = async {
        let (tasks, events) = read_records(&tx).await?;
        let receipt: Metadata<Option<Receipt>> = metadata::read_in(&tx, KEY).await?;
        Ok(Snapshot {
            tasks,
            events,
            receipt: receipt.value,
        })
    }
    .await;
    finish(tx, result).await
}

pub async fn apply(
    proposal: &DayPlanProposal,
    reviewed_window: DayPlanWindow,
) -> Result<Commit, Error> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_WORK_ITEMS, STORE_EVENTS, STORE_METADATA],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        let (tasks, events) = read_records(&tx).await?;
        let now = crate::clock::now_ms();
        let window =
            crate::clock::day_plan_window(bruhs_models::EpochMs::saturating_from_i64(now).as_f64());
        if window != reviewed_window {
            return Err(DayPlanChangeError::InvalidWindow.into());
        }
        let preferences = super::scheduling::read_in(&tx).await?.value;
        let prepared = prepare_day_plan_with_availability(
            proposal,
            &tasks,
            &events,
            window,
            now,
            &preferences.availability,
            crate::clock::offset_minutes_east,
        )?;
        let current: Metadata<Option<Receipt>> = metadata::read_in(&tx, KEY).await?;
        let changed_tasks = prepared
            .changed_tasks
            .into_iter()
            .map(|after| {
                let before = tasks
                    .iter()
                    .find(|task| task.id == after.id)
                    .cloned()
                    .ok_or(SaveError::Corrupt)?;
                Ok((before, after))
            })
            .collect::<Result<Vec<_>, SaveError>>()?;
        let commit = Commit {
            tasks: changed_tasks,
            events: prepared
                .added_events
                .into_iter()
                .map(|event| (None, Some(event)))
                .collect(),
            receipt: Some(Receipt {
                id: uuid::Uuid::new_v4().to_string(),
                ownership: prepared.ownership,
            }),
        };
        persist(
            &tx,
            &commit,
            metadata::next_revision(current.revision, current.revision)?,
        )
        .await?;
        Ok(commit)
    }
    .await;
    finish(tx, result).await
}

pub async fn undo(expected: &Receipt) -> Result<Commit, Error> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_WORK_ITEMS, STORE_EVENTS, STORE_METADATA],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        let current: Metadata<Option<Receipt>> = metadata::read_in(&tx, KEY).await?;
        let receipt = current
            .value
            .as_ref()
            .filter(|record| record.id == expected.id)
            .ok_or(SaveError::Conflict)?;
        let (tasks, events) = read_records(&tx).await?;
        let prepared =
            prepare_day_plan_undo(&receipt.ownership, &tasks, &events, crate::clock::now_ms())?;
        let changed_tasks = prepared
            .changed_tasks
            .into_iter()
            .map(|after| {
                let before = tasks
                    .iter()
                    .find(|task| task.id == after.id)
                    .cloned()
                    .ok_or(SaveError::Corrupt)?;
                Ok((before, after))
            })
            .collect::<Result<Vec<_>, SaveError>>()?;
        let removed = prepared
            .removed_event_ids
            .into_iter()
            .map(|id| {
                events
                    .iter()
                    .find(|event| event.id == id)
                    .cloned()
                    .map(|event| (Some(event), None))
                    .ok_or(SaveError::Corrupt)
            })
            .collect::<Result<_, _>>()?;
        let commit = Commit {
            tasks: changed_tasks,
            events: removed,
            receipt: None,
        };
        persist(
            &tx,
            &commit,
            metadata::next_revision(current.revision, current.revision)?,
        )
        .await?;
        Ok(commit)
    }
    .await;
    finish(tx, result).await
}
