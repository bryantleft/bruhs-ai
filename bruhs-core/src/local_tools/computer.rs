//! Bruh computers: one persistent headless browser per bruh with Computer
//! on, started on first use, closed when idle, behind its own network guard.
//!
//! A bruh's grant and page live only in its slot. They never enter the
//! host's thread grants or browser sessions, so Verification doesn't list
//! them and the thread routes can't reach them.

use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Duration,
};

use async_trait::async_trait;
use bruhs_models::{
    crew::{
        handed_back_note, ACTION_STOPPED, CLOSED_BY_OWNER, CLOSE_GRACE_MS, COMPUTERS_FULL,
        COMPUTER_IDLE_MS, COMPUTER_OFF_DENIAL, LOCAL_HOST_DENIAL, MAX_COMPUTERS, NOT_FOR_BRUHS,
        OWNER_HAS_COMPUTER, TAKE_CONTROL_FIRST, TURN_ON_IN_SETTINGS, USE_THE_WINDOW,
    },
    local_tools::{
        validate_action, validate_input, BrowserAction, BrowserActionKind, BrowserSession,
        BrowserStart, ComputerInput, ComputerRequest, ComputerState, Control, Principal, ToolGrant,
        ToolReply, ToolScope, Viewport, COMPUTER_VIEWPORT,
    },
};
use futures_util::future::{join_all, BoxFuture};
use tokio::{sync::watch, time::Instant};
use uuid::Uuid;

use super::{
    browser::{Browser, LaunchOptions},
    network::NetworkGuard,
    GrantState, LocalToolsHost, ProcessPolicy, ToolError,
};

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;

/// How often the idle reaper looks for browsers to close.
const REAP_EVERY: Duration = Duration::from_secs(30);
/// How long asking an open page for its address may take.
const PAGE_URL_WAIT: Duration = Duration::from_secs(2);
const ACTION_TIMED_OUT: &str = "Browser action timed out. Check the page and retry.";

/// A bruh whose Computer is on, as the reactive store lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BruhComputer {
    pub agent_id: String,
    pub name: String,
    pub paused: bool,
    /// `<home>/bruhs/<agent_id>/browser`, from the store's agent id.
    pub profile_dir: PathBuf,
}

/// An open bruh page: a real [`Browser`] or a test fake.
#[async_trait]
pub(super) trait ComputerPage: Send {
    fn session(&self) -> &BrowserSession;
    /// The page target's debugging socket, for the live view.
    #[expect(dead_code, reason = "used from Task 7")]
    fn page_socket(&self) -> Option<String>;
    /// The page view in CSS px: [`COMPUTER_VIEWPORT`] headless, the
    /// window's when headed.
    fn viewport(&self) -> Viewport;
    async fn run(
        &mut self,
        action: BrowserAction,
        policy: ProcessPolicy,
    ) -> Result<ToolReply, ToolError>;
    async fn owner_input(&mut self, input: &ComputerInput) -> Result<(), ToolError>;
    async fn url(&mut self) -> Option<String>;
    /// Releases a mouse button an interrupted action left pressed.
    async fn release_pointer(&mut self);
    /// A graceful close: Chrome writes its profile, then exits.
    async fn close(&mut self);
}

#[async_trait]
impl ComputerPage for Browser {
    fn session(&self) -> &BrowserSession {
        &self.info
    }

    fn page_socket(&self) -> Option<String> {
        Some(Browser::page_socket(self).to_owned())
    }

    fn viewport(&self) -> Viewport {
        Browser::viewport(self)
    }

    async fn run(
        &mut self,
        action: BrowserAction,
        policy: ProcessPolicy,
    ) -> Result<ToolReply, ToolError> {
        self.command(action, policy).await
    }

    async fn owner_input(&mut self, input: &ComputerInput) -> Result<(), ToolError> {
        Browser::owner_input(self, input).await
    }

    async fn url(&mut self) -> Option<String> {
        self.current_url().await.ok()
    }

    async fn release_pointer(&mut self) {
        if let Err(error) = Browser::release_pointer(self).await {
            tracing::warn!(%error, "couldn't release the bruh browser's mouse button");
        }
    }

    async fn close(&mut self) {
        if let Err(error) = Browser::close(self, Duration::from_millis(CLOSE_GRACE_MS)).await {
            tracing::warn!(%error, "bruh browser didn't close cleanly");
        }
    }
}

/// What the host asks a launcher to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLaunch {
    pub agent_id: String,
    /// The bruh grant's id, which the session carries.
    pub grant_id: String,
    pub profile_dir: PathBuf,
    pub url: String,
    pub headless: bool,
    /// The browser's network guard.
    pub proxy: SocketAddr,
}

pub(super) type Launcher = Arc<
    dyn Fn(PageLaunch) -> BoxFuture<'static, Result<Box<dyn ComputerPage>, ToolError>>
        + Send
        + Sync,
>;

/// The production launcher: Chrome on the bruh's kept profile.
pub(super) fn browser_launcher(client: reqwest::Client, policy: ProcessPolicy) -> Launcher {
    Arc::new(move |launch: PageLaunch| {
        let client = client.clone();
        Box::pin(async move {
            let options = LaunchOptions {
                profile: Some(launch.profile_dir),
                headless: launch.headless,
                proxy: Some(launch.proxy),
            };
            let request = BrowserStart {
                grant_id: launch.grant_id,
                url: launch.url,
            };
            let browser = Browser::start(&client, request, policy, &options).await?;
            Ok(Box::new(browser) as Box<dyn ComputerPage>)
        })
    })
}

/// Every synced bruh's slot, and the seats that cap open browsers.
#[derive(Default)]
pub(super) struct Computers {
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    /// Grant ids of launching and open pages, at most [`MAX_COMPUTERS`].
    seats: Arc<Mutex<HashSet<String>>>,
}

struct Slot {
    agent_id: String,
    profile_dir: PathBuf,
    grant: GrantState,
    meta: Mutex<Meta>,
    /// Bumped when the bruh is stopped; an in-flight action ends.
    interrupt: watch::Sender<u64>,
    state: watch::Sender<ComputerState>,
    page: tokio::sync::Mutex<PageSlot>,
}

struct Meta {
    name: String,
    paused: bool,
    /// Who drives the browser. While the owner does, agent commands are
    /// refused.
    control: Control,
    /// What the bruh's next browser call learns: the owner handed back or
    /// closed the browser.
    note: Option<String>,
}

#[derive(Default)]
struct PageSlot {
    open: Option<OpenPage>,
    /// Where the browser reopens. In memory only.
    last_url: Option<String>,
    last_used: Option<Instant>,
    error: Option<String>,
    /// The open page is a visible window, not headless.
    headed: bool,
}

/// A running page with the guard its traffic goes through and its seat.
/// Dropping it stops the guard and frees the seat.
struct OpenPage {
    page: Box<dyn ComputerPage>,
    _guard: NetworkGuard,
    seat: Seat,
}

struct Seat {
    seats: Arc<Mutex<HashSet<String>>>,
    id: String,
}

impl Seat {
    fn take(seats: &Arc<Mutex<HashSet<String>>>, id: &str) -> Result<Self, ToolError> {
        let mut taken = seats.lock().unwrap_or_else(PoisonError::into_inner);
        if taken.len() >= MAX_COMPUTERS && !taken.contains(id) {
            return Err(ToolError::Refused(COMPUTERS_FULL.into()));
        }
        taken.insert(id.into());
        drop(taken);
        Ok(Self {
            seats: Arc::clone(seats),
            id: id.into(),
        })
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.seats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl Slot {
    fn new(computer: BruhComputer) -> Self {
        let grant = ToolGrant {
            id: Uuid::new_v4().to_string(),
            principal: Principal::Bruh(computer.agent_id.clone()).wire(),
            scope: ToolScope::Browser,
        };
        let (revoked, _) = watch::channel(false);
        let (state, _) = watch::channel(ComputerState {
            agent_id: computer.agent_id.clone(),
            ..ComputerState::default()
        });
        Self {
            agent_id: computer.agent_id,
            profile_dir: computer.profile_dir,
            grant: GrantState { grant, revoked },
            meta: Mutex::new(Meta {
                name: computer.name,
                paused: computer.paused,
                control: Control::Bruh,
                note: None,
            }),
            interrupt: watch::channel(0).0,
            state,
            page: tokio::sync::Mutex::new(PageSlot::default()),
        }
    }

    fn meta(&self) -> std::sync::MutexGuard<'_, Meta> {
        self.meta.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, computer: &BruhComputer) {
        let mut meta = self.meta();
        meta.name.clone_from(&computer.name);
        let stopped = computer.paused && !meta.paused;
        meta.paused = computer.paused;
        drop(meta);
        if stopped {
            self.interrupt
                .send_modify(|count| *count = count.wrapping_add(1));
        }
    }

    fn paused(&self) -> bool {
        self.meta().paused
    }

    fn control(&self) -> Control {
        self.meta().control
    }

    /// Hand the browser to `control`, published at once, leaving `note` for
    /// the bruh's next browser call when given.
    fn set_control(&self, control: Control, note: Option<String>) {
        let mut meta = self.meta();
        meta.control = control;
        if note.is_some() {
            meta.note = note;
        }
        // Under the meta lock, so a concurrent publish can't send a stale
        // control after this one.
        self.state.send_modify(|state| state.control = control);
        drop(meta);
    }

    fn note(&self) -> Option<String> {
        self.meta().note.clone()
    }

    /// The bruh has read `note`. A newer note stays.
    fn clear_note(&self, note: &str) {
        let mut meta = self.meta();
        if meta.note.as_deref() == Some(note) {
            meta.note = None;
        }
    }

    fn publish(&self, page: &PageSlot) {
        let meta = self.meta();
        self.state.send_modify(|state| {
            state.session_id = page
                .open
                .as_ref()
                .map(|open| open.page.session().id.clone());
            state.url.clone_from(&page.last_url);
            state.error.clone_from(&page.error);
            state.control = meta.control;
            state.headed = page.headed && page.open.is_some();
            state.viewport = page
                .open
                .as_ref()
                .map_or(COMPUTER_VIEWPORT, |open| open.page.viewport());
        });
        drop(meta);
    }

    /// Close the open page gracefully, keeping where it was, and give back
    /// its seat for a relaunch.
    async fn close_page(&self, page: &mut PageSlot) -> Option<Seat> {
        let mut open = page.open.take()?;
        page.headed = false;
        let name = self.meta().name.clone();
        tracing::debug!(bruh = %name, "closing bruh browser");
        // It reopens where it was, which a click may have changed.
        if let Ok(Some(url)) = tokio::time::timeout(PAGE_URL_WAIT, open.page.url()).await {
            page.last_url = Some(url);
        }
        open.page.close().await;
        let OpenPage { seat, .. } = open;
        Some(seat)
    }

    /// Close the page, if open, and publish that.
    async fn shut(&self, page: &mut PageSlot) {
        drop(self.close_page(page).await);
        self.publish(page);
    }

    /// Ask the open page where it is now, else keep the last address.
    async fn refresh_url(&self, page: &mut PageSlot) {
        if let Some(open) = page.open.as_mut() {
            if let Ok(Some(url)) = tokio::time::timeout(PAGE_URL_WAIT, open.page.url()).await {
                page.last_url = Some(url);
            }
        }
    }
}

/// `reply` with `note` beside its value.
fn with_note(mut reply: ToolReply, note: String) -> ToolReply {
    if let serde_json::Value::Object(value) = &mut reply.value {
        value.insert("note".into(), note.into());
    } else {
        let value = std::mem::take(&mut reply.value);
        reply.value = serde_json::json!({"result": value, "note": note});
    }
    reply
}

/// Validation errors as the host reports them: a host on this computer is
/// refused, anything else is invalid.
fn invalid(error: &'static str) -> ToolError {
    if error == LOCAL_HOST_DENIAL {
        ToolError::Refused(error.into())
    } else {
        ToolError::Invalid(error.into())
    }
}

impl LocalToolsHost {
    fn computer_slot(&self, agent_id: &str) -> Option<Arc<Slot>> {
        self.computers
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent_id)
            .cloned()
    }

    fn computer_slots(&self) -> Vec<Arc<Slot>> {
        self.computers
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .cloned()
            .collect()
    }

    /// Adds and updates the listed bruhs' slots, and takes out the rest.
    fn update_slots(&self, computers: Vec<BruhComputer>) -> Vec<Arc<Slot>> {
        let mut slots = self
            .computers
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let listed: HashSet<&str> = computers.iter().map(|c| c.agent_id.as_str()).collect();
        let gone: Vec<String> = slots
            .keys()
            .filter(|id| !listed.contains(id.as_str()))
            .cloned()
            .collect();
        let removed = gone.iter().filter_map(|id| slots.remove(id)).collect();
        for computer in computers {
            match slots.get(&computer.agent_id) {
                Some(slot) => slot.update(&computer),
                None => {
                    slots.insert(computer.agent_id.clone(), Arc::new(Slot::new(computer)));
                }
            }
        }
        drop(slots);
        removed
    }

    /// Make the host's bruh computers match `computers`: new bruhs get a
    /// slot (no browser yet), stopped bruhs' in-flight actions end, and the
    /// browsers of bruhs not listed close. Returns once those have closed.
    pub async fn sync_bruhs(&self, computers: Vec<BruhComputer>) {
        let removed = self.update_slots(computers);
        for slot in &removed {
            slot.grant.revoked.send_replace(true);
        }
        join_all(removed.iter().map(|slot| async move {
            let mut page = slot.page.lock().await;
            slot.shut(&mut page).await;
        }))
        .await;
    }

    pub async fn computer_state(&self, agent_id: &str) -> Result<ComputerState, ToolError> {
        let slot = self
            .computer_slot(agent_id)
            .ok_or_else(|| ToolError::NotFound(TURN_ON_IN_SETTINGS.into()))?;
        let state = slot.state.borrow().clone();
        Ok(state)
    }

    /// The owner opens a bruh's browser.
    pub async fn open_computer(&self, agent_id: &str) -> Result<ComputerState, ToolError> {
        let slot = self
            .computer_slot(agent_id)
            .ok_or_else(|| ToolError::NotFound(TURN_ON_IN_SETTINGS.into()))?;
        let mut page = slot.page.lock().await;
        self.ensure_open(&slot, &mut page).await?;
        page.last_used = Some(Instant::now());
        drop(page);
        let state = slot.state.borrow().clone();
        Ok(state)
    }

    /// The owner closes a bruh's browser. Its profile stays. Closed while
    /// the owner holds it, it goes back to the bruh with a note.
    pub async fn close_computer(&self, agent_id: &str) -> Result<ComputerState, ToolError> {
        let slot = self
            .computer_slot(agent_id)
            .ok_or_else(|| ToolError::NotFound(TURN_ON_IN_SETTINGS.into()))?;
        let mut page = slot.page.lock().await;
        slot.shut(&mut page).await;
        if slot.control() == Control::Owner {
            slot.set_control(Control::Bruh, Some(CLOSED_BY_OWNER.into()));
        }
        drop(page);
        let state = slot.state.borrow().clone();
        Ok(state)
    }

    /// Close every bruh browser, as the server stops.
    /// Nothing opens again afterwards.
    pub async fn close_all(&self) {
        let slots = self.computer_slots();
        for slot in &slots {
            slot.grant.revoked.send_replace(true);
        }
        join_all(slots.iter().map(|slot| async move {
            let mut page = slot.page.lock().await;
            slot.shut(&mut page).await;
        }))
        .await;
    }

    /// One action from a bruh's own browser tool.
    pub async fn bruh_command(
        &self,
        agent_id: &str,
        action: BrowserAction,
    ) -> Result<ToolReply, ToolError> {
        let slot = self
            .computer_slot(agent_id)
            .ok_or_else(|| ToolError::Refused(COMPUTER_OFF_DENIAL.into()))?;
        if !action.for_bruhs() {
            return Err(ToolError::Refused(NOT_FOR_BRUHS.into()));
        }
        // Navigate's check refuses a host on this computer.
        let viewport = slot.state.borrow().viewport;
        validate_action(&action, viewport).map_err(invalid)?;
        if slot.paused() {
            return Err(ToolError::Refused(ACTION_STOPPED.into()));
        }
        if slot.control() == Control::Owner {
            return Err(ToolError::Refused(OWNER_HAS_COMPUTER.into()));
        }
        let mut revoked = slot.grant.revoked.subscribe();
        let mut interrupt = slot.interrupt.subscribe();
        if *revoked.borrow_and_update() {
            return Err(ToolError::Refused(COMPUTER_OFF_DENIAL.into()));
        }
        let work = async {
            let mut page = slot.page.lock().await;
            // The owner may have taken control while this waited.
            if slot.control() == Control::Owner {
                return Err(ToolError::Refused(OWNER_HAS_COMPUTER.into()));
            }
            // The note is cleared only once the bruh has read it: from a
            // refused input or a reply. A failed or cut-off read keeps it.
            let note = slot.note();
            if let Some(note) = note.as_deref() {
                if action.kind() == BrowserActionKind::Input {
                    slot.clear_note(note);
                    return Err(ToolError::Invalid(note.into()));
                }
            }
            self.ensure_open(&slot, &mut page).await?;
            let navigated = match &action {
                BrowserAction::Navigate { url } => Some(url.clone()),
                _ => None,
            };
            let open = page
                .open
                .as_mut()
                .ok_or_else(|| ToolError::Unavailable("The bruh's browser didn't open.".into()))?;
            let reply =
                tokio::time::timeout(self.policy.timeout, open.page.run(action, self.policy))
                    .await
                    .unwrap_or_else(|_| Err(ToolError::Unavailable(ACTION_TIMED_OUT.into())));
            let url = tokio::time::timeout(PAGE_URL_WAIT, open.page.url())
                .await
                .ok()
                .flatten();
            if let Some(url) = url.or(navigated.filter(|_| reply.is_ok())) {
                page.last_url = Some(url);
            }
            page.last_used = Some(Instant::now());
            slot.publish(&page);
            drop(page);
            match (reply, note) {
                (Ok(reply), Some(note)) => {
                    slot.clear_note(&note);
                    Ok(with_note(reply, note))
                }
                (reply, _) => reply,
            }
        };
        // The handlers only name the outcome: `work` (and the page lock it
        // may hold) is dropped when the `select!` ends.
        let outcome = tokio::select! {
            biased;
            _ = revoked.changed() => Err(ToolError::Refused(COMPUTER_OFF_DENIAL.into())),
            _ = interrupt.changed() => Err(ToolError::Refused(ACTION_STOPPED.into())),
            reply = work => Ok(reply),
        };
        match outcome {
            Ok(reply) => reply,
            Err(ToolError::Refused(message)) if message == ACTION_STOPPED => {
                // A click cut off between press and release would leave the
                // button held on a browser that stays open.
                let mut page = slot.page.lock().await;
                if let Some(open) = page.open.as_mut() {
                    let _ = tokio::time::timeout(PAGE_URL_WAIT, open.page.release_pointer()).await;
                }
                drop(page);
                Err(ToolError::Refused(message))
            }
            Err(error) => Err(error),
        }
    }

    /// One of the owner's requests from a bruh's Computer tab.
    pub async fn computer_request(
        &self,
        agent_id: &str,
        request: ComputerRequest,
    ) -> Result<ComputerState, ToolError> {
        let slot = || {
            self.computer_slot(agent_id)
                .ok_or_else(|| ToolError::NotFound(TURN_ON_IN_SETTINGS.into()))
        };
        let state = |slot: &Slot| slot.state.borrow().clone();
        match request {
            ComputerRequest::Open => self.open_computer(agent_id).await,
            ComputerRequest::Close => self.close_computer(agent_id).await,
            ComputerRequest::TakeControl => {
                let slot = slot()?;
                self.take_control(&slot).await?;
                Ok(state(&slot))
            }
            ComputerRequest::HandBack => {
                let slot = slot()?;
                self.hand_back(&slot).await;
                Ok(state(&slot))
            }
            ComputerRequest::OpenWindow => {
                let slot = slot()?;
                self.open_window(&slot).await?;
                Ok(state(&slot))
            }
        }
    }

    /// The owner takes the browser, opening it if it's closed. New agent
    /// commands are refused at once; one already running finishes.
    async fn take_control(&self, slot: &Slot) -> Result<(), ToolError> {
        slot.set_control(Control::Owner, None);
        // An open page isn't locked, so an action in flight isn't waited on.
        if slot.state.borrow().session_id.is_some() {
            return Ok(());
        }
        let mut page = slot.page.lock().await;
        if let Err(error) = self.ensure_open(slot, &mut page).await {
            slot.set_control(Control::Bruh, None);
            return Err(error);
        }
        page.last_used = Some(Instant::now());
        drop(page);
        Ok(())
    }

    /// The owner hands the browser back. A window closes and the page
    /// reopens headless where it was. The bruh's next browser call learns
    /// where that is.
    async fn hand_back(&self, slot: &Slot) {
        let mut page = slot.page.lock().await;
        if slot.control() != Control::Owner {
            return;
        }
        if page.headed {
            // A failed relaunch shows in the state's error; the next agent
            // command opens it again.
            if let Err(error) = self.relaunch(slot, &mut page, true).await {
                tracing::warn!(%error, "bruh browser didn't reopen headless");
            }
        }
        slot.refresh_url(&mut page).await;
        let url = page
            .last_url
            .clone()
            .unwrap_or_else(|| "about:blank".into());
        page.last_used = Some(Instant::now());
        slot.set_control(Control::Bruh, Some(handed_back_note(&url)));
        slot.publish(&page);
        drop(page);
    }

    /// The owner opens the bruh's browser in a window on this computer, for
    /// what the live view can't do: the headless page closes gracefully and
    /// the same profile opens headed where it was.
    async fn open_window(&self, slot: &Slot) -> Result<(), ToolError> {
        slot.set_control(Control::Owner, None);
        let mut page = slot.page.lock().await;
        if page.headed && page.open.is_some() {
            return Ok(());
        }
        let opened = self.relaunch(slot, &mut page, false).await;
        drop(page);
        if opened.is_err() {
            // No window: the bruh keeps its browser, which reopens headless
            // where it was on its next step.
            slot.set_control(Control::Bruh, None);
        }
        opened
    }

    /// One input from the owner's live view. Needs control, a headless page
    /// (a window takes its own input) and a point inside the view.
    pub async fn owner_input(&self, agent_id: &str, input: ComputerInput) -> Result<(), ToolError> {
        let slot = self
            .computer_slot(agent_id)
            .ok_or_else(|| ToolError::NotFound(TURN_ON_IN_SETTINGS.into()))?;
        let usable = |headed: bool| {
            if slot.control() != Control::Owner {
                Err(ToolError::Refused(TAKE_CONTROL_FIRST.into()))
            } else if headed {
                Err(ToolError::Refused(USE_THE_WINDOW.into()))
            } else {
                Ok(())
            }
        };
        let (headed, viewport) = {
            let state = slot.state.borrow();
            (state.headed, state.viewport)
        };
        usable(headed)?;
        validate_input(&input, viewport).map_err(invalid)?;
        let mut page = slot.page.lock().await;
        // Control or the window may have changed while this waited.
        usable(page.headed)?;
        self.ensure_open(&slot, &mut page).await?;
        let open = page
            .open
            .as_mut()
            .ok_or_else(|| ToolError::Unavailable("The bruh's browser didn't open.".into()))?;
        let result = open.page.owner_input(&input).await;
        if matches!(
            input,
            ComputerInput::Navigate { .. } | ComputerInput::Back | ComputerInput::Reload
        ) {
            slot.refresh_url(&mut page).await;
        }
        page.last_used = Some(Instant::now());
        slot.publish(&page);
        drop(page);
        result
    }

    /// The bruh's Computer state as it changes, for runs that wait while
    /// the owner holds it. `None` without a slot.
    pub fn control_changes(&self, agent_id: &str) -> Option<watch::Receiver<ComputerState>> {
        self.computer_slot(agent_id)
            .map(|slot| slot.state.subscribe())
    }

    /// The address the bruh's page is on now: asked of the open page, else
    /// where it was last.
    pub async fn page_url(&self, agent_id: &str) -> Option<String> {
        let slot = self.computer_slot(agent_id)?;
        let asked = tokio::time::timeout(PAGE_URL_WAIT, async {
            let mut page = slot.page.lock().await;
            match page.open.as_mut() {
                Some(open) => open.page.url().await,
                None => None,
            }
        })
        .await
        .ok()
        .flatten();
        asked.or_else(|| slot.state.borrow().url.clone())
    }

    /// Closes bruh browsers idle for [`COMPUTER_IDLE_MS`]. The task ends
    /// with the host.
    pub fn start_idle_reaper(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let host: Weak<Self> = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(REAP_EVERY);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(host) = host.upgrade() else {
                    return;
                };
                host.close_idle().await;
            }
        })
    }

    async fn close_idle(&self) {
        let idle = Duration::from_millis(COMPUTER_IDLE_MS);
        join_all(self.computer_slots().iter().map(|slot| async move {
            // A page in use or held by the owner isn't idle.
            if slot.control() == Control::Owner {
                return;
            }
            let Ok(mut page) = slot.page.try_lock() else {
                return;
            };
            let expired = page.last_used.is_some_and(|used| used.elapsed() >= idle);
            if page.open.is_some() && expired {
                slot.shut(&mut page).await;
            }
        }))
        .await;
    }

    /// Launch the slot's page if it isn't open. A failure lands in the
    /// state's `error`.
    async fn ensure_open(&self, slot: &Slot, page: &mut PageSlot) -> Result<(), ToolError> {
        // A slot that was removed or shut for good never launches again, so
        // it can't recreate a deleted bruh's profile.
        if *slot.grant.revoked.borrow() {
            return Err(ToolError::Refused(COMPUTER_OFF_DENIAL.into()));
        }
        if page.open.is_some() {
            return Ok(());
        }
        let launched = self.launch(slot, page.last_url.clone(), true, None).await;
        Self::opened(slot, page, launched, true)
    }

    /// Close the open page, if any, and open the same profile again where it
    /// was: `headless`, or in a window. The page keeps its seat.
    async fn relaunch(
        &self,
        slot: &Slot,
        page: &mut PageSlot,
        headless: bool,
    ) -> Result<(), ToolError> {
        let seat = slot.close_page(page).await;
        slot.publish(page);
        let launched = if *slot.grant.revoked.borrow() {
            Err(ToolError::Refused(COMPUTER_OFF_DENIAL.into()))
        } else {
            self.launch(slot, page.last_url.clone(), headless, seat)
                .await
        };
        Self::opened(slot, page, launched, headless)
    }

    /// Record a launch's outcome in the slot and publish it.
    fn opened(
        slot: &Slot,
        page: &mut PageSlot,
        launched: Result<OpenPage, ToolError>,
        headless: bool,
    ) -> Result<(), ToolError> {
        let result = match launched {
            Ok(open) => {
                page.open = Some(open);
                page.headed = !headless;
                page.error = None;
                page.last_used = Some(Instant::now());
                Ok(())
            }
            Err(error) => {
                page.error = Some(error.to_string());
                Err(error)
            }
        };
        slot.publish(page);
        result
    }

    /// Open the slot's profile at `url`, on `seat` when a relaunch keeps
    /// one.
    async fn launch(
        &self,
        slot: &Slot,
        url: Option<String>,
        headless: bool,
        seat: Option<Seat>,
    ) -> Result<OpenPage, ToolError> {
        let seat = match seat {
            Some(seat) => seat,
            None => Seat::take(&self.computers.seats, &slot.grant.grant.id)?,
        };
        // One guard per browser, so one page can't starve another bruh's.
        let guard = NetworkGuard::start((self.network)()?)?;
        let url = url
            .filter(|url| {
                validate_action(
                    &BrowserAction::Navigate { url: url.clone() },
                    COMPUTER_VIEWPORT,
                )
                .is_ok()
            })
            .unwrap_or_else(|| "about:blank".into());
        let page = (self.launcher)(PageLaunch {
            agent_id: slot.agent_id.clone(),
            grant_id: slot.grant.grant.id.clone(),
            profile_dir: slot.profile_dir.clone(),
            url,
            headless,
            proxy: guard.addr(),
        })
        .await?;
        Ok(OpenPage {
            page,
            _guard: guard,
            seat,
        })
    }

    /// The bruh grant's principal and id.
    #[cfg(test)]
    fn bruh_grant(&self, agent_id: &str) -> Option<(String, String)> {
        let slot = self.computer_slot(agent_id)?;
        let grant = &slot.grant.grant;
        Some((grant.principal.clone(), grant.id.clone()))
    }
}

#[cfg(any(test, feature = "test-support"))]
pub use fake::FakePages;

#[cfg(any(test, feature = "test-support"))]
mod fake {
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{Arc, Mutex, PoisonError},
    };

    use async_trait::async_trait;
    use bruhs_models::local_tools::{
        BrowserAction, BrowserSession, ComputerInput, ToolReply, Viewport, COMPUTER_VIEWPORT,
    };
    use tokio::sync::Notify;

    use super::{ComputerPage, Launcher, LocalToolsHost, PageLaunch};
    use crate::local_tools::{network::Upstream, ProcessPolicy, ToolError};

    /// What the fake launcher and its pages saw. Clones share it.
    #[derive(Clone, Default)]
    pub struct FakePages {
        seen: Arc<Mutex<Seen>>,
    }

    #[derive(Default)]
    struct Seen {
        launches: Vec<PageLaunch>,
        sessions: HashMap<String, BrowserSession>,
        actions: HashMap<String, Vec<BrowserAction>>,
        inputs: HashMap<String, Vec<ComputerInput>>,
        closes: HashMap<String, Vec<bool>>,
        releases: HashMap<String, usize>,
        urls: HashMap<String, String>,
        holds: HashMap<String, Arc<Notify>>,
    }

    impl FakePages {
        fn seen(&self) -> std::sync::MutexGuard<'_, Seen> {
            self.seen.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub fn launches(&self) -> Vec<PageLaunch> {
            self.seen().launches.clone()
        }

        /// The session of the bruh's latest launch.
        pub fn session(&self, agent_id: &str) -> Option<BrowserSession> {
            self.seen().sessions.get(agent_id).cloned()
        }

        pub fn actions(&self, agent_id: &str) -> Vec<BrowserAction> {
            self.seen()
                .actions
                .get(agent_id)
                .cloned()
                .unwrap_or_default()
        }

        pub fn inputs(&self, agent_id: &str) -> Vec<ComputerInput> {
            self.seen()
                .inputs
                .get(agent_id)
                .cloned()
                .unwrap_or_default()
        }

        pub fn closes(&self, agent_id: &str) -> usize {
            self.seen().closes.get(agent_id).map_or(0, Vec::len)
        }

        /// How often the host released the bruh's mouse button.
        pub fn releases(&self, agent_id: &str) -> usize {
            self.seen().releases.get(agent_id).copied().unwrap_or(0)
        }

        /// For each close, whether the profile folder still existed.
        pub fn closed_with_tree(&self, agent_id: &str) -> Vec<bool> {
            self.seen()
                .closes
                .get(agent_id)
                .cloned()
                .unwrap_or_default()
        }

        /// The page's address, as if it navigated by itself.
        pub fn set_url(&self, agent_id: &str, url: &str) {
            self.seen().urls.insert(agent_id.into(), url.into());
        }

        /// The bruh's next action waits until the returned `Notify` is
        /// notified.
        pub fn hold_next_action(&self, agent_id: &str) -> Arc<Notify> {
            let notify = Arc::new(Notify::new());
            self.seen()
                .holds
                .insert(agent_id.into(), Arc::clone(&notify));
            notify
        }

        fn launcher(&self) -> Launcher {
            let pages = self.clone();
            Arc::new(move |launch: PageLaunch| {
                let pages = pages.clone();
                Box::pin(async move {
                    std::fs::create_dir_all(&launch.profile_dir)?;
                    std::fs::write(launch.profile_dir.join("Cookies"), b"fake")?;
                    let session = BrowserSession {
                        id: uuid::Uuid::new_v4().to_string(),
                        grant_id: launch.grant_id.clone(),
                        url: launch.url.clone(),
                    };
                    let page = FakePage {
                        agent_id: launch.agent_id.clone(),
                        profile_dir: launch.profile_dir.clone(),
                        session: session.clone(),
                        pages: pages.clone(),
                    };
                    let mut seen = pages.seen();
                    seen.urls
                        .insert(launch.agent_id.clone(), launch.url.clone());
                    seen.sessions.insert(launch.agent_id.clone(), session);
                    seen.launches.push(launch);
                    drop(seen);
                    Ok(Box::new(page) as Box<dyn ComputerPage>)
                })
            })
        }
    }

    struct FakePage {
        agent_id: String,
        profile_dir: PathBuf,
        session: BrowserSession,
        pages: FakePages,
    }

    #[async_trait]
    impl ComputerPage for FakePage {
        fn session(&self) -> &BrowserSession {
            &self.session
        }

        fn page_socket(&self) -> Option<String> {
            None
        }

        fn viewport(&self) -> Viewport {
            COMPUTER_VIEWPORT
        }

        async fn run(
            &mut self,
            action: BrowserAction,
            _policy: ProcessPolicy,
        ) -> Result<ToolReply, ToolError> {
            let hold = {
                let mut seen = self.pages.seen();
                seen.actions
                    .entry(self.agent_id.clone())
                    .or_default()
                    .push(action.clone());
                seen.holds.remove(&self.agent_id)
            };
            if let Some(hold) = hold {
                hold.notified().await;
            }
            if let BrowserAction::Navigate { url } = action {
                self.pages.seen().urls.insert(self.agent_id.clone(), url);
            }
            Ok(ToolReply {
                value: serde_json::json!({"ok": true}),
                image: None,
            })
        }

        async fn owner_input(&mut self, input: &ComputerInput) -> Result<(), ToolError> {
            let mut seen = self.pages.seen();
            seen.inputs
                .entry(self.agent_id.clone())
                .or_default()
                .push(input.clone());
            if let ComputerInput::Navigate { url } = input {
                seen.urls.insert(self.agent_id.clone(), url.clone());
            }
            drop(seen);
            Ok(())
        }

        async fn url(&mut self) -> Option<String> {
            self.pages.seen().urls.get(&self.agent_id).cloned()
        }

        async fn release_pointer(&mut self) {
            *self
                .pages
                .seen()
                .releases
                .entry(self.agent_id.clone())
                .or_default() += 1;
        }

        async fn close(&mut self) {
            let tree = self.profile_dir.exists();
            self.pages
                .seen()
                .closes
                .entry(self.agent_id.clone())
                .or_default()
                .push(tree);
        }
    }

    impl LocalToolsHost {
        /// A host whose bruh browsers are fakes, with no upstream proxy.
        pub fn with_fake_pages() -> (Arc<Self>, FakePages) {
            Self::with_fake_pages_and_network(|| Ok(Upstream::default()))
        }

        pub(in crate::local_tools) fn with_fake_pages_and_network(
            network: fn() -> Result<Upstream, ToolError>,
        ) -> (Arc<Self>, FakePages) {
            let pages = FakePages::default();
            let client = reqwest::Client::new();
            let host =
                Self::with_parts(ProcessPolicy::default(), client, pages.launcher(), network)
                    .unwrap_or_else(|error| panic!("the fake host has a valid policy: {error}"));
            (host, pages)
        }
    }
}
