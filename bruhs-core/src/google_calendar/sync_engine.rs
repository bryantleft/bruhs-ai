//! Background sync engine for Google Calendar. Owns the polling tokio
//! task; consumers subscribe to a tokio::sync::broadcast channel of
//! `CalendarSyncEvent`s.
//!
//! Durability: Google's `nextSyncToken` only moves forward once every
//! upsert/delete fetched with it was handed to at least one subscriber. If
//! nobody is listening (or a send fails) the old token is kept and the next
//! tick fetches the same changes again. A subscriber that falls behind the
//! broadcast buffer loses events after they were sent; it asks for
//! [`SyncEngine::force_full_sync`], which re-lists every enabled calendar
//! without a sync token so the dropped events are replayed.

use bruhs_models::CalendarSyncEvent;
use bruhs_models::EpochMs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, Notify};

use super::cursor::CalendarFiles;
use super::tokens::{TokenError, TokenStore};

/// Broadcast channel capacity. Each subscriber buffers up to this many
/// events before the slowest one is dropped. 256 is generous for a poll
/// that emits at most ~30 events per tick.
const CHANNEL_CAPACITY: usize = 256;

/// Default poll interval: 5 minutes (matches spec §3.4).
pub const SYNC_INTERVAL_SECS: u64 = 300;

/// Calendar-list refresh cadence: 1 hour (matches spec §3.4).
pub const CALENDARS_REFRESH_SECS: u64 = 3600;

/// Calendar synced when the stored calendar list is empty (first run).
pub const PRIMARY_CALENDAR: &str = "primary";

#[derive(Clone)]
pub struct SyncEngine {
    tx: broadcast::Sender<CalendarSyncEvent>,
    force: Arc<Notify>,
    /// Set by `force_full_sync`; the next tick ignores stored sync tokens.
    full_resync: Arc<AtomicBool>,
    tokens: TokenStore,
    files: CalendarFiles,
}

impl SyncEngine {
    pub fn new(tokens: TokenStore, files: CalendarFiles) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx,
            force: Arc::new(Notify::new()),
            full_resync: Arc::new(AtomicBool::new(false)),
            tokens,
            files,
        }
    }

    /// Subscribe to the event stream. Each subscriber sees events that
    /// land AFTER it subscribes; clients hydrate earlier state from their
    /// own storage.
    pub fn subscribe(&self) -> broadcast::Receiver<CalendarSyncEvent> {
        self.tx.subscribe()
    }

    /// Trigger an immediate sync tick (used by the Refresh button).
    pub fn force_sync(&self) {
        self.force.notify_one();
    }

    /// Trigger an immediate tick that re-lists every enabled calendar from
    /// scratch instead of continuing from the stored sync tokens. Used when
    /// a subscriber lagged and lost events.
    pub fn force_full_sync(&self) {
        self.full_resync.store(true, Ordering::SeqCst);
        self.force.notify_one();
    }

    /// Whether a full resync was requested and has not completed yet.
    pub fn full_sync_pending(&self) -> bool {
        self.full_resync.load(Ordering::SeqCst)
    }

    pub(super) fn tokens(&self) -> &TokenStore {
        &self.tokens
    }

    pub(super) fn files(&self) -> &CalendarFiles {
        &self.files
    }

    /// Send `event` to all subscribers. Returns whether at least one
    /// subscriber received it (status events may go unheard).
    pub(super) fn emit(&self, event: CalendarSyncEvent) -> bool {
        self.tx.send(event).is_ok()
    }

    fn emit_failed(&self, now_ms: i64, message: String) {
        tracing::warn!("google calendar sync: {message}");
        // Status only: nothing to retry if nobody is listening.
        self.emit(CalendarSyncEvent::SyncFailed {
            at_ms: EpochMs::saturating_from_i64(now_ms).as_f64(),
            message,
        });
    }
}

use chrono::{Duration as ChronoDuration, Utc};
use std::time::Duration;

use super::client::{CalendarClient, ListEventsRequest};
use super::config::OAuthConfig;
use super::cursor::SyncCursor;
use super::mapping::{classify, ClassifiedEvent};
use super::oauth::{refresh_access_token, OAuthError};

/// What one calendar's sync produced.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CalendarOutcome {
    upserted: usize,
    deleted: usize,
    /// Every change event reached at least one subscriber.
    delivered: bool,
    /// The calendar could not be listed; a `SyncFailed` was emitted.
    failed: bool,
    /// The provider rejected the stored sync token (HTTP 410).
    token_expired: bool,
}

/// The calendar one `sync_one_calendar` call lists.
struct CalendarRef<'a> {
    client: &'a CalendarClient,
    access_token: &'a str,
    local_id: &'a str,
    provider_id: &'a str,
}

/// The end of one successful listing.
struct Listed {
    next_sync_token: Option<String>,
}

/// Days before now that a window listing covers.
pub const SYNC_WINDOW_PAST_DAYS: i64 = 7;
/// Days after now that a window listing covers: the six weeks a month
/// grid can show from today.
pub const SYNC_WINDOW_FUTURE_DAYS: i64 = 42;
/// How old a window listing may get before the window is listed again.
pub const WINDOW_RELIST_SECS: i64 = 86_400;

/// A `timeMin`..`timeMax` listing range around a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SyncWindow {
    time_min: chrono::DateTime<Utc>,
    time_max: chrono::DateTime<Utc>,
}

impl SyncWindow {
    fn around(now_ms: i64) -> Self {
        let now = chrono::DateTime::<Utc>::from_timestamp_millis(now_ms).unwrap_or_else(Utc::now);
        Self {
            time_min: now - ChronoDuration::days(SYNC_WINDOW_PAST_DAYS),
            time_max: now + ChronoDuration::days(SYNC_WINDOW_FUTURE_DAYS),
        }
    }
}

/// One `events.list` request shape.
#[derive(Debug, Clone, Copy)]
enum Listing<'a> {
    /// Changes since a sync token.
    Changes(&'a str),
    /// Every event in a range.
    Window(SyncWindow),
}

/// Whether the window has not been listed within [`WINDOW_RELIST_SECS`].
fn window_relist_due(cursor: &SyncCursor, now_ms: i64) -> bool {
    cursor.window_listed_at_ms.is_none_or(|listed| {
        now_ms.saturating_sub(listed) >= WINDOW_RELIST_SECS.saturating_mul(1_000)
    })
}

impl SyncEngine {
    /// Run one sync tick for all enabled calendars. Failures are reported
    /// as `SyncFailed` events.
    pub(super) async fn run_tick(&self, cfg: &OAuthConfig, client: &CalendarClient) {
        let now_ms = Utc::now().timestamp_millis();
        let full = self.full_resync.swap(false, Ordering::SeqCst);
        self.emit(CalendarSyncEvent::SyncStarted {
            at_ms: EpochMs::saturating_from_i64(now_ms).as_f64(),
        });

        // Refresh access token once before iterating calendars.
        let mut current = match self.tokens.get().await {
            Ok(t) => t,
            Err(TokenError::NotFound) => {
                self.emit(CalendarSyncEvent::SyncCompleted {
                    at_ms: EpochMs::saturating_from_i64(now_ms).as_f64(),
                    upserted: 0,
                    deleted: 0,
                });
                return;
            }
            Err(e) => {
                self.restore_full_resync(full);
                self.emit_failed(now_ms, format!("keychain read: {e}"));
                return;
            }
        };
        if current.is_expiring_soon(now_ms) {
            match refresh_access_token(cfg, &current.refresh_token).await {
                Ok(refreshed) => {
                    current.access_token = refreshed.access_token;
                    current.expires_at_ms = now_ms + refreshed.expires_in_secs * 1000;
                    if let Err(e) = self.tokens.put(&current).await {
                        self.restore_full_resync(full);
                        self.emit_failed(now_ms, format!("keychain write: {e}"));
                        return;
                    }
                }
                Err(e) => {
                    self.restore_full_resync(full);
                    self.emit_failed(now_ms, format!("token refresh: {e}"));
                    return;
                }
            }
        }

        // Enumerate enabled calendars. If the persisted list is empty
        // (first run, or never refreshed), fall back to PRIMARY_CALENDAR
        // so the first tick still works. An unreadable list is an error:
        // syncing "primary" instead would ignore the user's toggles.
        let calendars = match self.files.load_calendars().await {
            Ok(list) => list,
            Err(e) => {
                self.restore_full_resync(full);
                self.emit_failed(now_ms, format!("calendar list: {e}"));
                return;
            }
        };
        let enabled: Vec<(String, String)> = if calendars.is_empty() {
            vec![(PRIMARY_CALENDAR.to_string(), PRIMARY_CALENDAR.to_string())]
        } else {
            calendars
                .iter()
                .filter(|c| c.sync_enabled)
                .filter_map(|c| c.provider_id.as_ref().map(|p| (c.id.clone(), p.clone())))
                .collect()
        };

        let mut total_upserted = 0usize;
        let mut total_deleted = 0usize;
        let mut all_delivered = true;
        let mut any_failed = false;
        for (local_id, provider_id) in &enabled {
            let outcome = self
                .sync_one_calendar(
                    client,
                    &current.access_token,
                    local_id,
                    provider_id,
                    now_ms,
                    full,
                )
                .await;
            total_upserted += outcome.upserted;
            total_deleted += outcome.deleted;
            all_delivered &= outcome.delivered;
            any_failed |= outcome.failed;
        }
        // A replay nobody heard has to happen again for the next listener.
        self.restore_full_resync(full && !all_delivered);

        // A failed calendar already reported SyncFailed. Completing the tick
        // would clear that error and advance the client's sync time past
        // changes it never received.
        if any_failed {
            return;
        }
        self.emit(CalendarSyncEvent::SyncCompleted {
            at_ms: EpochMs::saturating_from_i64(now_ms).as_f64(),
            upserted: total_upserted,
            deleted: total_deleted,
        });
    }

    /// Re-arm a full resync that this tick consumed but did not finish.
    fn restore_full_resync(&self, needed: bool) {
        if needed {
            self.full_resync.store(true, Ordering::SeqCst);
        }
    }

    /// Sync a single calendar. Never errors; failures are emitted as
    /// `SyncFailed` events.
    ///
    /// Without a stored sync token (first run, after a 410, or with
    /// `full`) the sync window is listed and its `nextSyncToken` starts the
    /// incremental chain. With a token, changes since it are listed; then,
    /// once the last window listing is older than [`WINDOW_RELIST_SECS`],
    /// the window is listed again so instances of unchanged recurring
    /// series that slid into it arrive. That relisting only adds upserts:
    /// the incremental token chain continues untouched, so no change made
    /// outside the window is skipped.
    async fn sync_one_calendar(
        &self,
        client: &CalendarClient,
        access_token: &str,
        local_id: &str,
        provider_id: &str,
        now_ms: i64,
        full: bool,
    ) -> CalendarOutcome {
        let mut cursor = match self.files.load_cursor(provider_id).await {
            Ok(cursor) => cursor,
            Err(e) => {
                // Unreadable cursor: fall back to a full window listing.
                tracing::warn!("sync cursor for {provider_id} unreadable, resyncing: {e}");
                SyncCursor::default()
            }
        };
        let request_token = if full {
            None
        } else {
            cursor.sync_token.clone()
        };
        let mut outcome = CalendarOutcome {
            delivered: true,
            ..CalendarOutcome::default()
        };
        let calendar = CalendarRef {
            client,
            access_token,
            local_id,
            provider_id,
        };

        let Some(token) = request_token else {
            let window = SyncWindow::around(now_ms);
            let Some(listed) = self
                .list_all_pages(&calendar, Listing::Window(window), now_ms, &mut outcome)
                .await
            else {
                return outcome;
            };
            if outcome.delivered {
                if let Some(ns) = listed.next_sync_token {
                    cursor.sync_token = Some(ns);
                }
                cursor.window_listed_at_ms = Some(now_ms);
            } else {
                tracing::info!(
                    "holding sync token for {provider_id}: changes were not delivered to any subscriber"
                );
            }
            cursor.last_synced_at_ms = Some(now_ms);
            self.save_cursor(provider_id, cursor).await;
            return outcome;
        };

        match self
            .list_all_pages(&calendar, Listing::Changes(&token), now_ms, &mut outcome)
            .await
        {
            Some(listed) if outcome.delivered => {
                if let Some(ns) = listed.next_sync_token {
                    cursor.sync_token = Some(ns);
                }
            }
            Some(_) => tracing::info!(
                "holding sync token for {provider_id}: changes were not delivered to any subscriber"
            ),
            None if outcome.token_expired => {
                cursor.sync_token = None;
                self.save_cursor(provider_id, cursor).await;
                return outcome;
            }
            None => return outcome,
        }

        if window_relist_due(&cursor, now_ms) {
            let changes_delivered = outcome.delivered;
            outcome.delivered = true;
            let window = SyncWindow::around(now_ms);
            let listed = self
                .list_all_pages(&calendar, Listing::Window(window), now_ms, &mut outcome)
                .await;
            if listed.is_none() {
                // The changes above were handled; keep their token.
                self.save_cursor(provider_id, cursor).await;
                outcome.delivered = false;
                return outcome;
            }
            if outcome.delivered {
                cursor.window_listed_at_ms = Some(now_ms);
            }
            outcome.delivered &= changes_delivered;
        }
        cursor.last_synced_at_ms = Some(now_ms);
        self.save_cursor(provider_id, cursor).await;
        outcome
    }

    /// Lists every page of one request, emitting its changes. Returns the
    /// final page's `nextSyncToken`, or `None` when a request failed (a
    /// `SyncFailed` was emitted and `outcome` marks the failure).
    async fn list_all_pages(
        &self,
        calendar: &CalendarRef<'_>,
        listing: Listing<'_>,
        now_ms: i64,
        outcome: &mut CalendarOutcome,
    ) -> Option<Listed> {
        let provider_id = calendar.provider_id;
        let mut page_token: Option<String> = None;
        loop {
            let req = build_request_for(provider_id, listing, page_token.take());
            let resp = match calendar
                .client
                .list_events(calendar.access_token, &req)
                .await
            {
                Ok(r) => r,
                Err(OAuthError::Status { status: 410, body }) => {
                    tracing::info!("sync token invalidated for {provider_id}: 410");
                    self.emit_failed(
                        now_ms,
                        format!("sync token expired ({provider_id}): {body}"),
                    );
                    outcome.delivered = false;
                    outcome.failed = true;
                    outcome.token_expired = true;
                    return None;
                }
                Err(e) => {
                    self.emit_failed(now_ms, format!("events.list ({provider_id}): {e}"));
                    outcome.delivered = false;
                    outcome.failed = true;
                    return None;
                }
            };
            for raw in &resp.items {
                match classify(raw, Some(calendar.local_id), now_ms) {
                    Ok(ClassifiedEvent::Upsert(event)) => {
                        outcome.upserted += 1;
                        outcome.delivered &= self.emit(CalendarSyncEvent::EventUpserted { event });
                    }
                    Ok(ClassifiedEvent::Delete { google_event_id }) => {
                        outcome.deleted += 1;
                        outcome.delivered &=
                            self.emit(CalendarSyncEvent::EventDeleted { google_event_id });
                    }
                    Err(reason) => {
                        tracing::warn!("skipping malformed event in {provider_id}: {reason}");
                    }
                }
            }
            if let Some(np) = resp.next_page_token {
                page_token = Some(np);
                continue;
            }
            return Some(Listed {
                next_sync_token: resp.next_sync_token,
            });
        }
    }

    async fn save_cursor(&self, provider_id: &str, cursor: SyncCursor) {
        if let Err(e) = self.files.save_cursor(provider_id, cursor).await {
            tracing::warn!("could not save sync cursor for {provider_id}: {e}");
        }
    }

    /// Re-enumerate the user's calendars and persist the result. Emits
    /// `CalendarsRefreshed`. Preserves the user's `sync_enabled` toggle
    /// across refreshes; newly-discovered calendars default to
    /// `sync_enabled = primary` (primary on, others off). Nothing is written
    /// when the stored list cannot be read.
    pub(super) async fn refresh_calendars(&self, client: &CalendarClient) {
        let now_ms = Utc::now().timestamp_millis();
        let current = match self.tokens.get().await {
            Ok(t) => t,
            Err(TokenError::NotFound) => return,
            Err(e) => {
                self.emit_failed(now_ms, format!("keychain read: {e}"));
                return;
            }
        };
        let resp = match client.list_calendars(&current.access_token).await {
            Ok(r) => r,
            Err(e) => {
                self.emit_failed(now_ms, format!("calendarList: {e}"));
                return;
            }
        };
        let merged = self
            .files
            .merge_calendars(move |existing| merge_calendar_list(&existing, &resp.items))
            .await;
        match merged {
            Ok(next) => {
                // Status only: clients also fetch the list on demand.
                self.emit(CalendarSyncEvent::CalendarsRefreshed { calendars: next });
            }
            Err(e) => self.emit_failed(now_ms, format!("save calendars: {e}")),
        }
    }

    /// Drive the engine forever. Polls every `SYNC_INTERVAL_SECS`, or
    /// immediately when `force_sync()` is signalled. Re-enumerates the
    /// calendar list every `CALENDARS_REFRESH_SECS`.
    pub async fn run_forever(self, cfg: OAuthConfig, client: CalendarClient) {
        let force = Arc::clone(&self.force);
        let mut event_tick = tokio::time::interval(Duration::from_secs(SYNC_INTERVAL_SECS));
        let mut calendars_tick = tokio::time::interval(Duration::from_secs(CALENDARS_REFRESH_SECS));
        event_tick.tick().await; // consume the immediate first tick
        calendars_tick.tick().await; // consume the immediate first tick

        // First run: refresh calendars now so later event ticks have a
        // list to iterate.
        self.refresh_calendars(&client).await;
        loop {
            tokio::select! {
                _ = event_tick.tick() => self.run_tick(&cfg, &client).await,
                _ = calendars_tick.tick() => self.refresh_calendars(&client).await,
                // Force triggers an immediate event tick ("Refresh" in
                // Settings, or a lagged subscriber asking for a replay).
                _ = force.notified() => self.run_tick(&cfg, &client).await,
            }
        }
    }
}

/// The stored list updated from a fresh `calendarList` response: known
/// calendars keep their local id, toggle and last sync time.
fn merge_calendar_list(
    existing: &[bruhs_models::Calendar],
    fetched: &[super::client::RawCalendarListItem],
) -> Vec<bruhs_models::Calendar> {
    let mut next: Vec<bruhs_models::Calendar> = fetched
        .iter()
        .map(|raw| {
            let prior = existing
                .iter()
                .find(|c| c.provider_id.as_deref() == Some(raw.id.as_str()));
            bruhs_models::Calendar {
                id: prior.map_or_else(|| uuid::Uuid::new_v4().to_string(), |p| p.id.clone()),
                source: bruhs_models::EventSource::Google,
                provider_id: Some(raw.id.clone()),
                name: raw
                    .summary_override
                    .clone()
                    .or_else(|| raw.summary.clone())
                    .unwrap_or_else(|| raw.id.clone()),
                color: None,
                sync_enabled: prior.map_or(raw.primary, |p| p.sync_enabled),
                primary: raw.primary,
                last_synced_at: prior.and_then(|p| p.last_synced_at),
            }
        })
        .collect();
    next.sort_by(|a, b| b.primary.cmp(&a.primary).then(a.name.cmp(&b.name)));
    next
}

fn build_request_for(
    provider_id: &str,
    listing: Listing<'_>,
    page_token: Option<String>,
) -> ListEventsRequest {
    match listing {
        Listing::Changes(token) => ListEventsRequest {
            calendar_id: provider_id.to_string(),
            sync_token: Some(token.to_string()),
            page_token,
            ..Default::default()
        },
        Listing::Window(window) => ListEventsRequest {
            calendar_id: provider_id.to_string(),
            time_min_rfc3339: Some(window.time_min.to_rfc3339()),
            time_max_rfc3339: Some(window.time_max.to_rfc3339()),
            page_token,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod tick_tests {
    use super::*;
    use crate::google_calendar::config::OAuthConfig;
    use crate::google_calendar::tokens::TokenSet;
    use bruhs_models::{Calendar, EventSource};
    use wiremock::matchers::{method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct Fixture {
        dir: tempfile::TempDir,
        server: MockServer,
        engine: SyncEngine,
        cfg: OAuthConfig,
        client: CalendarClient,
    }

    /// Engine with fresh in-memory tokens and a temp state directory; never
    /// touches the real keychain or `~/Library`.
    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let tokens = TokenStore::in_memory();
        tokens
            .put(&TokenSet {
                access_token: "at-current".into(),
                refresh_token: "rt-current".into(),
                expires_at_ms: i64::MAX / 2,
                email: Some("test@example.com".into()),
                obtained_at_ms: 0,
            })
            .await
            .unwrap();
        let server = MockServer::start().await;
        Fixture {
            engine: SyncEngine::new(tokens, CalendarFiles::at(dir.path())),
            cfg: OAuthConfig::for_test(&server.uri()),
            client: CalendarClient::with_base(server.uri()),
            server,
            dir,
        }
    }

    fn one_event_page(sync_token: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "items": [
                {
                    "id": "evt-1",
                    "status": "confirmed",
                    "summary": "Sync",
                    "start": { "dateTime": "2026-05-15T09:00:00Z" },
                    "end":   { "dateTime": "2026-05-15T10:00:00Z" }
                }
            ],
            "nextSyncToken": sync_token
        }))
    }

    fn drain(rx: &mut broadcast::Receiver<CalendarSyncEvent>) -> Vec<CalendarSyncEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    async fn stored_token(f: &Fixture) -> Option<String> {
        f.engine
            .files
            .load_cursor(PRIMARY_CALENDAR)
            .await
            .unwrap()
            .sync_token
    }

    #[tokio::test]
    async fn run_tick_emits_upserted_for_confirmed_items() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(one_event_page("tok-1"))
            .mount(&f.server)
            .await;

        let mut rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;

        let events = drain(&mut rx);
        assert!(matches!(
            events.as_slice(),
            [
                CalendarSyncEvent::SyncStarted { .. },
                CalendarSyncEvent::EventUpserted { .. },
                CalendarSyncEvent::SyncCompleted {
                    upserted: 1,
                    deleted: 0,
                    ..
                },
            ]
        ));
        assert_eq!(stored_token(&f).await.as_deref(), Some("tok-1"));
    }

    /// With nobody subscribed the fetched changes went nowhere, so the next
    /// tick must fetch them again instead of continuing past them.
    #[tokio::test]
    async fn sync_token_is_held_until_changes_are_delivered() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(one_event_page("tok-1"))
            .mount(&f.server)
            .await;

        f.engine.run_tick(&f.cfg, &f.client).await;
        assert_eq!(stored_token(&f).await, None);

        let mut rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert!(drain(&mut rx)
            .iter()
            .any(|e| matches!(e, CalendarSyncEvent::EventUpserted { .. })));
        assert_eq!(stored_token(&f).await.as_deref(), Some("tok-1"));
    }

    #[tokio::test]
    async fn force_full_sync_ignores_the_stored_token_once() {
        let f = fixture().await;
        f.engine
            .files
            .save_cursor(
                PRIMARY_CALENDAR,
                SyncCursor {
                    sync_token: Some("tok-old".into()),
                    last_synced_at_ms: Some(1),
                    window_listed_at_ms: None,
                },
            )
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param_is_missing("syncToken"))
            .respond_with(one_event_page("tok-full"))
            .expect(1)
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("syncToken", "tok-full"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "items": [], "nextSyncToken": "tok-2" })),
            )
            .expect(1)
            .mount(&f.server)
            .await;

        let mut rx = f.engine.subscribe();
        f.engine.force_full_sync();
        f.engine.force.notified().await; // the permit force_full_sync left
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert!(drain(&mut rx)
            .iter()
            .any(|e| matches!(e, CalendarSyncEvent::EventUpserted { .. })));
        assert_eq!(stored_token(&f).await.as_deref(), Some("tok-full"));

        // The next ordinary tick continues from the new token.
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert_eq!(stored_token(&f).await.as_deref(), Some("tok-2"));
    }

    #[tokio::test]
    async fn undelivered_full_sync_is_retried() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param_is_missing("syncToken"))
            .respond_with(one_event_page("tok-full"))
            .expect(2)
            .mount(&f.server)
            .await;

        f.engine.force_full_sync();
        f.engine.run_tick(&f.cfg, &f.client).await; // nobody listening
        assert!(f.engine.full_resync.load(Ordering::SeqCst));

        let _rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert!(!f.engine.full_resync.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn corrupt_calendar_list_fails_the_tick_without_syncing() {
        let f = fixture().await;
        let list_path = f.dir.path().join("calendars.json");
        std::fs::write(&list_path, b"[{ broken").unwrap();
        Mock::given(method("GET"))
            .respond_with(one_event_page("tok-1"))
            .expect(0)
            .mount(&f.server)
            .await;

        let mut rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert!(drain(&mut rx)
            .iter()
            .any(|e| matches!(e, CalendarSyncEvent::SyncFailed { message, .. } if message.contains("calendar list"))));
    }

    #[tokio::test]
    async fn refresh_keeps_toggles_and_never_overwrites_a_corrupt_list() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/users/me/calendarList"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [
                    { "id": "me@example.com", "summary": "Me", "primary": true },
                    { "id": "team@group.calendar.google.com", "summary": "Team" }
                ]
            })))
            .mount(&f.server)
            .await;
        let team_local = Calendar {
            id: "local-team".into(),
            source: EventSource::Google,
            provider_id: Some("team@group.calendar.google.com".into()),
            name: "Team".into(),
            color: None,
            sync_enabled: true,
            primary: false,
            last_synced_at: Some(7),
        };
        f.engine
            .files
            .merge_calendars(move |_| vec![team_local])
            .await
            .unwrap();

        f.engine.refresh_calendars(&f.client).await;
        let list = f.engine.files.load_calendars().await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].primary && list[0].sync_enabled);
        let team = list.iter().find(|c| c.id == "local-team").unwrap();
        assert!(team.sync_enabled, "user toggle must survive a refresh");
        assert_eq!(team.last_synced_at, Some(7));

        let list_path = f.dir.path().join("calendars.json");
        std::fs::write(&list_path, b"not json").unwrap();
        let mut rx = f.engine.subscribe();
        f.engine.refresh_calendars(&f.client).await;
        assert_eq!(std::fs::read(&list_path).unwrap(), b"not json");
        assert!(drain(&mut rx)
            .iter()
            .any(|e| matches!(e, CalendarSyncEvent::SyncFailed { .. })));
    }

    /// A record the sync engine delivered is not a local edit. Its stamps
    /// must not be newer than the sync time the tick reports, or the next
    /// remote change to it is mistaken for a conflict with local intent.
    #[tokio::test]
    async fn synced_records_are_not_newer_than_the_sync_that_delivered_them() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(one_event_page("tok-1"))
            .mount(&f.server)
            .await;

        let mut rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        let events = drain(&mut rx);
        let synced = events
            .iter()
            .find_map(|e| match e {
                CalendarSyncEvent::EventUpserted { event } => Some(event.as_ref().clone()),
                _ => None,
            })
            .expect("an upsert");
        let completed_at = events
            .iter()
            .find_map(|e| match e {
                CalendarSyncEvent::SyncCompleted { at_ms, .. } => {
                    Some(EpochMs::saturating_from_f64(*at_ms).as_millis())
                }
                _ => None,
            })
            .expect("a completed sync");
        assert!(
            synced.updated_at <= completed_at,
            "{} > {completed_at}",
            synced.updated_at
        );
        assert!(synced.created_at <= completed_at);

        let mut remote = synced.clone();
        remote.start_at_ms += 3_600_000.0;
        assert!(
            !bruhs_models::should_defer_remote_event(&synced, &remote, Some(completed_at)),
            "an untouched synced event must take the remote change"
        );
    }

    /// A calendar that failed must leave the failure visible; a trailing
    /// SyncCompleted would clear the error and report a stale success.
    #[tokio::test]
    async fn a_failed_calendar_does_not_report_a_completed_sync() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(ResponseTemplate::new(503).set_body_string("backend unavailable"))
            .mount(&f.server)
            .await;

        let mut rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        let events = drain(&mut rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, CalendarSyncEvent::SyncFailed { message, .. } if message.contains("503"))));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, CalendarSyncEvent::SyncCompleted { .. })),
            "{events:?}"
        );
        assert_eq!(stored_token(&f).await, None);
    }

    /// The provider boundary is read-only: a sync issues only GET requests
    /// to the Calendar API, never an insert, patch, update or delete.
    #[tokio::test]
    async fn a_sync_only_reads_from_the_provider() {
        let f = fixture().await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .respond_with(one_event_page("tok-1"))
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/users/me/calendarList"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": [{ "id": "primary", "summary": "Me", "primary": true }]
            })))
            .mount(&f.server)
            .await;

        let _rx = f.engine.subscribe();
        f.engine.refresh_calendars(&f.client).await;
        f.engine.run_tick(&f.cfg, &f.client).await;
        f.engine.force_full_sync();
        f.engine.run_tick(&f.cfg, &f.client).await;
        let requests = f.server.received_requests().await.unwrap();
        assert!(requests.len() >= 3, "{}", requests.len());
        for request in requests {
            assert_eq!(request.method.as_str(), "GET", "{}", request.url);
        }
    }

    async fn save_token_cursor(f: &Fixture, window_listed_at_ms: Option<i64>) {
        f.engine
            .files
            .save_cursor(
                PRIMARY_CALENDAR,
                SyncCursor {
                    sync_token: Some("tok-old".into()),
                    last_synced_at_ms: Some(1),
                    window_listed_at_ms,
                },
            )
            .await
            .unwrap();
    }

    fn empty_changes(next: &str) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .set_body_json(serde_json::json!({ "items": [], "nextSyncToken": next }))
    }

    async fn stored_cursor(f: &Fixture) -> SyncCursor {
        f.engine.files.load_cursor(PRIMARY_CALENDAR).await.unwrap()
    }

    /// CAL-12: a sync token only reports changes, so instances of an
    /// unchanged recurring series that slide into the window would never
    /// arrive. A day-old window is listed again, and the incremental token
    /// chain continues from the changes listing, not the window's token.
    #[tokio::test]
    async fn a_stale_window_is_listed_again_without_breaking_the_token_chain() {
        let f = fixture().await;
        let day_ms = 86_400_000;
        save_token_cursor(&f, Some(Utc::now().timestamp_millis() - 2 * day_ms)).await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("syncToken", "tok-old"))
            .respond_with(empty_changes("tok-2"))
            .expect(1)
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param_is_missing("syncToken"))
            .respond_with(one_event_page("tok-window"))
            .expect(1)
            .mount(&f.server)
            .await;

        let mut rx = f.engine.subscribe();
        let before = Utc::now().timestamp_millis();
        f.engine.run_tick(&f.cfg, &f.client).await;
        let events = drain(&mut rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, CalendarSyncEvent::EventUpserted { .. })),
            "a day-old window was not listed again: {events:?}"
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, CalendarSyncEvent::SyncCompleted { upserted: 1, .. })));
        let cursor = stored_cursor(&f).await;
        assert_eq!(cursor.sync_token.as_deref(), Some("tok-2"));
        assert!(cursor.window_listed_at_ms.is_some_and(|at| at >= before));

        // The window request covers the month grid ahead.
        let requests = f.server.received_requests().await.unwrap();
        let window = requests
            .iter()
            .find(|r| !r.url.query().unwrap_or_default().contains("syncToken"))
            .expect("a window request");
        let time_max = window
            .url
            .query_pairs()
            .find(|(k, _)| k == "timeMax")
            .map(|(_, v)| chrono::DateTime::parse_from_rfc3339(&v).unwrap())
            .expect("timeMax");
        assert!(
            time_max.timestamp_millis() >= before + (SYNC_WINDOW_FUTURE_DAYS - 1) * day_ms,
            "{time_max}"
        );
    }

    /// A window listed within the last day is not listed again.
    #[tokio::test]
    async fn a_fresh_window_continues_by_sync_token_only() {
        let f = fixture().await;
        let listed_at = Utc::now().timestamp_millis() - 3_600_000;
        save_token_cursor(&f, Some(listed_at)).await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("syncToken", "tok-old"))
            .respond_with(empty_changes("tok-2"))
            .expect(1)
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param_is_missing("syncToken"))
            .respond_with(one_event_page("tok-window"))
            .expect(0)
            .mount(&f.server)
            .await;

        let _rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        let cursor = stored_cursor(&f).await;
        assert_eq!(cursor.sync_token.as_deref(), Some("tok-2"));
        assert_eq!(cursor.window_listed_at_ms, Some(listed_at));
    }

    /// A cursor saved before the sliding window has no listing time, so the
    /// first tick after an upgrade lists the window once.
    #[tokio::test]
    async fn a_cursor_from_before_the_sliding_window_lists_the_window_once() {
        let f = fixture().await;
        let cursors = f.dir.path().join("cursors");
        save_token_cursor(&f, None).await;
        let files: Vec<_> = std::fs::read_dir(&cursors)
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(files.len(), 1);
        // The shape written by older builds.
        std::fs::write(
            files[0].path(),
            r#"{ "sync_token": "tok-old", "last_synced_at_ms": 1 }"#,
        )
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("syncToken", "tok-old"))
            .respond_with(empty_changes("tok-2"))
            .expect(1)
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("syncToken", "tok-2"))
            .respond_with(empty_changes("tok-3"))
            .expect(1)
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param_is_missing("syncToken"))
            .respond_with(one_event_page("tok-window"))
            .expect(1)
            .mount(&f.server)
            .await;

        let _rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert_eq!(stored_cursor(&f).await.sync_token.as_deref(), Some("tok-3"));
        let window_requests = f
            .server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| !r.url.query().unwrap_or_default().contains("syncToken"))
            .count();
        assert_eq!(
            window_requests, 1,
            "a cursor from before the sliding window lists the window once"
        );
    }

    /// A window listing nobody heard is listed again on the next tick.
    #[tokio::test]
    async fn an_undelivered_window_listing_is_retried() {
        let f = fixture().await;
        save_token_cursor(&f, None).await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param("syncToken", "tok-old"))
            .respond_with(empty_changes("tok-old"))
            .mount(&f.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/calendars/primary/events"))
            .and(query_param_is_missing("syncToken"))
            .respond_with(one_event_page("tok-window"))
            .expect(2)
            .mount(&f.server)
            .await;

        f.engine.run_tick(&f.cfg, &f.client).await; // nobody listening
        assert_eq!(stored_cursor(&f).await.window_listed_at_ms, None);
        let _rx = f.engine.subscribe();
        f.engine.run_tick(&f.cfg, &f.client).await;
        assert!(
            stored_cursor(&f).await.window_listed_at_ms.is_some(),
            "a delivered window listing records its time"
        );
    }

    #[tokio::test]
    async fn without_tokens_a_tick_completes_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let engine = SyncEngine::new(TokenStore::in_memory(), CalendarFiles::at(dir.path()));
        let mut rx = engine.subscribe();
        engine
            .run_tick(
                &OAuthConfig::for_test(&server.uri()),
                &CalendarClient::with_base(server.uri()),
            )
            .await;
        assert!(matches!(
            drain(&mut rx).as_slice(),
            [
                CalendarSyncEvent::SyncStarted { .. },
                CalendarSyncEvent::SyncCompleted { upserted: 0, .. }
            ]
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::CalendarSyncEvent;

    fn engine() -> SyncEngine {
        SyncEngine::new(
            TokenStore::in_memory(),
            CalendarFiles::at(std::env::temp_dir().join("bruhs-unused")),
        )
    }

    #[tokio::test]
    async fn subscribe_then_emit_receives() {
        let engine = engine();
        let mut rx = engine.subscribe();
        assert!(engine.emit(CalendarSyncEvent::SyncStarted { at_ms: 1.0 }));
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, CalendarSyncEvent::SyncStarted { .. }));
    }

    #[tokio::test]
    async fn emit_with_no_subscribers_reports_undelivered() {
        let engine = engine();
        assert!(!engine.emit(CalendarSyncEvent::Disconnected));
    }

    #[tokio::test]
    async fn force_sync_signals_handle() {
        let engine = engine();
        engine.force_sync();
        engine.force.notified().await;
    }
}
