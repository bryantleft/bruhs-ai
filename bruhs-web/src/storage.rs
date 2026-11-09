//! IndexedDB-backed persistence for the WASM app.
//!
//! Task helpers use an ordered, original-checked writer with visible recovery.
//! Other legacy `persist_*` / `forget_*` helpers remain best-effort. Life and
//! automation use revision-checked metadata; workflow record and
//! checkpoint writes/rollback use one transaction in `workflow`.
//!
//! Schema migration — `DB_VERSION` is bumped each time a new object store is
//! added. Existing stores are retained, and connections close when their async
//! operation finishes so they do not hold future upgrades open.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use idb::{
    Database, DatabaseEvent, Event, Factory, KeyPath, ObjectStoreParams, Request, TransactionMode,
};
use serde::{Deserialize, Serialize};
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::JsValue;

pub mod meeting_generations;
pub mod meeting_notes;
pub mod metadata;
pub mod people;
pub mod relationships;
pub mod tasks;
pub mod transcripts;
pub mod workflow;

use crate::state::{
    AgentMessage, Calendar, CalendarEvent, CaptureRecord, InboxConversation, KnowledgeAttachment,
    KnowledgeDocument, Meeting, PersonProfile, Project, SourceClip, Thread, TranscriptSegment,
    WorkItem,
};

const DB_NAME: &str = "bruhs";
const DB_VERSION: u32 = 11;
const STORE_MEETING_GENERATIONS: &str = "meeting_generations";
const STORE_METADATA: &str = "metadata";
const STORE_THREADS: &str = "threads";
const STORE_MESSAGES: &str = "messages";
const STORE_WORK_ITEMS: &str = "work_items";
const STORE_PROJECTS: &str = "projects";
const STORE_EVENTS: &str = "events";
const STORE_CALENDARS: &str = "calendars";
const STORE_MEETINGS: &str = "meetings";
const STORE_MEETING_NOTES: &str = "meeting_notes";
const STORE_SEGMENTS: &str = "transcript_segments";
const STORE_PEOPLE: &str = "people";
const STORE_CAPTURES: &str = "captures";
const STORE_KNOWLEDGE_DOCUMENTS: &str = "knowledge_documents";
const STORE_SOURCE_CLIPS: &str = "source_clips";
const STORE_KNOWLEDGE_ATTACHMENTS: &str = "knowledge_attachments";
const STORE_INBOX_CONVERSATIONS: &str = "inbox_conversations";
const STORE_RELATIONSHIPS: &str = "relationships";

pub async fn load_meeting_notes() -> Result<Vec<bruhs_models::MeetingNote>, idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_MEETING_NOTES], TransactionMode::ReadOnly)?;
    let raw = tx
        .object_store(STORE_MEETING_NOTES)?
        .get_all(None, None)?
        .await?;
    tx.await?;
    let mut notes = raw
        .into_iter()
        .map(from_js)
        .collect::<Result<Vec<bruhs_models::MeetingNote>, _>>()?;
    notes.sort_unstable_by_key(|note| std::cmp::Reverse(note.updated_at));
    Ok(notes)
}

/// Wrapper record so the messages store can be keyed by `thread_id` while
/// holding an arbitrarily long list of messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MessagesRecord {
    pub thread_id: String,
    pub messages: Vec<AgentMessage>,
}

fn store_params(key: &str) -> ObjectStoreParams {
    let mut params = ObjectStoreParams::new();
    params.key_path(Some(KeyPath::new_single(key)));
    params
}

/// Serialize a value into a JS-compatible `JsValue` (plain JS objects, not Maps).
/// The `json_compatible` serializer is required so IndexedDB sees structured-clone
/// friendly objects.
fn to_js<T: Serialize>(value: &T) -> Result<JsValue, idb::Error> {
    value
        .serialize(&Serializer::json_compatible())
        .map_err(|e| {
            idb::Error::UnexpectedJsType("serialize failure", JsValue::from_str(&e.to_string()))
        })
}

fn from_js<T: for<'de> Deserialize<'de>>(value: JsValue) -> Result<T, idb::Error> {
    serde_wasm_bindgen::from_value(value).map_err(|e| {
        idb::Error::UnexpectedJsType("deserialize failure", JsValue::from_str(&e.to_string()))
    })
}

/// The page's connection to the database, shared by every operation. It is
/// opened on first use and stays open; when another tab upgrades the schema
/// (`versionchange`) or the browser closes it, it is dropped from the cache
/// and closed once the transactions holding it finish, and the next
/// operation opens a fresh one.
#[derive(Clone)]
pub struct Connection(Rc<OpenDatabase>);

impl std::ops::Deref for Connection {
    type Target = Database;
    fn deref(&self) -> &Database {
        &self.0 .0
    }
}

/// Closes the connection when the last holder lets go (after its running
/// transactions finish, as `close` waits for them).
pub struct OpenDatabase(Database);

impl Drop for OpenDatabase {
    fn drop(&mut self) {
        self.0.close();
    }
}

thread_local! {
    /// `(generation, connection)`: the generation tells a stale connection's
    /// close handler not to evict a newer one.
    static SHARED_DB: RefCell<Option<(u64, Rc<OpenDatabase>)>> = const { RefCell::new(None) };
    static DB_GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// Forget connection `generation` if it is still the cached one. The drop is
/// deferred: this runs inside the connection's own event handler, which the
/// connection owns.
fn evict_db(generation: u64) {
    let evicted = SHARED_DB.with(|shared| {
        let mut shared = shared.borrow_mut();
        match shared.as_ref() {
            Some((cached, _)) if *cached == generation => shared.take(),
            _ => None,
        }
    });
    if let Some(evicted) = evicted {
        wasm_bindgen_futures::spawn_local(async move { drop(evicted) });
    }
}

/// The shared connection, opening (and upgrading) the database on first use.
pub async fn open_db() -> Result<Connection, idb::Error> {
    if let Some((_, db)) = SHARED_DB.with(|shared| shared.borrow().clone()) {
        return Ok(Connection(db));
    }
    let generation = DB_GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let mut db = open_fresh().await?;
    // Another tab wants a newer schema: let it, instead of blocking it.
    db.on_version_change(move |_| evict_db(generation));
    db.on_close(move |_| evict_db(generation));
    let db = Rc::new(OpenDatabase(db));
    // Two first uses can race; keep whichever connection was cached first.
    let shared =
        SHARED_DB.with(|shared| Rc::clone(&shared.borrow_mut().get_or_insert((generation, db)).1));
    Ok(Connection(shared))
}

async fn open_fresh() -> Result<Database, idb::Error> {
    let factory = Factory::new()?;
    let mut req = factory.open(DB_NAME, Some(DB_VERSION))?;

    req.on_upgrade_needed(|event| {
        let db = match event.database() {
            Ok(db) => db,
            Err(e) => {
                log::error!("idb upgrade: failed to get database handle: {e:?}");
                return;
            }
        };

        // Upgrade additively without trying to recreate existing stores.
        for (name, key) in [
            (STORE_METADATA, "key"),
            (STORE_THREADS, "id"),
            (STORE_MESSAGES, "thread_id"),
            (STORE_WORK_ITEMS, "id"),
            (STORE_PROJECTS, "id"),
            (STORE_EVENTS, "id"),
            (STORE_CALENDARS, "id"),
            (STORE_MEETINGS, "id"),
            (STORE_MEETING_NOTES, "id"),
            (STORE_MEETING_GENERATIONS, "id"),
            (STORE_SEGMENTS, "id"),
            (STORE_PEOPLE, "id"),
            (STORE_CAPTURES, "id"),
            (STORE_KNOWLEDGE_DOCUMENTS, "id"),
            (STORE_SOURCE_CLIPS, "id"),
            (STORE_KNOWLEDGE_ATTACHMENTS, "id"),
            (STORE_INBOX_CONVERSATIONS, "id"),
            (STORE_RELATIONSHIPS, "person_id"),
        ] {
            if db.store_names().iter().any(|existing| existing == name) {
                continue;
            }
            if let Err(e) = db.create_object_store(name, store_params(key)) {
                log::warn!("idb upgrade: create_object_store({name}) skipped: {e:?}");
            }
        }
        if let Ok(request) = event.target() {
            if let Some(tx) = request.transaction() {
                let result = tx.object_store(STORE_SEGMENTS).and_then(|store| {
                    if !store.index_names().iter().any(|name| name == "by_meeting") {
                        store.create_index(
                            "by_meeting",
                            KeyPath::new_single("meeting_id"),
                            None,
                        )?;
                    }
                    Ok(())
                });
                if result.is_err() {
                    log::error!("Transcript index upgrade failed; original database retained.");
                    let _ = tx.abort();
                }
            }
        }
    });

    req.await
}

// ── Threads ──────────────────────────────────────────────────────────────────

pub async fn load_threads() -> Result<Vec<Thread>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_THREADS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        tx.object_store(STORE_THREADS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| metadata::SaveError::Corrupt))
            .collect::<Result<Vec<Thread>, _>>()
    }
    .await;
    metadata::finish(tx, result).await
}

// ── Messages (per thread) ────────────────────────────────────────────────────

pub(crate) async fn load_messages(
    thread_id: &str,
) -> Result<Option<MessagesRecord>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_MESSAGES], TransactionMode::ReadOnly)?,
    );
    let result = async {
        let store = tx.object_store(STORE_MESSAGES)?;
        match store.get(JsValue::from_str(thread_id))?.await? {
            Some(raw) => {
                let record: MessagesRecord =
                    from_js(raw).map_err(|_| metadata::SaveError::Corrupt)?;
                if record.thread_id != thread_id {
                    return Err(metadata::SaveError::Corrupt);
                }
                // Preserve the raw baseline; normalize only the display copy.
                Ok(Some(record))
            }
            None => Ok(None),
        }
    }
    .await;
    metadata::finish(tx, result).await
}

// ── Work items ───────────────────────────────────────────────────────────────

/// Commit the task links and their local calendar blocks together. A plan is
/// either durable in both stores or absent from both stores.
pub async fn save_day_plan(
    work_items: &[WorkItem],
    events: &[CalendarEvent],
) -> Result<(), idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(
        &[STORE_WORK_ITEMS, STORE_EVENTS],
        TransactionMode::ReadWrite,
    )?;
    let work_item_store = tx.object_store(STORE_WORK_ITEMS)?;
    let event_store = tx.object_store(STORE_EVENTS)?;
    for work_item in work_items {
        work_item_store.put(&to_js(work_item)?, None)?.await?;
    }
    for event in events {
        event_store.put(&to_js(event)?, None)?.await?;
    }
    tx.commit()?.await?;
    Ok(())
}

/// Reverse an applied day plan in one IndexedDB transaction.
pub async fn undo_day_plan(
    work_items: &[WorkItem],
    event_ids: &[String],
) -> Result<(), idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(
        &[STORE_WORK_ITEMS, STORE_EVENTS],
        TransactionMode::ReadWrite,
    )?;
    let work_item_store = tx.object_store(STORE_WORK_ITEMS)?;
    let event_store = tx.object_store(STORE_EVENTS)?;
    for work_item in work_items {
        work_item_store.put(&to_js(work_item)?, None)?.await?;
    }
    for event_id in event_ids {
        event_store.delete(JsValue::from_str(event_id))?.await?;
    }
    tx.commit()?.await?;
    Ok(())
}

// ── Projects ─────────────────────────────────────────────────────────────────

pub async fn load_projects() -> Result<Vec<Project>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_PROJECTS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        tx.object_store(STORE_PROJECTS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| metadata::SaveError::Corrupt))
            .collect()
    }
    .await;
    metadata::finish(tx, result).await
}

// ── Calendar events ──────────────────────────────────────────────────────────

pub async fn load_events() -> Result<Vec<CalendarEvent>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_EVENTS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        tx.object_store(STORE_EVENTS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| metadata::SaveError::Corrupt))
            .collect::<Result<Vec<CalendarEvent>, _>>()
    }
    .await;
    metadata::finish(tx, result).await
}

// ── Calendars ────────────────────────────────────────────────────────────────

pub async fn save_calendar(calendar: &Calendar) -> Result<(), idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_CALENDARS], TransactionMode::ReadWrite)?;
    let store = tx.object_store(STORE_CALENDARS)?;
    store.put(&to_js(calendar)?, None)?.await?;
    tx.commit()?.await?;
    Ok(())
}
pub async fn load_calendars() -> Result<Vec<Calendar>, idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_CALENDARS], TransactionMode::ReadOnly)?;
    let store = tx.object_store(STORE_CALENDARS)?;
    let raw = store.get_all(None, None)?.await?;
    tx.await?;
    let mut calendars = Vec::with_capacity(raw.len());
    for v in raw {
        match from_js::<Calendar>(v) {
            Ok(c) => calendars.push(c),
            Err(e) => log::warn!("idb load_calendars: skipping malformed record: {e:?}"),
        }
    }
    Ok(calendars)
}

// ── Meetings ─────────────────────────────────────────────────────────────────

pub async fn save_meeting(meeting: &Meeting) -> Result<(), idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_MEETINGS], TransactionMode::ReadWrite)?;
    let store = tx.object_store(STORE_MEETINGS)?;
    store.put(&to_js(meeting)?, None)?.await?;
    tx.commit()?.await?;
    Ok(())
}

pub async fn load_meetings() -> Result<Vec<Meeting>, idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_MEETINGS], TransactionMode::ReadOnly)?;
    let store = tx.object_store(STORE_MEETINGS)?;
    let raw = store.get_all(None, None)?.await?;
    tx.await?;
    let mut out = Vec::with_capacity(raw.len());
    for v in raw {
        match from_js::<Meeting>(v) {
            Ok(m) => out.push(m),
            Err(e) => log::warn!("idb load_meetings: skipping malformed record: {e:?}"),
        }
    }
    Ok(out)
}

// ── Transcript segments ──────────────────────────────────────────────────────

/// Loads every finalized transcript segment once for the reusable universal
/// search index. Live partials remain available through `MeetingSegmentsSignal`
/// but are excluded here because the canonical pass will replace them.
pub async fn load_all_segments() -> Result<Vec<TranscriptSegment>, idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_SEGMENTS], TransactionMode::ReadOnly)?;
    let store = tx.object_store(STORE_SEGMENTS)?;
    let raw = store.get_all(None, None)?.await?;
    tx.await?;

    let mut out = raw
        .into_iter()
        .filter_map(|value| match from_js::<TranscriptSegment>(value) {
            Ok(segment) if !segment.partial => Some(segment),
            Ok(_) => None,
            Err(error) => {
                log::warn!("idb load_all_segments: skipping malformed record: {error:?}");
                None
            }
        })
        .collect::<Vec<_>>();
    out.sort_by(|left, right| {
        right
            .meeting_id
            .cmp(&left.meeting_id)
            .then_with(|| left.start_ms.cmp(&right.start_ms))
    });
    Ok(out)
}

// ── Universal capture ────────────────────────────────────────────────────────

pub async fn load_captures() -> Result<Vec<CaptureRecord>, idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_CAPTURES], TransactionMode::ReadOnly)?;
    let raw = tx
        .object_store(STORE_CAPTURES)?
        .get_all(None, None)?
        .await?;
    tx.await?;
    let mut captures = Vec::with_capacity(raw.len());
    for value in raw {
        match from_js::<CaptureRecord>(value) {
            Ok(capture) => captures.push(capture),
            Err(error) => log::warn!("idb load_captures: skipping malformed record: {error:?}"),
        }
    }
    captures.sort_unstable_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
    Ok(captures)
}

// ── Knowledge ───────────────────────────────────────────────────────────────

pub async fn load_knowledge_documents() -> Result<Vec<KnowledgeDocument>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_KNOWLEDGE_DOCUMENTS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        tx.object_store(STORE_KNOWLEDGE_DOCUMENTS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| metadata::SaveError::Corrupt))
            .collect::<Result<Vec<KnowledgeDocument>, _>>()
    }
    .await;
    let mut documents = metadata::finish(tx, result).await?;
    documents.sort_unstable_by_key(|document| std::cmp::Reverse(document.updated_at_ms));
    Ok(documents)
}

pub async fn load_source_clips() -> Result<Vec<SourceClip>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_SOURCE_CLIPS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        tx.object_store(STORE_SOURCE_CLIPS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| metadata::SaveError::Corrupt))
            .collect::<Result<Vec<SourceClip>, _>>()
    }
    .await;
    let mut clips = metadata::finish(tx, result).await?;
    clips.sort_unstable_by_key(|clip| std::cmp::Reverse(clip.updated_at_ms));
    Ok(clips)
}

fn attachment_to_js(attachment: &KnowledgeAttachment) -> Result<JsValue, idb::Error> {
    // One byte-buffer copy, not millions of per-byte WASM/JS crossings. Legacy
    // plain arrays remain readable through serde_bytes's sequence visitor.
    attachment
        .serialize(&Serializer::json_compatible().serialize_bytes_as_arrays(false))
        .map_err(|error| {
            idb::Error::UnexpectedJsType(
                "serialize attachment",
                JsValue::from_str(&error.to_string()),
            )
        })
}

pub async fn load_knowledge_attachment(
    id: &str,
) -> Result<Option<KnowledgeAttachment>, idb::Error> {
    let db = open_db().await?;
    let tx = db.transaction(&[STORE_KNOWLEDGE_ATTACHMENTS], TransactionMode::ReadOnly)?;
    let raw = tx
        .object_store(STORE_KNOWLEDGE_ATTACHMENTS)?
        .get(JsValue::from_str(id))?
        .await?;
    tx.await?;
    raw.map(from_js).transpose()
}

// ── Inbox and relationships ────────────────────────────────────────────────

pub async fn load_inbox_conversations() -> Result<Vec<InboxConversation>, metadata::SaveError> {
    let db = open_db().await?;
    let tx = metadata::LocalTransaction::new(
        db.transaction(&[STORE_INBOX_CONVERSATIONS], TransactionMode::ReadOnly)?,
    );
    let result = async {
        tx.object_store(STORE_INBOX_CONVERSATIONS)?
            .get_all(None, None)?
            .await?
            .into_iter()
            .map(|raw| from_js(raw).map_err(|_| metadata::SaveError::Corrupt))
            .collect::<Result<Vec<InboxConversation>, _>>()
    }
    .await;
    let mut conversations = metadata::finish(tx, result).await?;
    conversations.sort_unstable_by_key(|item| std::cmp::Reverse(item.updated_at_ms));
    Ok(conversations)
}

// ── Fire-and-forget helpers ──────────────────────────────────────────────────
//
// Legacy helpers log errors only. Task mutations instead use the app-owned
// ordered writer below, with visible recovery and exact-original checks.

pub fn persist_work_item(expected: Option<WorkItem>, item: WorkItem) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit(
        item.id.clone(),
        expected,
        Some(item),
    );
}

pub fn forget_work_item(item: WorkItem) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit(
        item.id.clone(),
        Some(item),
        None,
    );
}

pub fn persist_event(expected: Option<CalendarEvent>, event: CalendarEvent) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_event(
        event.id.clone(),
        expected,
        Some(event),
    );
}

pub fn persist_capture(expected: Option<CaptureRecord>, capture: CaptureRecord) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>()
        .edit_capture(expected, capture);
}

pub fn persist_knowledge_document(
    expected: Option<KnowledgeDocument>,
    document: KnowledgeDocument,
) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_document(
        document.id.clone(),
        expected,
        Some(document),
    );
}

pub fn forget_knowledge_document(original: KnowledgeDocument) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_document(
        original.id.clone(),
        Some(original),
        None,
    );
}

pub fn persist_source_clip(expected: Option<SourceClip>, clip: SourceClip) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_source(expected, clip);
}

pub fn persist_inbox_conversation(
    expected: Option<InboxConversation>,
    conversation: InboxConversation,
) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_inbox(
        conversation.id.clone(),
        expected,
        Some(conversation),
    );
}

pub fn forget_inbox_conversation(original: InboxConversation) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_inbox(
        original.id.clone(),
        Some(original),
        None,
    );
}

pub fn forget_event(original: CalendarEvent) {
    leptos::prelude::expect_context::<crate::task_saves::TaskSaves>().edit_event(
        original.id.clone(),
        Some(original),
        None,
    );
}

pub fn persist_calendar(calendar: Calendar) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = save_calendar(&calendar).await {
            log::warn!("idb save_calendar failed: {e:?}");
        }
    });
}

pub fn persist_meeting(meeting: Meeting) {
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = save_meeting(&meeting).await {
            log::warn!("idb save_meeting failed: {e:?}");
        }
    });
}
