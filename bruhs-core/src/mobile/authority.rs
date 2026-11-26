//! One-use pairing, device scopes, durable command high-water marks and immediate revocation.
use super::{MobileError, MobileResult, MAX_DEVICE_LIFETIME, PAIRING_LIFETIME};
use base64::Engine;
use bruhs_models::mobile::*;
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::File,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, OnceLock,
    },
    time::Duration,
};
use tokio::sync::watch;
use uuid::Uuid;
use zeroize::Zeroizing;

mod storage;
#[cfg(test)]
mod tests;

pub trait MobileClock: Send + Sync {
    fn now_ms(&self) -> i64;
}
struct WallClock;
impl MobileClock for WallClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}

struct LaunchState {
    id: Uuid,
    failure: Arc<LaunchFailure>,
}
struct LaunchFailure {
    poisoned: AtomicBool,
    changed: watch::Sender<bool>,
}
#[derive(Clone)]
pub struct MobileLaunch(Arc<LaunchState>);
impl MobileLaunch {
    /// All windows in one process share the fail-closed launch boundary.
    pub fn process() -> Self {
        static LAUNCH: OnceLock<MobileLaunch> = OnceLock::new();
        LAUNCH.get_or_init(Self::isolated).clone()
    }
    /// Isolated native/test host, without sharing another desktop's failure state.
    pub fn isolated() -> Self {
        let (changed, _) = watch::channel(false);
        Self(Arc::new(LaunchState {
            id: Uuid::new_v4(),
            failure: Arc::new(LaunchFailure {
                poisoned: AtomicBool::new(false),
                changed,
            }),
        }))
    }
    /// A window keeps this identity across listener restarts. Other windows cannot inherit devices.
    pub fn process_owner() -> Self {
        Self(Arc::new(LaunchState {
            id: Uuid::new_v4(),
            failure: Self::process().0.failure.clone(),
        }))
    }
    fn require(&self) -> MobileResult<()> {
        if self.0.failure.poisoned.load(Ordering::Acquire) {
            Err(MobileError::Storage)
        } else {
            Ok(())
        }
    }
    fn poison(&self) {
        self.0.failure.poisoned.store(true, Ordering::Release);
        self.0.failure.changed.send_replace(true);
    }
    pub fn failures(&self) -> watch::Receiver<bool> {
        self.0.failure.changed.subscribe()
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceRow {
    device: MobileDevice,
    token_digest: [u8; 32],
    launch_id: Uuid,
    last: Option<CommandRow>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandRow {
    id: Uuid,
    sequence: u64,
    digest: [u8; 32],
    outcome: MobileCommandOutcome,
}
impl CommandRow {
    fn receipt(&self) -> MobileCommandReceipt {
        MobileCommandReceipt {
            id: self.id,
            sequence: self.sequence,
            outcome: self.outcome.clone(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    version: u32,
    devices: BTreeMap<Uuid, DeviceRow>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            version: 1,
            devices: BTreeMap::new(),
        }
    }
}
struct Pairing {
    id: Uuid,
    digest: [u8; 32],
    expires: i64,
    device_lifetime_ms: i64,
    scopes: MobileScopes,
}
struct State {
    catalog: Catalog,
    pairing: Option<Pairing>,
    revocations: u64,
    persisted_revocations: u64,
    inflight: BTreeMap<Uuid, (Uuid, u64)>,
}
struct Inner {
    state: Mutex<State>,
    path: PathBuf,
    owner: Mutex<Option<File>>,
    writer: Arc<tokio::sync::Mutex<()>>,
    launch: MobileLaunch,
    clock: Arc<dyn MobileClock>,
    active: AtomicBool,
    changed: watch::Sender<u64>,
}
impl Drop for Inner {
    fn drop(&mut self) {
        let unflushed = match self.state.get_mut() {
            Ok(state) => state.revocations != state.persisted_revocations,
            Err(_) => true,
        };
        if unflushed {
            self.launch.poison();
        }
        let owner = self
            .owner
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if owner.is_some_and(|file| file.unlock().is_err()) {
            self.launch.poison();
        }
    }
}
struct WriteFailure(MobileLaunch, bool);
impl Drop for WriteFailure {
    fn drop(&mut self) {
        if !self.1 {
            self.0.poison();
        }
    }
}
#[derive(Clone)]
pub struct MobileAuthority(Arc<Inner>);
/// No token or model-supplied scope travels into the native queue.
#[derive(Clone)]
pub struct MobileLease {
    authority: MobileAuthority,
    device_id: Uuid,
    launch_id: Uuid,
}
/// Private owner-only link. Neither Debug nor Serialize can reveal its secret.
pub struct MobilePairingOffer {
    pub id: Uuid,
    pub url: Zeroizing<String>,
    pub expires_at_ms: i64,
}
pub enum MobileReservation {
    Existing(MobileCommandReceipt),
    New(MobileCommandTicket),
}
#[derive(Clone)]
pub struct MobileCommandTicket(Arc<Ticket>);
struct Ticket {
    authority: MobileAuthority,
    device_id: Uuid,
    id: Uuid,
    sequence: u64,
    digest: [u8; 32],
    command: MobileCommand,
    receipt: Mutex<Option<MobileCommandReceipt>>,
}
impl MobileCommandTicket {
    pub fn action(&self) -> &MobileAction {
        &self.0.command.action
    }
    pub fn id(&self) -> Uuid {
        self.0.id
    }
    pub fn sequence(&self) -> u64 {
        self.0.sequence
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        if let Ok(mut state) = self.authority.state() {
            if state.inflight.get(&self.device_id) == Some(&(self.id, self.sequence)) {
                state.inflight.remove(&self.device_id);
            }
            if let Some(last) = state
                .catalog
                .devices
                .get_mut(&self.device_id)
                .and_then(|row| row.last.as_mut())
                .filter(|last| {
                    last.id == self.id
                        && last.sequence == self.sequence
                        && matches!(last.outcome, MobileCommandOutcome::Pending)
                })
            {
                last.outcome = MobileCommandOutcome::Unconfirmed;
            }
        }
    }
}
impl MobileAuthority {
    pub async fn open(path: PathBuf, launch: MobileLaunch) -> MobileResult<Self> {
        Self::open_with_clock(path, launch, Arc::new(WallClock)).await
    }
    pub async fn open_with_clock(
        path: PathBuf,
        launch: MobileLaunch,
        clock: Arc<dyn MobileClock>,
    ) -> MobileResult<Self> {
        launch.require()?;
        let source = path.clone();
        let (mut catalog, owner) = tokio::task::spawn_blocking(move || storage::open(&source))
            .await
            .map_err(|_| {
                launch.poison();
                MobileError::Storage
            })?
            .map_err(|error| {
                if matches!(error, MobileError::Storage) {
                    launch.poison();
                }
                error
            })?;
        for row in catalog.devices.values_mut() {
            if row.launch_id != launch.0.id {
                row.device.revoked = true;
            }
            if let Some(last) = &mut row.last {
                if matches!(last.outcome, MobileCommandOutcome::Pending) {
                    last.outcome = MobileCommandOutcome::Unconfirmed;
                }
            }
        }
        let (changed, _) = watch::channel(0);
        Ok(Self(Arc::new(Inner {
            state: Mutex::new(State {
                catalog,
                pairing: None,
                revocations: 0,
                persisted_revocations: 0,
                inflight: BTreeMap::new(),
            }),
            path,
            owner: Mutex::new(Some(owner)),
            writer: Arc::new(tokio::sync::Mutex::new(())),
            launch,
            clock,
            active: AtomicBool::new(true),
            changed,
        })))
    }
    fn state(&self) -> MobileResult<MutexGuard<'_, State>> {
        self.0.state.lock().map_err(|_| {
            self.0.launch.poison();
            MobileError::Storage
        })
    }
    fn require_active(&self) -> MobileResult<()> {
        self.0.launch.require()?;
        if self.0.active.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(MobileError::Disabled)
        }
    }
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.0.changed.subscribe()
    }
    pub fn failures(&self) -> watch::Receiver<bool> {
        self.0.launch.failures()
    }
    fn changed(&self) {
        self.0
            .changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    pub fn disable(&self) {
        if let Ok(mut state) = self.state() {
            self.0.active.store(false, Ordering::Release);
            state.pairing = None;
        } else {
            self.0.active.store(false, Ordering::Release);
        }
        self.changed();
    }
    pub async fn release(&self) {
        self.disable();
        let _writer = self.0.writer.lock().await;
        if self
            .state()
            .is_ok_and(|state| state.revocations != state.persisted_revocations)
        {
            self.0.launch.poison();
        }
        if let Ok(mut owner) = self.0.owner.lock() {
            let file = owner.take();
            drop(owner);
            if file.is_some_and(|file| file.unlock().is_err()) {
                self.0.launch.poison();
            }
        }
    }
    pub fn healthy(&self) -> MobileResult<()> {
        self.require_active()
    }
    pub fn begin_pairing(
        &self,
        origin: &url::Url,
        scopes: MobileScopes,
        lifetime: Duration,
    ) -> MobileResult<MobilePairingOffer> {
        self.require_active()?;
        if !scopes.contains(&MobileScope::Read)
            || lifetime.is_zero()
            || duration_ms(lifetime)? == 0
            || lifetime > MAX_DEVICE_LIFETIME
        {
            return Err(MobileError::Invalid(
                "Choose Read chats and a positive device lifetime of at most one day.".into(),
            ));
        }
        if origin.scheme() != "https"
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
        {
            return Err(MobileError::Invalid("Use a complete trusted HTTPS origin without credentials, a path, query or fragment.".into()));
        }
        let secret = random_token()?;
        let now = self.0.clock.now_ms();
        let expires = now
            .checked_add(duration_ms(PAIRING_LIFETIME)?)
            .ok_or_else(|| {
                MobileError::Invalid("The desktop clock is outside the supported range.".into())
            })?;
        let id = Uuid::new_v4();
        self.state()?.pairing = Some(Pairing {
            id,
            digest: hash(secret.as_bytes()),
            expires,
            device_lifetime_ms: duration_ms(lifetime)?,
            scopes,
        });
        let mut link = origin.clone();
        link.set_fragment(Some(&format!("pair={}", secret.as_str())));
        self.changed();
        Ok(MobilePairingOffer {
            id,
            url: Zeroizing::new(link.into()),
            expires_at_ms: expires,
        })
    }
    pub fn cancel_pairing(&self, id: Uuid) {
        if let Ok(mut state) = self.state() {
            if state.pairing.as_ref().is_some_and(|pair| pair.id == id) {
                state.pairing = None;
            }
        }
        self.changed();
    }
    pub fn pairing_pending(&self, id: Uuid) -> bool {
        self.require_active().is_ok()
            && self.state().is_ok_and(|state| {
                state
                    .pairing
                    .as_ref()
                    .is_some_and(|pair| pair.id == id && pair.expires > self.0.clock.now_ms())
            })
    }
    pub async fn pair(&self, input: MobilePairInput) -> MobileResult<MobilePairResult> {
        self.require_active()?;
        validate_name(&input.name)?;
        let digest = token_digest(&input.secret)?;
        let raw = random_token()?;
        let now = self.0.clock.now_ms();
        let device = {
            let mut state = self.state()?;
            let pair = state
                .pairing
                .as_ref()
                .filter(|pair| pair.expires > now && same_hash(&pair.digest, &digest))
                .ok_or(MobileError::Unauthorized)?;
            let scopes = pair.scopes.clone();
            let expires = now
                .checked_add(pair.device_lifetime_ms)
                .ok_or(MobileError::Unauthorized)?;
            if state.catalog.devices.len() >= PAGE_SIZE {
                let removable = state
                    .catalog
                    .devices
                    .iter()
                    .find(|(_, row)| {
                        row.device.revoked
                            || row.device.expires_at_ms <= now
                            || row.launch_id != self.0.launch.0.id
                    })
                    .map(|(id, _)| *id);
                if let Some(id) = removable {
                    state.catalog.devices.remove(&id);
                } else {
                    return Err(MobileError::Conflict("The device list is full. Revoke an unused device on the laptop before pairing another.".into()));
                }
            }
            state.pairing.take();
            let device = MobileDevice {
                id: Uuid::new_v4(),
                name: input.name,
                scopes,
                expires_at_ms: expires,
                last_seen_ms: Some(now),
                revoked: false,
            };
            state.catalog.devices.insert(
                device.id,
                DeviceRow {
                    device: device.clone(),
                    token_digest: hash(raw.as_bytes()),
                    launch_id: self.0.launch.0.id,
                    last: None,
                },
            );
            device
        };
        self.persist().await?;
        self.require_active()?;
        self.changed();
        Ok(MobilePairResult {
            token: raw.to_string(),
            device,
            next_sequence: 1,
        })
    }
    pub fn authenticate(&self, token: &str) -> MobileResult<MobileLease> {
        self.require_active()?;
        let digest = token_digest(token)?;
        let now = self.0.clock.now_ms();
        let mut state = self.state()?;
        let row = state
            .catalog
            .devices
            .values_mut()
            .find(|row| same_hash(&row.token_digest, &digest))
            .ok_or(MobileError::Unauthorized)?;
        require_row(row, self.0.launch.0.id, now)?;
        row.device.last_seen_ms = Some(now);
        let device_id = row.device.id;
        drop(state);
        Ok(MobileLease {
            authority: self.clone(),
            device_id,
            launch_id: self.0.launch.0.id,
        })
    }
    pub fn devices(&self) -> MobileResult<Vec<MobileDevice>> {
        self.require_active()?;
        Ok(self
            .state()?
            .catalog
            .devices
            .values()
            .map(|row| row.device.clone())
            .collect())
    }
    /// Deny synchronously on the native thread; durability is awaited off that thread.
    pub fn revoke_now(&self, id: Uuid) -> MobileResult<()> {
        self.require_active()?;
        let mut state = self.state()?;
        let row = state
            .catalog
            .devices
            .get_mut(&id)
            .ok_or(MobileError::Unauthorized)?;
        if !row.device.revoked {
            row.device.revoked = true;
            state.revocations = state.revocations.checked_add(1).ok_or_else(|| {
                self.0.launch.poison();
                MobileError::Storage
            })?;
        }
        drop(state);
        self.changed();
        Ok(())
    }
    pub async fn persist_revocation(&self) -> MobileResult<()> {
        self.persist().await
    }
    pub async fn disconnect(&self, lease: &MobileLease) -> MobileResult<()> {
        lease.check(MobileScope::Read)?;
        self.revoke_now(lease.device_id)?;
        self.persist().await
    }
    pub fn session(&self, lease: &MobileLease) -> MobileResult<MobileSessionInfo> {
        self.require_active()?;
        let state = self.state()?;
        let row = self.row(&state, lease)?;
        let next = row
            .last
            .as_ref()
            .map_or(Some(1), |last| last.sequence.checked_add(1))
            .ok_or_else(|| {
                MobileError::Conflict(
                    "This device command sequence is exhausted. Pair again.".into(),
                )
            })?;
        let session = MobileSessionInfo {
            device: row.device.clone(),
            next_sequence: next,
            last_command: row.last.as_ref().map(CommandRow::receipt),
        };
        drop(state);
        Ok(session)
    }
    fn row<'a>(&self, state: &'a State, lease: &MobileLease) -> MobileResult<&'a DeviceRow> {
        self.require_active()?;
        if !Arc::ptr_eq(&self.0, &lease.authority.0) || lease.launch_id != self.0.launch.0.id {
            return Err(MobileError::Unauthorized);
        }
        let row = state
            .catalog
            .devices
            .get(&lease.device_id)
            .ok_or(MobileError::Unauthorized)?;
        require_row(row, lease.launch_id, self.0.clock.now_ms())?;
        Ok(row)
    }
    pub async fn reserve(
        &self,
        lease: &MobileLease,
        command: &MobileCommand,
    ) -> MobileResult<MobileReservation> {
        validate_command(command)?;
        self.require_active()?;
        let encoded = serde_json::to_vec(command)
            .map_err(|_| MobileError::Invalid("Invalid mobile command.".into()))?;
        if encoded.len() > REQUEST_BYTES {
            return Err(MobileError::Invalid(
                "The mobile command exceeds 64 KiB. Shorten its prompt.".into(),
            ));
        }
        let digest = hash(&encoded);
        {
            let mut state = self.state()?;
            let row = self.row(&state, lease)?;
            if !row.device.scopes.contains(&command.action.scope()) {
                return Err(MobileError::Forbidden);
            }
            if let Some(last) = &row.last {
                if command.sequence == last.sequence
                    && command.id == last.id
                    && same_hash(&last.digest, &digest)
                {
                    return Ok(MobileReservation::Existing(last.receipt()));
                }
                if command.sequence <= last.sequence {
                    return Err(MobileError::Conflict("This command sequence was already consumed. Read the device session before sending another command.".into()));
                }
                if matches!(last.outcome, MobileCommandOutcome::Pending) {
                    return Err(MobileError::Conflict("A command is still pending on this device. Check its delivery before sending another.".into()));
                }
            }
            if state.inflight.contains_key(&lease.device_id) {
                return Err(MobileError::Conflict("The previous command is still being saved. Check its delivery before sending another.".into()));
            }
            let next = row
                .last
                .as_ref()
                .map_or(Some(1), |last| last.sequence.checked_add(1));
            if next != Some(command.sequence) {
                return Err(MobileError::Conflict(
                    "The command sequence changed. Read the device session before retrying.".into(),
                ));
            }
            state
                .catalog
                .devices
                .get_mut(&lease.device_id)
                .ok_or(MobileError::Unauthorized)?
                .last = Some(CommandRow {
                id: command.id,
                sequence: command.sequence,
                digest,
                outcome: MobileCommandOutcome::Pending,
            });
            state
                .inflight
                .insert(lease.device_id, (command.id, command.sequence));
        }
        let ticket = MobileCommandTicket(Arc::new(Ticket {
            authority: self.clone(),
            device_id: lease.device_id,
            id: command.id,
            sequence: command.sequence,
            digest,
            command: command.clone(),
            receipt: Mutex::new(None),
        }));
        self.persist().await?;
        lease.check(command.action.scope())?;
        Ok(MobileReservation::New(ticket))
    }
    /// Checks and accepts the exact queued operation under the same lock as revocation.
    pub fn commit(
        &self,
        lease: &MobileLease,
        ticket: &MobileCommandTicket,
        operation: impl FnOnce(&MobileAction) -> MobileResult<MobileCommandOutcome>,
    ) -> MobileResult<MobileCommandReceipt> {
        self.require_active()?;
        let mut state = self.state()?;
        let row = self.row(&state, lease)?;
        if !row.device.scopes.contains(&ticket.action().scope())
            || !Arc::ptr_eq(&self.0, &ticket.0.authority.0)
            || ticket.0.device_id != lease.device_id
        {
            return Err(MobileError::Forbidden);
        }
        let last = row
            .last
            .as_ref()
            .filter(|last| {
                last.id == ticket.0.id
                    && last.sequence == ticket.0.sequence
                    && same_hash(&last.digest, &ticket.0.digest)
                    && matches!(last.outcome, MobileCommandOutcome::Pending)
            })
            .ok_or_else(|| {
                MobileError::Conflict("This command has already been accepted or withdrawn.".into())
            })?;
        let _ = last;
        let outcome = match operation(ticket.action()) {
            Ok(outcome) => outcome,
            Err(error) => MobileCommandOutcome::Rejected {
                reason: error.to_string(),
            },
        };
        if matches!(outcome, MobileCommandOutcome::Pending) {
            return Err(MobileError::Invalid(
                "Native dispatch must finish with an accepted or rejected receipt.".into(),
            ));
        }
        let last = state
            .catalog
            .devices
            .get_mut(&lease.device_id)
            .and_then(|row| row.last.as_mut())
            .ok_or(MobileError::Unauthorized)?;
        last.outcome = outcome;
        let receipt = last.receipt();
        *ticket.0.receipt.lock().map_err(|_| MobileError::Storage)? = Some(receipt.clone());
        drop(state);
        self.changed();
        Ok(receipt)
    }
    pub async fn finish(
        &self,
        lease: &MobileLease,
        ticket: &MobileCommandTicket,
    ) -> MobileResult<MobileCommandReceipt> {
        if !Arc::ptr_eq(&self.0, &ticket.0.authority.0)
            || ticket.0.device_id != lease.device_id
            || !Arc::ptr_eq(&self.0, &lease.authority.0)
        {
            return Err(MobileError::Forbidden);
        }
        let receipt = ticket
            .0
            .receipt
            .lock()
            .map_err(|_| MobileError::Storage)?
            .clone()
            .ok_or_else(|| {
                MobileError::Conflict(
                    "Native delivery has not completed. Check the same command again.".into(),
                )
            })?;
        self.persist().await?;
        lease.check(ticket.action().scope())?;
        let mut state = self.state()?;
        if state.inflight.get(&lease.device_id) == Some(&(ticket.id(), ticket.sequence())) {
            state.inflight.remove(&lease.device_id);
        }
        drop(state);
        Ok(receipt)
    }
    pub async fn persist(&self) -> MobileResult<()> {
        self.require_active()?;
        let writer = Arc::clone(&self.0.writer).lock_owned().await;
        self.require_active()?;
        let (catalog, revocations) = {
            let state = self.state()?;
            (state.catalog.clone(), state.revocations)
        };
        let path = self.0.path.clone();
        let authority = self.clone();
        // Cancellation cannot release serialization while a blocking atomic write still runs.
        tokio::task::spawn_blocking(move || {
            let _writer = writer;
            let mut failure = WriteFailure(authority.0.launch.clone(), false);
            let result = storage::save(&path, &catalog);
            if result.is_err() {
                authority.0.launch.poison();
                authority.changed();
            }
            if result.is_ok() {
                authority.state()?.persisted_revocations = revocations;
            }
            failure.1 = true;
            result
        })
        .await
        .map_err(|_| {
            self.0.launch.poison();
            MobileError::Storage
        })
        .and_then(|result| result)
    }
}
impl MobileLease {
    pub fn id(&self) -> Uuid {
        self.device_id
    }
    pub fn authority(&self) -> MobileAuthority {
        self.authority.clone()
    }
    pub fn check(&self, scope: MobileScope) -> MobileResult<MobileDevice> {
        self.with_scope(scope, || ())
            .and_then(|()| self.authority.session(self).map(|session| session.device))
    }
    /// Native reads/actions use this immediately before touching desktop-owned state.
    pub fn with_scope<R>(
        &self,
        scope: MobileScope,
        operation: impl FnOnce() -> R,
    ) -> MobileResult<R> {
        self.authority.require_active()?;
        let state = self.authority.state()?;
        let row = self.authority.row(&state, self)?;
        if !row.device.scopes.contains(&scope) {
            return Err(MobileError::Forbidden);
        }
        let result = operation();
        drop(state);
        Ok(result)
    }
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.authority.changes()
    }
    pub fn failures(&self) -> watch::Receiver<bool> {
        self.authority.failures()
    }
    pub fn remaining(&self) -> MobileResult<Duration> {
        let session = self.authority.session(self)?;
        let remaining = session
            .device
            .expires_at_ms
            .saturating_sub(self.authority.0.clock.now_ms());
        if remaining <= 0 {
            return Err(MobileError::Unauthorized);
        }
        Ok(Duration::from_millis(
            u64::try_from(remaining).map_err(|_| MobileError::Unauthorized)?,
        ))
    }
}
fn require_row(row: &DeviceRow, launch: Uuid, now: i64) -> MobileResult<()> {
    if row.launch_id != launch || row.device.revoked || row.device.expires_at_ms <= now {
        Err(MobileError::Unauthorized)
    } else {
        Ok(())
    }
}
fn validate_name(name: &str) -> MobileResult<()> {
    if name.trim().is_empty()
        || name.len() > 128
        || bruhs_models::secrets::review_text(name) != name
    {
        return Err(MobileError::Invalid(
            "Use a visible device name of at most 128 bytes without formatting controls.".into(),
        ));
    }
    Ok(())
}
fn validate_command(command: &MobileCommand) -> MobileResult<()> {
    if command.sequence == 0 {
        return Err(MobileError::Invalid(
            "Use the next device command sequence.".into(),
        ));
    }
    let thread = match &command.action {
        MobileAction::OpenLinkedDraft { target } => {
            return target
                .destination()
                .map(drop)
                .map_err(|error| MobileError::Invalid(error.to_string()));
        }
        MobileAction::ContinueLinkedDraft { .. } => return Ok(()),
        MobileAction::Prompt { thread_id, text } => {
            if text.trim().is_empty()
                || text.len() > RESPONSE_BYTES
                || text.trim_start().starts_with('/')
            {
                return Err(MobileError::Invalid("Send a plain prompt of at most 32 KiB. Desktop slash commands stay on the laptop.".into()));
            }
            thread_id
        }
        MobileAction::Permission {
            thread_id,
            request_id,
            review_hash,
            ..
        }
        | MobileAction::AnswerQuestion {
            thread_id,
            request_id,
            review_hash,
            ..
        } => {
            if request_id.is_empty()
                || request_id.len() > 128
                || review_hash.len() != 64
                || !review_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(MobileError::Invalid(
                    "Use the exact pending request from this thread.".into(),
                ));
            }
            thread_id
        }
    };
    if thread.is_empty() || thread.len() > 128 {
        return Err(MobileError::Invalid(
            "Use an existing thread and the next device command sequence.".into(),
        ));
    }
    Ok(())
}
fn random_token() -> MobileResult<Zeroizing<String>> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    rand::rngs::OsRng
        .try_fill_bytes(bytes.as_mut())
        .map_err(|_| MobileError::Unavailable)?;
    Ok(Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes.as_ref()),
    ))
}
fn token_digest(token: &str) -> MobileResult<[u8; 32]> {
    if token.len() != 43
        || base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token)
            .map_or(true, |bytes| bytes.len() != 32)
    {
        return Err(MobileError::Unauthorized);
    }
    Ok(hash(token.as_bytes()))
}
fn hash(value: &[u8]) -> [u8; 32] {
    Sha256::digest(value).into()
}
fn same_hash(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}
fn duration_ms(value: Duration) -> MobileResult<i64> {
    i64::try_from(value.as_millis())
        .map_err(|_| MobileError::Invalid("Choose a device lifetime of at most one day.".into()))
}
