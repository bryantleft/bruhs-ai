//! One authoritative scheduling policy, serialized with task and calendar writes.

use super::metadata::{self, LocalTransaction, Metadata, SaveError};
use super::{from_js, open_db, to_js, STORE_EVENTS, STORE_METADATA, STORE_WORK_ITEMS};
use bruhs_models::{
    CalendarEvent, ConflictResolution, EventSource, SchedulingPreferences, WorkItem,
};
use idb::{Transaction, TransactionMode};
use wasm_bindgen::JsValue;

pub const KEY: &str = "bruhs.scheduling";
pub type Policy = Metadata<SchedulingPreferences>;

pub async fn load() -> Result<Policy, SaveError> {
    metadata::load_or_migrate(KEY).await
}
pub async fn read_in(tx: &Transaction) -> Result<Policy, SaveError> {
    metadata::read_in(tx, KEY).await
}
pub async fn save(expected: u64, value: SchedulingPreferences) -> Result<Policy, SaveError> {
    let revision = metadata::save(KEY, expected, &value).await?;
    Ok(Metadata {
        key: KEY.into(),
        revision,
        value,
    })
}

/// Read-only JSON view of the authoritative record, including corrupt records.
/// This is a structured-record inspection, not an original-byte export.
pub async fn inspect() -> Result<Option<String>, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(&[STORE_METADATA], TransactionMode::ReadOnly)?);
    let result = async {
        tx.object_store(STORE_METADATA)?
            .get(JsValue::from_str(KEY))?
            .await?
            .map(|raw| {
                js_sys::JSON::stringify(&raw)
                    .ok()
                    .and_then(|text| text.as_string())
                    .ok_or(SaveError::Corrupt)
            })
            .transpose()
    }
    .await;
    metadata::finish(tx, result).await
}

pub struct Resolution {
    pub policy: Policy,
    pub events: Vec<(Option<CalendarEvent>, Option<CalendarEvent>)>,
}

pub async fn resolve_conflict(
    expected: u64,
    id: &str,
    resolution: ConflictResolution,
    displayed: &[CalendarEvent],
) -> Result<Resolution, SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(
        db.transaction(&[STORE_METADATA, STORE_EVENTS], TransactionMode::ReadWrite)?,
    );
    let result = async {
        let mut policy = read_in(&tx).await?;
        let revision = metadata::next_revision(policy.revision, expected)?;
        let conflict = policy
            .value
            .conflicts
            .iter()
            .find(|conflict| conflict.id == id && conflict.resolution.is_none())
            .ok_or(SaveError::Rejected(
                "This conflict changed. Reload saved preferences before resolving it.",
            ))?;
        let store = tx.object_store(STORE_EVENTS)?;
        let original: CalendarEvent = store
            .get(JsValue::from_str(&conflict.event_id))?
            .await?
            .ok_or(SaveError::Conflict)
            .and_then(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))?;
        if displayed.iter().find(|event| event.id == original.id) != Some(&original) {
            return Err(SaveError::Conflict);
        }
        let mut updated = vec![original.clone()];
        if !policy
            .value
            .resolve_conflict(id, resolution, &mut updated, crate::clock::now_ms())
        {
            return Err(SaveError::Conflict);
        }
        let events = updated
            .into_iter()
            .filter_map(|event| {
                let before = (event.id == original.id).then(|| original.clone());
                (before.as_ref() != Some(&event)).then_some((before, Some(event)))
            })
            .collect::<Vec<_>>();
        policy.revision = revision;
        let encoded_policy = metadata::encode(&policy)?;
        let values = events
            .iter()
            .map(|(_, after)| Ok(to_js(after.as_ref().ok_or(SaveError::Corrupt)?)?))
            .collect::<Result<Vec<_>, SaveError>>()?;
        for value in values {
            store.put(&value, None)?.await?;
        }
        tx.object_store(STORE_METADATA)?
            .put(&encoded_policy, None)?
            .await?;
        Ok(Resolution { policy, events })
    }
    .await;
    metadata::finish(tx, result).await
}

/// Automatic placement reads the policy and occupied calendar inside its write
/// transaction. A stale displayed task/calendar is rejected, never overwritten.
pub async fn place_task(
    expected_task: &WorkItem,
    displayed_events: &[CalendarEvent],
) -> Result<(WorkItem, WorkItem, CalendarEvent), SaveError> {
    let db = open_db().await?;
    let tx = LocalTransaction::new(db.transaction(
        &[STORE_METADATA, STORE_WORK_ITEMS, STORE_EVENTS],
        TransactionMode::ReadWrite,
    )?);
    let result = async {
        let policy = read_in(&tx).await?;
        let tasks = tx.object_store(STORE_WORK_ITEMS)?;
        let original: WorkItem = tasks
            .get(JsValue::from_str(&expected_task.id))?
            .await?
            .ok_or(SaveError::Conflict)
            .and_then(|raw| from_js(raw).map_err(|_| SaveError::Corrupt))?;
        if &original != expected_task
            || original.status.is_completed()
            || !original.planning.time_block_event_ids.is_empty()
        {
            return Err(SaveError::Conflict);
        }
        let events = tx.object_store(STORE_EVENTS)?;
        let occupied = events
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js::<CalendarEvent>(raw).map_err(|_| SaveError::Corrupt))
            .collect::<Result<Vec<_>, _>>()?;
        if occupied.len() != displayed_events.len()
            || occupied.iter().any(|event| {
                displayed_events
                    .iter()
                    .find(|displayed| displayed.id == event.id)
                    != Some(event)
            })
        {
            return Err(SaveError::Conflict);
        }
        let now = crate::clock::now_ms();
        let (start, end) = policy
            .value
            .first_available_slot_in_zone(
                &occupied,
                now,
                now.saturating_add(30 * 86_400_000),
                original.planning.estimate_minutes.unwrap_or(60).max(15),
                crate::clock::offset_minutes_east,
            )
            .ok_or(SaveError::Rejected(
                "No open slot matches your saved availability in the next 30 days.",
            ))?;
        let event = focus_block(&original, start, end, now);
        let mut task = original.clone();
        task.planning.time_block_event_ids.push(event.id.clone());
        task.updated_at = now;
        let encoded_task = to_js(&task)?;
        let encoded_event = to_js(&event)?;
        tasks.put(&encoded_task, None)?.await?;
        events.add(&encoded_event, None)?.await?;
        Ok((original, task, event))
    }
    .await;
    metadata::finish(tx, result).await
}

fn focus_block(task: &WorkItem, start_ms: i64, end_ms: i64, now_ms: i64) -> CalendarEvent {
    CalendarEvent {
        id: uuid::Uuid::new_v4().to_string(),
        title: format!("Focus: {}", task.title),
        start_at_ms: bruhs_models::EpochMs::saturating_from_i64(start_ms).as_f64(),
        end_at_ms: bruhs_models::EpochMs::saturating_from_i64(end_ms).as_f64(),
        all_day: false,
        color: Some("blueberry".into()),
        notes: Some(format!("Time block for task {}", task.id)),
        created_at: now_ms,
        updated_at: now_ms,
        source: EventSource::Local,
        google_event_id: None,
        calendar_id: None,
        attendees: Vec::new(),
        organizer: None,
        location: None,
        conferencing: None,
        recurring_event_id: None,
        html_link: None,
    }
}
