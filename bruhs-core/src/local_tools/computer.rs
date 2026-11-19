//! Bruh computers: one persistent headless browser per bruh with Computer
//! on, started on first use, closed when idle, behind its own network guard.
//!
//! A bruh's grant and page live only in its slot. They never enter the
//! host's thread grants or browser sessions, so Verification doesn't list
//! them and the thread routes can't reach them.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, Weak},
    time::Duration,
};

use async_trait::async_trait;
use bruhs_models::{
    crew::{
        handed_back_note, same_page_host, selector_reads_values, ACTION_STOPPED,
        BROWSER_PAGE_CLOSED_DURING_STEP, BROWSER_PAGE_STOPPED, CLOSED_BY_OWNER, CLOSE_GRACE_MS,
        COMPUTERS_FULL, COMPUTER_IDLE_MS, COMPUTER_OFF_DENIAL, COMPUTER_UNSUPPORTED, FILL_FAILED,
        LIVE_VIEW_ENDED, LOCAL_HOST_DENIAL, LOGIN_WRONG_ORIGIN, MAX_COMPUTERS, NOT_FOR_BRUHS,
        OWNER_HAS_COMPUTER, PAGE_CHANGED, PAGE_UNKNOWN, SCREENSHOT_AFTER_FILL, SELECTOR_AFTER_FILL,
        TAKE_CONTROL_FIRST, TURN_ON_IN_SETTINGS, USE_THE_WINDOW, VAULT_FOR_BRUHS,
    },
    local_tools::{
        validate_action, validate_input, BrowserAction, BrowserActionKind, BrowserSession,
        BrowserStart, ComputerInput, ComputerRequest, ComputerState, Control, Principal, ToolGrant,
        ToolReply, ToolScope, Viewport, COMPUTER_VIEWPORT,
    },
    vault::{same_origin, SecretField},
};
use futures_util::future::{join_all, BoxFuture};
use tokio::{sync::watch, time::Instant};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    browser::{Browser, LaunchOptions},
    network::NetworkGuard,
    screencast::{ComputerWatch, ScreencastHub, ScreencastTask},
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
/// Why no bruh browser opens on this system: none on Unix. Elsewhere the
/// network guard can't read this computer's addresses and a crashed
/// browser's profile isn't recovered yet.
pub(super) fn platform_refusal() -> Option<&'static str> {
    (!cfg!(unix)).then_some(COMPUTER_UNSUPPORTED)
}

/// A vault value the command route resolved for one fill, with the login's
/// origin and the field it is. Neither `Debug` nor `Clone`; the value is
/// zeroed when dropped, which is when the step returns.
pub struct FillValue {
    /// The login's exact origin ([`bruhs_models::vault::login_origin`]).
    pub origin: String,
    pub field: SecretField,
    pub value: Zeroizing<String>,
}

/// 32 documents, each marked by its loader and its history entry.
const MAX_FILLED_MARKS: usize = 64;

/// The documents vault fills touched on one bruh page, by their marks: the
/// main frame's loader and its history entry, oldest first. A navigation
/// keeps them, since Back may return to one; they go when the browser
/// closes.
#[derive(Default)]
pub(super) struct FilledDocuments(VecDeque<String>);

impl FilledDocuments {
    /// A document a fill may type into, marked before it does.
    pub(super) fn mark(&mut self, marks: impl IntoIterator<Item = String>) {
        for mark in marks {
            if !self.0.contains(&mark) {
                self.0.push_back(mark);
            }
        }
        while self.0.len() > MAX_FILLED_MARKS {
            self.0.pop_front();
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a document with these marks is one a fill touched.
    pub(super) fn holds_any(&self, marks: &[String]) -> bool {
        marks.iter().any(|mark| self.0.contains(mark))
    }
}

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
    /// The page target's debugging socket, for the live view. `None` for
    /// a page without one, such as a fake.
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
    /// Type a vault value into `selector` on the main document, only while
    /// it is on `origin`, and press Enter when `submit`. Each document it
    /// may type into is marked in `filled` before it types.
    async fn fill_secret(
        &mut self,
        selector: &str,
        field: SecretField,
        origin: &str,
        value: &str,
        submit: bool,
        filled: &mut FilledDocuments,
    ) -> Result<ToolReply, ToolError>;
    /// The main document's marks ([`FilledDocuments`]), or `None` when the
    /// page doesn't say.
    async fn document_ids(&mut self) -> Option<Vec<String>>;
    async fn url(&mut self) -> Option<String>;
    /// The page target is gone (crashed, or closed itself) and takes no
    /// command again; Chrome may still run.
    fn gone(&self) -> bool;
    /// For a window: its main-frame address as the owner browses. The
    /// sender drops when the page is gone (closed by hand, or Chrome quit).
    fn watch_window(&self) -> watch::Receiver<Option<String>>;
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

    async fn fill_secret(
        &mut self,
        selector: &str,
        field: SecretField,
        origin: &str,
        value: &str,
        submit: bool,
        filled: &mut FilledDocuments,
    ) -> Result<ToolReply, ToolError> {
        Browser::fill_secret(self, selector, field, origin, value, submit, filled).await
    }

    async fn document_ids(&mut self) -> Option<Vec<String>> {
        Browser::document_ids(self).await.ok()
    }

    async fn url(&mut self) -> Option<String> {
        self.current_url().await.ok()
    }

    fn gone(&self) -> bool {
        Browser::gone(self)
    }

    fn watch_window(&self) -> watch::Receiver<Option<String>> {
        Browser::watch_window(self)
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
    /// Why no bruh browser opens on this system, if none does
    /// ([`platform_refusal`]).
    refusal: Option<&'static str>,
    /// Runs in the idle reaper between its first look at a slot and its
    /// try at the page lock, so a test can act in that gap.
    #[cfg(test)]
    before_reap: Mutex<Option<ReapHook>>,
}

#[cfg(test)]
type ReapHook = Box<dyn Fn(&Slot) + Send + Sync>;

impl Computers {
    pub(super) fn new(refusal: Option<&'static str>) -> Self {
        Self {
            refusal,
            ..Self::default()
        }
    }
}

struct Slot {
    agent_id: String,
    profile_dir: PathBuf,
    grant: GrantState,
    meta: Mutex<Meta>,
    /// Bumped when the bruh is stopped; an in-flight action ends.
    interrupt: watch::Sender<u64>,
    state: watch::Sender<ComputerState>,
    /// The live view's newest frame and its viewers, across relaunches.
    hub: Arc<ScreencastHub>,
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

impl PageSlot {
    /// Whether the open page is still the window `session` names. A window
    /// that was closed or relaunched since isn't.
    fn is_window(&self, session: &str) -> bool {
        self.headed
            && self
                .open
                .as_ref()
                .is_some_and(|open| open.page.session().id == session)
    }
}

/// A running page with the guard its traffic goes through, its seat and
/// its live view. Dropping it stops the guard and the screencast and frees
/// the seat.
struct OpenPage {
    page: Box<dyn ComputerPage>,
    _guard: NetworkGuard,
    seat: Seat,
    /// Streams this page while someone watches. Each launch has its own,
    /// since a relaunched page has a new socket.
    screencast: Option<ScreencastTask>,
    /// The documents vault fills touched, on which the bruh's screenshots
    /// and value-probing selectors are refused.
    filled: FilledDocuments,
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
    /// `budget` is the largest live view frame, in bytes.
    /// `refusal` says why no browser opens on this system, if none does.
    fn new(computer: BruhComputer, budget: usize, refusal: Option<&'static str>) -> Self {
        let grant = ToolGrant {
            id: Uuid::new_v4().to_string(),
            principal: Principal::Bruh(computer.agent_id.clone()).wire(),
            scope: ToolScope::Browser,
        };
        let (revoked, _) = watch::channel(false);
        // Where no browser can open, the state says so from the start.
        let error = refusal.map(str::to_owned);
        let (state, _) = watch::channel(ComputerState {
            agent_id: computer.agent_id.clone(),
            error: error.clone(),
            unsupported: refusal.is_some(),
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
            hub: ScreencastHub::new(budget),
            page: tokio::sync::Mutex::new(PageSlot {
                error,
                ..PageSlot::default()
            }),
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
        let session = page
            .open
            .as_ref()
            .map(|open| open.page.session().id.clone());
        let headed = page.headed && page.open.is_some();
        self.state.send_modify(|state| {
            // The same window keeps the size its frames gave it; a launch
            // starts from the page's own measure.
            let same_window = headed && state.headed && state.session_id == session;
            state.session_id = session;
            state.url.clone_from(&page.last_url);
            state.error.clone_from(&page.error);
            state.control = meta.control;
            state.headed = headed;
            if !same_window {
                state.viewport = page
                    .open
                    .as_ref()
                    .map_or(COMPUTER_VIEWPORT, |open| open.page.viewport());
            }
        });
        drop(meta);
    }

    /// Close the open page gracefully, keeping where it was, and give back
    /// its seat for a relaunch.
    async fn close_page(&self, page: &mut PageSlot) -> Option<Seat> {
        let mut open = page.open.take()?;
        // Its screencast goes first, so it doesn't reconnect to a page
        // that is closing.
        drop(open.screencast.take());
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

/// Which page a bruh's step may act on.
#[derive(Clone, Copy)]
enum StepPage<'a> {
    /// Any page (tests).
    #[cfg_attr(
        not(any(test, feature = "test-support")),
        expect(
            dead_code,
            reason = "only the test-only bruh_command skips the page check"
        )
    )]
    Unchecked,
    /// Input only on the host of the page the gate allowed it on.
    AllowedOn(Option<&'a str>),
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

/// Why a bruh's step is refused on a document a vault fill touched: a
/// screenshot, which could show a revealed value, or a selector whose
/// match count could read it back. A page that can't say which document
/// it is counts as a filled one.
async fn after_fill_refusal(open: &mut OpenPage, action: &BrowserAction) -> Option<&'static str> {
    if open.filled.is_empty() {
        return None;
    }
    let refusal = match action {
        BrowserAction::Screenshot => SCREENSHOT_AFTER_FILL,
        BrowserAction::Click { selector }
        | BrowserAction::Type {
            selector: Some(selector),
            ..
        }
        | BrowserAction::Fill { selector, .. }
            if selector_reads_values(selector) =>
        {
            SELECTOR_AFTER_FILL
        }
        _ => return None,
    };
    let marks = tokio::time::timeout(PAGE_URL_WAIT, open.page.document_ids())
        .await
        .ok()
        .flatten();
    match marks {
        Some(marks) if !open.filled.holds_any(&marks) => None,
        _ => Some(refusal),
    }
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
                    let slot = Slot::new(
                        computer,
                        self.policy.max_output_bytes,
                        self.computers.refusal,
                    );
                    slots.insert(slot.agent_id.clone(), Arc::new(slot));
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
            slot.hub.end();
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
            slot.hub.end();
        }
        join_all(slots.iter().map(|slot| async move {
            let mut page = slot.page.lock().await;
            slot.shut(&mut page).await;
        }))
        .await;
    }

    /// The refusals a bruh's browser step meets before it runs, without
    /// running it: Computer off, an action bruhs don't get, an invalid
    /// action, a stopped bruh, or the owner holding the computer. The
    /// command route repeats them, so a step it can't run says why.
    pub fn check_bruh_command(
        &self,
        agent_id: &str,
        action: &BrowserAction,
    ) -> Result<(), ToolError> {
        self.checked_slot(agent_id, action).map(drop)
    }

    fn checked_slot(&self, agent_id: &str, action: &BrowserAction) -> Result<Arc<Slot>, ToolError> {
        let slot = self
            .computer_slot(agent_id)
            .ok_or_else(|| ToolError::Refused(COMPUTER_OFF_DENIAL.into()))?;
        if !action.for_bruhs() {
            return Err(ToolError::Refused(NOT_FOR_BRUHS.into()));
        }
        // Navigate's check refuses a host on this computer.
        let viewport = slot.state.borrow().viewport;
        validate_action(action, viewport).map_err(invalid)?;
        if slot.paused() {
            return Err(ToolError::Refused(ACTION_STOPPED.into()));
        }
        if slot.control() == Control::Owner {
            return Err(ToolError::Refused(OWNER_HAS_COMPUTER.into()));
        }
        Ok(slot)
    }

    /// One action from a bruh's own browser tool, with no page check. For
    /// tests; the command route uses [`Self::bruh_command_on`].
    #[cfg(any(test, feature = "test-support"))]
    pub async fn bruh_command(
        &self,
        agent_id: &str,
        action: BrowserAction,
    ) -> Result<ToolReply, ToolError> {
        self.bruh_step(agent_id, action, StepPage::Unchecked, None)
            .await
    }

    /// One action from a bruh's own browser tool, which the gate allowed on
    /// `allowed_on`. Input runs only while the live page is on that host,
    /// checked under the page lock right before it is sent: a page that
    /// moved is refused with [`PAGE_CHANGED`], and one that doesn't say
    /// where it is (now or when the gate looked) with [`PAGE_UNKNOWN`].
    pub async fn bruh_command_on(
        &self,
        agent_id: &str,
        action: BrowserAction,
        allowed_on: Option<&str>,
    ) -> Result<ToolReply, ToolError> {
        self.bruh_step(agent_id, action, StepPage::AllowedOn(allowed_on), None)
            .await
    }

    /// A vault fill from a bruh's own browser tool, which the gate allowed
    /// on `allowed_on`, with the value the command route resolved. It types
    /// only while the live page and `allowed_on` are both on the login's
    /// exact origin, checked under the page lock right before it types and
    /// again in the page as it writes; otherwise [`LOGIN_WRONG_ORIGIN`],
    /// and nothing is typed. The reply never holds the value, which is
    /// zeroed when this returns.
    pub async fn bruh_fill_on(
        &self,
        agent_id: &str,
        action: BrowserAction,
        value: FillValue,
        allowed_on: Option<&str>,
    ) -> Result<ToolReply, ToolError> {
        self.bruh_step(
            agent_id,
            action,
            StepPage::AllowedOn(allowed_on),
            Some(value),
        )
        .await
    }

    async fn bruh_step(
        &self,
        agent_id: &str,
        action: BrowserAction,
        page_check: StepPage<'_>,
        secret: Option<FillValue>,
    ) -> Result<ToolReply, ToolError> {
        let slot = self.checked_slot(agent_id, &action)?;
        // A vault fill goes only with its value, and the value only with
        // the fill of its own field.
        let fill = action
            .secret_fill()
            .map(|(selector, reference, submit)| (selector.to_owned(), reference.field, submit));
        match (&fill, &secret) {
            (Some(_), None) => return Err(ToolError::Invalid(VAULT_FOR_BRUHS.into())),
            (Some((_, field, _)), Some(value)) if *field == value.field => {}
            (None, None) => {}
            _ => return Err(ToolError::Invalid(FILL_FAILED.into())),
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
            if let Some(value) = &secret {
                // The exact origin takes the place of the host check.
                let allowed_on = match page_check {
                    StepPage::AllowedOn(allowed_on) => allowed_on,
                    StepPage::Unchecked => None,
                };
                let live = tokio::time::timeout(PAGE_URL_WAIT, open.page.url())
                    .await
                    .ok()
                    .flatten();
                if allowed_on.is_none() || live.is_none() {
                    return Err(ToolError::Refused(PAGE_UNKNOWN.into()));
                }
                if !same_origin(live.as_deref(), &value.origin)
                    || !same_origin(allowed_on, &value.origin)
                {
                    return Err(ToolError::Refused(LOGIN_WRONG_ORIGIN.into()));
                }
            } else if let StepPage::AllowedOn(allowed_on) = page_check {
                if action.kind() == BrowserActionKind::Input {
                    let live = tokio::time::timeout(PAGE_URL_WAIT, open.page.url())
                        .await
                        .ok()
                        .flatten();
                    if allowed_on.is_none() || live.is_none() {
                        return Err(ToolError::Refused(PAGE_UNKNOWN.into()));
                    }
                    if !same_page_host(allowed_on, live.as_deref()) {
                        return Err(ToolError::Refused(PAGE_CHANGED.into()));
                    }
                }
            }
            if let Some(refusal) = after_fill_refusal(open, &action).await {
                return Err(ToolError::Refused(refusal.into()));
            }
            // Anything but a read may have landed before a page dies.
            let acts = action.kind() != BrowserActionKind::Read;
            let reply = match (fill, &secret) {
                (Some((selector, field, submit)), Some(value)) => {
                    let OpenPage { page, filled, .. } = &mut *open;
                    let typed = page.fill_secret(
                        &selector,
                        field,
                        &value.origin,
                        &value.value,
                        submit,
                        filled,
                    );
                    // A fill cut off here may have typed; its documents
                    // are marked already.
                    tokio::time::timeout(self.policy.timeout, typed)
                        .await
                        .unwrap_or_else(|_| Err(ToolError::Invalid(FILL_FAILED.into())))
                }
                _ => tokio::time::timeout(self.policy.timeout, open.page.run(action, self.policy))
                    .await
                    .unwrap_or_else(|_| Err(ToolError::Unavailable(ACTION_TIMED_OUT.into()))),
            };
            if open.page.gone() {
                // The page target ended under Chrome. Close it where it was
                // last, so the next step reopens it. A read can be tried
                // again; a step that acts may have run, so it isn't.
                drop(slot.close_page(&mut page).await);
                slot.publish(&page);
                let reason = if acts {
                    BROWSER_PAGE_CLOSED_DURING_STEP
                } else {
                    BROWSER_PAGE_STOPPED
                };
                return Err(ToolError::Unavailable(reason.into()));
            }
            let url = tokio::time::timeout(PAGE_URL_WAIT, open.page.url())
                .await
                .ok()
                .flatten();
            if let Some(url) = url.or(navigated.filter(|_| reply.is_ok())) {
                page.last_url = Some(url);
            }
            // A failed step isn't use: a bruh retrying a broken page doesn't
            // keep the browser from closing when idle.
            if reply.is_ok() {
                page.last_used = Some(Instant::now());
            }
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
        // A closed page that is locked is opening for the bruh's action,
        // which isn't waited on either; owner input opens it if it didn't.
        let Ok(mut page) = slot.page.try_lock() else {
            return Ok(());
        };
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
        if slot.control() == Control::Owner {
            self.give_back(slot, &mut page).await;
        }
        drop(page);
    }

    /// Hand back under the page lock: a window reopens headless where it
    /// was, then control and the note go to the bruh.
    async fn give_back(&self, slot: &Slot, page: &mut PageSlot) {
        if page.headed {
            // A failed relaunch shows in the state's error; the next agent
            // command opens it again.
            if let Err(error) = self.relaunch(slot, page, true).await {
                tracing::warn!(%error, "bruh browser didn't reopen headless");
            }
        }
        slot.refresh_url(page).await;
        let url = page
            .last_url
            .clone()
            .unwrap_or_else(|| "about:blank".into());
        page.last_used = Some(Instant::now());
        slot.set_control(Control::Bruh, Some(handed_back_note(&url)));
        slot.publish(page);
    }

    /// Follow a window's page: where the owner browses becomes the slot's
    /// address, and a window closed by hand (or a Chrome that quit) is
    /// handed back as if the owner pressed Hand back.
    fn watch_window(&self, agent_id: &str, open: &OpenPage) {
        let mut window = open.page.watch_window();
        let (host, agent_id) = (self.this.clone(), agent_id.to_owned());
        let session = open.page.session().id.clone();
        tokio::spawn(async move {
            while window.changed().await.is_ok() {
                let url = window.borrow_and_update().clone();
                let (Some(url), Some(host)) = (url, host.upgrade()) else {
                    continue;
                };
                host.window_moved(&agent_id, &session, url).await;
            }
            if let Some(host) = host.upgrade() {
                host.window_gone(&agent_id, &session).await;
            }
        });
    }

    async fn window_moved(&self, agent_id: &str, session: &str, url: String) {
        let Some(slot) = self.computer_slot(agent_id) else {
            return;
        };
        let mut page = slot.page.lock().await;
        if page.is_window(session) {
            page.last_url = Some(url);
            slot.publish(&page);
        }
        drop(page);
    }

    async fn window_gone(&self, agent_id: &str, session: &str) {
        let Some(slot) = self.computer_slot(agent_id) else {
            return;
        };
        let mut page = slot.page.lock().await;
        if page.is_window(session) {
            tracing::debug!(bruh = %agent_id, "bruh window closed by hand");
            self.give_back(&slot, &mut page).await;
        }
        drop(page);
    }

    /// The owner opens the bruh's browser in a window on this computer, for
    /// what the live view can't do: the headless page closes gracefully and
    /// the same profile opens headed where it was.
    async fn open_window(&self, slot: &Slot) -> Result<(), ToolError> {
        let before = slot.control();
        slot.set_control(Control::Owner, None);
        let mut page = slot.page.lock().await;
        if page.headed && page.open.is_some() {
            return Ok(());
        }
        let opened = self.relaunch(slot, &mut page, false).await;
        if let Err(error) = &opened {
            if before == Control::Owner {
                // The owner keeps control and the live view: headless again
                // where it was, still showing why the window didn't open.
                let _ = self.ensure_open(slot, &mut page).await;
                page.error = Some(error.to_string());
                slot.publish(&page);
            } else {
                // The bruh keeps its browser, which reopens headless where
                // it was on its next step.
                slot.set_control(Control::Bruh, None);
            }
        }
        drop(page);
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
        page.last_used = Some(Instant::now());
        // Pointer and key input leave the state alone: the live view
        // follows where a click leads, which the slot's last address may
        // not know yet.
        if matches!(
            input,
            ComputerInput::Navigate { .. } | ComputerInput::Back | ComputerInput::Reload
        ) {
            slot.refresh_url(&mut page).await;
            slot.publish(&page);
        }
        drop(page);
        result
    }

    /// A live view of the bruh browser `session_id` names: its frames and
    /// state. A thread's verification browser has no live view.
    pub fn watch_computer(&self, session_id: &str) -> Result<ComputerWatch, ToolError> {
        let slot = self
            .computer_slots()
            .into_iter()
            .find(|slot| slot.state.borrow().session_id.as_deref() == Some(session_id))
            .ok_or_else(|| ToolError::NotFound(LIVE_VIEW_ENDED.into()))?;
        // An open page's screencast starts as its first viewer joins.
        let viewer = slot.hub.viewer()?;
        Ok(ComputerWatch {
            agent_id: slot.agent_id.clone(),
            viewer,
            state: slot.state.subscribe(),
        })
    }

    /// Whether the bruh has a Computer slot: its Computer is on here.
    pub fn has_computer(&self, agent_id: &str) -> bool {
        self.computer_slot(agent_id).is_some()
    }

    /// The bruh's Computer state as it changes, for runs that wait while
    /// the owner holds it. `None` without a slot.
    pub fn control_changes(&self, agent_id: &str) -> Option<watch::Receiver<ComputerState>> {
        self.computer_slot(agent_id)
            .map(|slot| slot.state.subscribe())
    }

    /// The address the bruh's page is on now, asked of the open page. A
    /// closed browser reopens where it was last. An open page that doesn't
    /// answer in time is on an unknown page, never on its last address.
    pub async fn page_url(&self, agent_id: &str) -> Option<String> {
        let slot = self.computer_slot(agent_id)?;
        tokio::time::timeout(PAGE_URL_WAIT, async {
            let mut page = slot.page.lock().await;
            match page.open.as_mut() {
                Some(open) => open.page.url().await,
                None => page.last_url.clone(),
            }
        })
        .await
        .ok()
        .flatten()
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
            // A page in use, held by the owner or watched isn't idle.
            if slot.control() == Control::Owner {
                return;
            }
            #[cfg(test)]
            if let Some(hook) = &*self
                .computers
                .before_reap
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
            {
                hook(slot);
            }
            let Ok(mut page) = slot.page.try_lock() else {
                return;
            };
            // Take control doesn't lock an open page, so the owner may have
            // taken it since the first look.
            if slot.control() == Control::Owner {
                return;
            }
            if slot.hub.watched() {
                // Its idle time starts when the last viewer leaves.
                page.last_used = Some(Instant::now());
                return;
            }
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
        // A page whose target ended takes no command: close it to reopen.
        if page.open.as_ref().is_some_and(|open| open.page.gone()) {
            drop(slot.close_page(page).await);
        }
        if page.open.is_some() {
            return Ok(());
        }
        let launched = self.launch(slot, page.last_url.clone(), true, None).await;
        self.opened(slot, page, launched, true)
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
        self.opened(slot, page, launched, headless)
    }

    /// Record a launch's outcome in the slot and publish it. A window is
    /// watched from here on.
    fn opened(
        &self,
        slot: &Slot,
        page: &mut PageSlot,
        launched: Result<OpenPage, ToolError>,
        headless: bool,
    ) -> Result<(), ToolError> {
        let result = match launched {
            Ok(mut open) => {
                if !headless {
                    self.watch_window(&slot.agent_id, &open);
                }
                // Every launch while watched (a reopen, a window, a hand
                // back) streams its new page.
                open.screencast = open.page.page_socket().map(|socket| {
                    ScreencastTask::start(
                        socket,
                        Arc::clone(&slot.hub),
                        slot.state.clone(),
                        self.policy,
                    )
                });
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
        if let Some(reason) = self.computers.refusal {
            return Err(ToolError::Unavailable(reason.into()));
        }
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
            screencast: None,
            filled: FilledDocuments::default(),
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
        sync::{Arc, Mutex, OnceLock, PoisonError, Weak},
    };

    use async_trait::async_trait;
    use bruhs_models::{
        local_tools::{
            BrowserAction, BrowserSession, ComputerInput, ToolReply, Viewport, COMPUTER_VIEWPORT,
        },
        vault::SecretField,
    };
    use tokio::sync::{watch, Notify};

    use super::{ComputerPage, FilledDocuments, Launcher, LocalToolsHost, PageLaunch};
    use crate::local_tools::{browser::filled_reply, network::Upstream, ProcessPolicy, ToolError};

    /// A vault fill a fake page took: selector, field, submit and the value,
    /// kept so tests can compare it.
    type SecretFill = (String, SecretField, bool, String);

    /// What the fake launcher and its pages saw. Clones share it.
    #[derive(Clone, Default)]
    pub struct FakePages {
        seen: Arc<Mutex<Seen>>,
        /// The host these pages belong to, for frames sent to its slots.
        host: Arc<OnceLock<Weak<LocalToolsHost>>>,
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
        /// Each bruh's latest page's window feed. Dropping it closes the
        /// window.
        windows: HashMap<String, watch::Sender<Option<String>>>,
        /// Sessions whose window was closed by hand: they answer nothing.
        gone: std::collections::HashSet<String>,
        /// Sessions whose page target ended: every command fails.
        dead: std::collections::HashSet<String>,
        fail_actions: std::collections::HashSet<String>,
        fail_launches: std::collections::HashSet<String>,
        /// The page socket each bruh's next launches report.
        page_sockets: HashMap<String, String>,
        secret_fills: HashMap<String, Vec<SecretFill>>,
        /// The document each bruh's page is on; `doc-0` until set.
        documents: HashMap<String, String>,
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

        /// The vault fills the bruh's pages took, values included.
        pub fn secret_fills(&self, agent_id: &str) -> Vec<SecretFill> {
            self.seen()
                .secret_fills
                .get(agent_id)
                .cloned()
                .unwrap_or_default()
        }

        /// The page's main document, as if it loaded another (or went
        /// back to one).
        pub fn set_document(&self, agent_id: &str, id: &str) {
            self.seen().documents.insert(agent_id.into(), id.into());
        }

        /// Whether the bruh's held action has reached its page.
        pub fn hold_reached(&self, agent_id: &str) -> bool {
            !self.seen().holds.contains_key(agent_id)
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

        /// The owner browses the bruh's window to `url`. The page itself
        /// isn't asked, as if it couldn't answer.
        pub fn browse_window(&self, agent_id: &str, url: &str) {
            if let Some(window) = self.seen().windows.get(agent_id) {
                window.send_replace(Some(url.into()));
            }
        }

        /// The owner closes the bruh's window by hand: its page answers
        /// nothing from now on.
        pub fn close_window(&self, agent_id: &str) {
            let mut seen = self.seen();
            if let Some(session) = seen.sessions.get(agent_id).map(|s| s.id.clone()) {
                seen.gone.insert(session);
            }
            let window = seen.windows.remove(agent_id);
            drop(seen);
            drop(window);
        }

        /// The bruh's page target ends while Chrome lives on, as after a
        /// crash or a page that closed itself.
        pub fn kill_target(&self, agent_id: &str) {
            let mut seen = self.seen();
            if let Some(session) = seen.sessions.get(agent_id).map(|s| s.id.clone()) {
                seen.dead.insert(session);
            }
        }

        /// The bruh's next action fails, as on a page that errors.
        pub fn fail_next_action(&self, agent_id: &str) {
            self.seen().fail_actions.insert(agent_id.into());
        }

        /// A frame from the bruh's page, as Chrome's screencast would send.
        pub fn publish_frame(&self, agent_id: &str, bytes: Vec<u8>) {
            let host = self.host.get().and_then(Weak::upgrade);
            if let Some(slot) = host.and_then(|host| host.computer_slot(agent_id)) {
                slot.hub.publish(bytes);
            }
        }

        /// The bruh's later pages report `url` as their debugging socket,
        /// so the host streams them from there.
        pub fn set_page_socket(&self, agent_id: &str, url: &str) {
            self.seen().page_sockets.insert(agent_id.into(), url.into());
        }

        /// The bruh's next launch fails, as when Chrome can't open.
        pub fn fail_next_launch(&self, agent_id: &str) {
            self.seen().fail_launches.insert(agent_id.into());
        }

        fn launcher(&self) -> Launcher {
            let pages = self.clone();
            Arc::new(move |launch: PageLaunch| {
                let pages = pages.clone();
                Box::pin(async move {
                    if pages.seen().fail_launches.remove(&launch.agent_id) {
                        return Err(ToolError::Unavailable("Chrome didn't open.".into()));
                    }
                    std::fs::create_dir_all(&launch.profile_dir)?;
                    std::fs::write(launch.profile_dir.join("Cookies"), b"fake")?;
                    let session = BrowserSession {
                        id: uuid::Uuid::new_v4().to_string(),
                        grant_id: launch.grant_id.clone(),
                        url: launch.url.clone(),
                    };
                    let (window, feed) = watch::channel(None);
                    let mut seen = pages.seen();
                    let page = FakePage {
                        agent_id: launch.agent_id.clone(),
                        profile_dir: launch.profile_dir.clone(),
                        session: session.clone(),
                        pages: pages.clone(),
                        window: feed,
                        socket: seen.page_sockets.get(&launch.agent_id).cloned(),
                        noticed: false,
                    };
                    seen.windows.insert(launch.agent_id.clone(), window);
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
        window: watch::Receiver<Option<String>>,
        socket: Option<String>,
        /// A command found the target dead, as a real page's connection
        /// learns only when it is used.
        noticed: bool,
    }

    impl FakePage {
        fn window_closed(&self) -> bool {
            self.pages.seen().gone.contains(&self.session.id)
        }

        fn dead(&self) -> bool {
            self.pages.seen().dead.contains(&self.session.id)
        }

        fn document(&self) -> String {
            self.pages
                .seen()
                .documents
                .get(&self.agent_id)
                .cloned()
                .unwrap_or_else(|| "doc-0".into())
        }
    }

    #[async_trait]
    impl ComputerPage for FakePage {
        fn session(&self) -> &BrowserSession {
            &self.session
        }

        fn page_socket(&self) -> Option<String> {
            self.socket.clone()
        }

        fn viewport(&self) -> Viewport {
            COMPUTER_VIEWPORT
        }

        async fn run(
            &mut self,
            action: BrowserAction,
            _policy: ProcessPolicy,
        ) -> Result<ToolReply, ToolError> {
            let (hold, fail) = {
                let mut seen = self.pages.seen();
                let fail = seen.fail_actions.remove(&self.agent_id);
                seen.actions
                    .entry(self.agent_id.clone())
                    .or_default()
                    .push(action.clone());
                (seen.holds.remove(&self.agent_id), fail)
            };
            if let Some(hold) = hold {
                hold.notified().await;
            }
            if self.dead() {
                self.noticed = true;
                return Err(ToolError::Unavailable(
                    "Chrome closed the debugging connection. Start another browser.".into(),
                ));
            }
            if fail {
                return Err(ToolError::Unavailable("The page failed.".into()));
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
            if self.dead() {
                self.noticed = true;
                return Err(ToolError::Unavailable(
                    "Chrome closed the debugging connection. Start another browser.".into(),
                ));
            }
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

        async fn fill_secret(
            &mut self,
            selector: &str,
            field: SecretField,
            _origin: &str,
            value: &str,
            submit: bool,
            filled: &mut FilledDocuments,
        ) -> Result<ToolReply, ToolError> {
            // As a real page does, before anything is typed.
            filled.mark([self.document()]);
            let hold = self.pages.seen().holds.remove(&self.agent_id);
            if let Some(hold) = hold {
                hold.notified().await;
            }
            if self.dead() {
                self.noticed = true;
                return Err(ToolError::Unavailable(
                    "Chrome closed the debugging connection. Start another browser.".into(),
                ));
            }
            self.pages
                .seen()
                .secret_fills
                .entry(self.agent_id.clone())
                .or_default()
                .push((selector.into(), field, submit, value.into()));
            Ok(filled_reply(field, submit))
        }

        async fn document_ids(&mut self) -> Option<Vec<String>> {
            if self.window_closed() || self.dead() {
                return None;
            }
            Some(vec![self.document()])
        }

        async fn url(&mut self) -> Option<String> {
            if self.window_closed() || self.dead() {
                return None;
            }
            self.pages.seen().urls.get(&self.agent_id).cloned()
        }

        fn gone(&self) -> bool {
            self.noticed
        }

        fn watch_window(&self) -> watch::Receiver<Option<String>> {
            self.window.clone()
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
            Self::with_fake_parts(network, None)
        }

        /// A fake host on a system where no bruh browser opens.
        #[cfg(test)]
        pub(in crate::local_tools) fn with_fake_pages_refusing(
            refusal: &'static str,
        ) -> (Arc<Self>, FakePages) {
            Self::with_fake_parts(|| Ok(Upstream::default()), Some(refusal))
        }

        fn with_fake_parts(
            network: fn() -> Result<Upstream, ToolError>,
            refusal: Option<&'static str>,
        ) -> (Arc<Self>, FakePages) {
            let pages = FakePages::default();
            let client = reqwest::Client::new();
            let host = Self::with_parts(
                ProcessPolicy::default(),
                client,
                pages.launcher(),
                network,
                refusal,
            )
            .unwrap_or_else(|error| panic!("the fake host has a valid policy: {error}"));
            let _ = pages.host.set(Arc::downgrade(&host));
            (host, pages)
        }
    }
}
