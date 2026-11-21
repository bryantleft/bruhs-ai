//! Other Bruhs tabs while this one puts its browser data back (D19, B Task
//! 13 note, B Task 14 I1). A tab that keeps running holds the old data in
//! memory and would write it back over the restore, so the restoring tab
//! takes the `bruhs-web-restore` Web Lock and posts `reload` on the
//! `bruhs-backup` BroadcastChannel. Each tab that hears it leaves: its
//! navigation guard stops asking, its IndexedDB connection closes and no
//! new operation opens one, then it answers `gone` and reloads. The
//! restoring tab writes once the answers stop, after at least one second
//! and at most two. A tab that starts while the lock is held waits for it
//! before it mounts, so it reads the restored data, and a second `reload`
//! after the write catches one that slipped past. Without Web Locks the
//! handshake still runs.
//!
//! No tab holds a lock while it simply runs: a held Web Lock keeps a page
//! out of the back and forward cache. A frozen background tab answers only
//! once the browser wakes it, which may be after the cap.

use std::cell::{Cell, RefCell};
use std::future::Future;

use js_sys::{Function, Object, Promise, Reflect};
use wasm_bindgen::{closure::Closure, JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

const LOCK_NAME: &str = "bruhs-web-restore";
const CHANNEL_NAME: &str = "bruhs-backup";
/// What a restoring tab posts: every other tab leaves and reloads.
const RELOAD: &str = "reload";
/// What a leaving tab answers once its writes have stopped.
const GONE: &str = "gone";
/// The answers have stopped once none came for this long.
const QUIET_MS: u32 = 250;
/// The restoring tab always waits this long, so a busy tab that reads the
/// message late is still fenced before the write (N1).
const ANSWER_FLOOR_MS: u32 = 1_000;
/// The longest the restoring tab waits for answers.
const ANSWER_CAP_MS: u32 = 2_000;
const POLL_MS: u32 = 50;

thread_local! {
    /// This tab's one channel. A message never reaches the object that
    /// posted it, so the restoring tab posts through this same one and
    /// doesn't reload itself.
    static CHANNEL: RefCell<Option<web_sys::BroadcastChannel>> = const { RefCell::new(None) };
    /// `gone` answers heard since the last `reload` this tab posted.
    static ANSWERS: Cell<u32> = const { Cell::new(0) };
    /// This tab is leaving for restored data.
    static LEAVING: Cell<bool> = const { Cell::new(false) };
}

/// Whether this tab is leaving for restored browser data: its navigation
/// guard doesn't ask, since its writes have stopped.
pub(crate) fn leaving() -> bool {
    LEAVING.with(Cell::get)
}

/// Stops this tab's writes ahead of a reload onto restored data.
pub(crate) fn leave() {
    LEAVING.with(|leaving| leaving.set(true));
    crate::storage::fence_writes();
}

/// Undoes [`leave`] for a restore that stopped before it wrote anything:
/// parked writes go on and the navigation guard asks again.
pub(crate) fn stay() {
    LEAVING.with(|leaving| leaving.set(false));
    crate::storage::unfence_writes();
}

/// Opens this tab's channel: when another tab restores its browser data,
/// this one leaves, answers and reloads. Called once at start.
pub(crate) fn listen() {
    let Ok(channel) = web_sys::BroadcastChannel::new(CHANNEL_NAME) else {
        return;
    };
    let on_message =
        Closure::<dyn Fn(web_sys::MessageEvent)>::new(|event: web_sys::MessageEvent| {
            match event.data().as_string().as_deref() {
                Some(RELOAD) => {
                    leave();
                    post(GONE);
                    if let Some(window) = web_sys::window() {
                        let _ = window.location().reload();
                    }
                }
                Some(GONE) => ANSWERS.with(|answers| answers.set(answers.get().saturating_add(1))),
                _ => {}
            }
        });
    channel.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The tab's lifetime: the channel stays open until it closes.
    on_message.forget();
    CHANNEL.with(|slot| *slot.borrow_mut() = Some(channel));
}

/// Waits while another tab restores its browser data, so this one mounts
/// on the restored data. Answers at once when nothing restores.
pub(crate) async fn wait_for_restore() {
    let Some((locks, request)) = lock_manager() else {
        return;
    };
    let options = Object::new();
    let _ = Reflect::set(&options, &"mode".into(), &"shared".into());
    // Taken and given back at once.
    let callback = Closure::once_into_js(|_lock: JsValue| JsValue::UNDEFINED);
    if let Ok(promise) = request.call3(&locks, &LOCK_NAME.into(), &options, &callback) {
        if let Ok(promise) = promise.dyn_into::<Promise>() {
            let _ = JsFuture::from(promise).await;
        }
    }
}

/// Runs `write` once every other tab has left (or a second passed),
/// holding the restore lock so tabs that start meanwhile wait for it.
pub(crate) async fn with_other_tabs_reloaded<T>(write: impl Future<Output = T>) -> T {
    let held = hold_lock().await;
    ANSWERS.with(|answers| answers.set(0));
    post(RELOAD);
    wait_for_answers().await;
    let out = write.await;
    if let Some((release, request)) = held {
        let _ = release.call0(&JsValue::UNDEFINED);
        let _ = JsFuture::from(request).await;
    }
    post(RELOAD);
    out
}

/// Waits at least [`ANSWER_FLOOR_MS`], then until no `gone` came for
/// [`QUIET_MS`], and at most [`ANSWER_CAP_MS`].
async fn wait_for_answers() {
    let (mut waited, mut quiet, mut heard) = (0, 0, 0);
    while !answers_done(waited, quiet) {
        gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
        waited += POLL_MS;
        let now = ANSWERS.with(Cell::get);
        quiet = if now == heard { quiet + POLL_MS } else { 0 };
        heard = now;
    }
}

/// Whether the wait is over after `waited` ms, the last `quiet` ms of
/// them with no new answer.
fn answers_done(waited: u32, quiet: u32) -> bool {
    waited >= ANSWER_CAP_MS || (waited >= ANSWER_FLOOR_MS && quiet >= QUIET_MS)
}

fn post(message: &str) {
    CHANNEL.with(|slot| {
        if let Some(channel) = slot.borrow().as_ref() {
            let _ = channel.post_message(&message.into());
        }
    });
}

/// `navigator.locks` and its `request`, when this browser has them.
fn lock_manager() -> Option<(JsValue, Function)> {
    let navigator = web_sys::window()?.navigator();
    let locks = Reflect::get(&navigator, &"locks".into()).ok()?;
    if locks.is_undefined() || locks.is_null() {
        return None;
    }
    let request = Reflect::get(&locks, &"request".into())
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
    Some((locks, request))
}

/// The restore lock, held until the answered function is called; the
/// promise settles once it is given back. `None` without Web Locks.
async fn hold_lock() -> Option<(Function, Promise)> {
    let (locks, request) = lock_manager()?;
    let mut release = None;
    let held = Promise::new(&mut |resolve, _| release = Some(resolve));
    let mut granted = None;
    let granted_promise = Promise::new(&mut |resolve, _| granted = Some(resolve));
    let (release, granted) = (release?, granted?);
    let refused = granted.clone();
    let callback = Closure::once_into_js(move |_lock: JsValue| {
        let _ = granted.call1(&JsValue::UNDEFINED, &JsValue::TRUE);
        JsValue::from(held)
    });
    let options = Object::new();
    let _ = Reflect::set(&options, &"mode".into(), &"exclusive".into());
    let request = request
        .call3(&locks, &LOCK_NAME.into(), &options, &callback)
        .ok()?
        .dyn_into::<Promise>()
        .ok()?;
    // A request the browser refuses never calls back: answer the wait.
    let on_refused = Closure::once_into_js(move |_error: JsValue| {
        let _ = refused.call1(&JsValue::UNDEFINED, &JsValue::FALSE);
    });
    let request = Reflect::get(&request, &"catch".into())
        .ok()?
        .dyn_into::<Function>()
        .ok()?
        .call1(&request, &on_refused)
        .ok()?
        .dyn_into::<Promise>()
        .ok()?;
    let granted = JsFuture::from(granted_promise).await.ok()?;
    granted
        .as_bool()
        .unwrap_or(false)
        .then_some((release, request))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_has_a_floor_and_a_cap() {
        // No answers at all: still the full floor.
        assert!(!answers_done(250, 250));
        assert!(!answers_done(950, 950));
        assert!(answers_done(1_000, 1_000));
        // Answers still coming after the floor: wait for them to stop.
        assert!(!answers_done(1_200, 100));
        assert!(answers_done(1_350, 250));
        // A tab that keeps answering can't hold the restore past the cap.
        assert!(answers_done(2_000, 0));
    }
}
