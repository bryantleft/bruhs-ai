//! The web app's one live connection to bruhs-server
//! (`bruhs_models::api::live_events`).
//!
//! Over plain `http://localhost` the browser allows six connections per host.
//! A stream per source (tasks, calendar, meetings) used half of them per tab,
//! so a second tab, or pages the browser keeps in its back/forward cache,
//! stalled every other request. Now each tab holds one stream, and closes it
//! while the page is hidden in that cache.
//!
//! On an error the stream is closed and reopened after a backoff (1 s,
//! doubling to 30 s) with a "reconnecting" banner. On every open the server
//! sends the task list and the meetings handshake (which reloads open
//! transcripts); a reopen also asks it to replay the calendar sync window,
//! since another client may have taken events this tab missed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use bruhs_models::api::live_events;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{EventSource, MessageEvent};

use crate::components::banner::{BannerKind, BannerStackSignal};

const BACKOFF_INITIAL_MS: u32 = 1_000;
const BACKOFF_MAX_MS: u32 = 30_000;

fn stream_url(api_base: &str, resume: bool) -> String {
    let base = api_base.trim_end_matches('/');
    let path = live_events::PATH;
    if resume {
        format!("{base}{path}?{}", live_events::RESUME_QUERY)
    } else {
        format!("{base}{path}")
    }
}

fn next_backoff(delay_ms: u32) -> u32 {
    delay_ms.saturating_mul(2).min(BACKOFF_MAX_MS)
}

type Handler = Rc<dyn Fn(String)>;

/// Handlers for the frames of the live stream, by frame name. Register them
/// all, then [`LiveEvents::connect`] once.
#[derive(Default)]
pub struct LiveEvents {
    handlers: Vec<(&'static str, Handler)>,
}

impl LiveEvents {
    /// Call `handler` with the data of every frame named `name`.
    pub fn on(&mut self, name: &'static str, handler: impl Fn(String) + 'static) {
        self.handlers.push((name, Rc::new(handler)));
    }

    /// Open the stream for the life of the page.
    pub fn connect(self, banners: Option<BannerStackSignal>) {
        let connection = Rc::new(Connection {
            handlers: self.handlers,
            backoff_ms: Cell::new(BACKOFF_INITIAL_MS),
            generation: Cell::new(0),
            hidden: Cell::new(false),
            opened: Cell::new(false),
            open: RefCell::new(None),
            banners,
        });
        Connection::open(&connection);
        Connection::follow_page(&connection);
    }
}

/// One open `EventSource` and the callbacks registered on it. Dropping it
/// closes the stream and frees the callbacks.
struct OpenStream {
    source: EventSource,
    _listeners: Vec<Closure<dyn FnMut(MessageEvent)>>,
    _on_error: Closure<dyn FnMut(web_sys::Event)>,
}

impl Drop for OpenStream {
    fn drop(&mut self) {
        self.source.close();
    }
}

struct Connection {
    handlers: Vec<(&'static str, Handler)>,
    backoff_ms: Cell<u32>,
    /// Bumped on every open and on page hide, so a retry scheduled for an
    /// older stream does nothing.
    generation: Cell<u32>,
    /// The page is in the back/forward cache; stay closed until it is shown.
    hidden: Cell<bool>,
    /// The stream was opened before, so the next open is a resume.
    opened: Cell<bool>,
    open: RefCell<Option<OpenStream>>,
    banners: Option<BannerStackSignal>,
}

impl Connection {
    fn open(connection: &Rc<Self>) {
        connection
            .generation
            .set(connection.generation.get().wrapping_add(1));
        connection.open.borrow_mut().take();
        let url = stream_url(crate::api::BASE_URL, connection.opened.replace(true));
        let source = match EventSource::new(&url) {
            Ok(source) => source,
            Err(error) => {
                log::warn!("live events stream failed to open: {error:?}");
                return;
            }
        };
        let listeners = connection
            .handlers
            .iter()
            .map(|(name, handler)| {
                let handler = Rc::clone(handler);
                let weak = Rc::downgrade(connection);
                let listener =
                    Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                        if let Some(connection) = weak.upgrade() {
                            connection.backoff_ms.set(BACKOFF_INITIAL_MS);
                        }
                        if let Some(data) = event.data().as_string() {
                            handler(data);
                        }
                    });
                if let Err(error) =
                    source.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref())
                {
                    log::warn!("could not listen for {name} events: {error:?}");
                }
                listener
            })
            .collect();
        let weak = Rc::downgrade(connection);
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if let Some(connection) = weak.upgrade() {
                Connection::retry_later(&connection);
            }
        });
        source.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        *connection.open.borrow_mut() = Some(OpenStream {
            source,
            _listeners: listeners,
            _on_error: on_error,
        });
    }

    /// Close the broken stream (the browser would retry at its own pace) and
    /// reopen it after the backoff. The `OpenStream` itself is dropped by the
    /// next open, not here: this runs inside its error callback.
    fn retry_later(connection: &Rc<Self>) {
        if let Some(open) = connection.open.borrow().as_ref() {
            open.source.close();
        }
        let delay = connection.backoff_ms.get();
        connection.backoff_ms.set(next_backoff(delay));
        log::warn!("live events stream error; reconnecting in {delay}ms");
        if let Some(banners) = connection.banners {
            banners.push(
                BannerKind::Warning,
                "Lost the connection to bruhs-server, reconnecting…",
                Some(delay + 500),
            );
        }
        let generation = connection.generation.get();
        let weak = Rc::downgrade(connection);
        gloo_timers::callback::Timeout::new(delay, move || {
            let Some(connection) = weak.upgrade() else {
                return;
            };
            if connection.generation.get() == generation && !connection.hidden.get() {
                Connection::open(&connection);
            }
        })
        // One-shot and freed after it runs; a stale one is a no-op.
        .forget();
    }

    /// Close the stream while the page sits in the back/forward cache, where
    /// it would otherwise keep a connection the open tabs need, and reopen it
    /// when the page is shown again.
    fn follow_page(connection: &Rc<Self>) {
        let Some(window) = web_sys::window() else {
            // No page to follow; keep the stream open regardless.
            std::mem::forget(Rc::clone(connection));
            return;
        };
        // These two own the connection: they live as long as the page, and
        // every other callback holds it weakly.
        let hiding = Rc::clone(connection);
        let on_hide = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            hiding.hidden.set(true);
            hiding
                .generation
                .set(hiding.generation.get().wrapping_add(1));
            hiding.open.borrow_mut().take();
        });
        let showing = Rc::clone(connection);
        let on_show = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if showing.hidden.replace(false) {
                showing.backoff_ms.set(BACKOFF_INITIAL_MS);
                Connection::open(&showing);
            }
        });
        for (name, callback) in [("pagehide", &on_hide), ("pageshow", &on_show)] {
            if let Err(error) =
                window.add_event_listener_with_callback(name, callback.as_ref().unchecked_ref())
            {
                log::warn!("could not listen for {name}: {error:?}");
            }
        }
        // Registered once for the life of the page.
        on_hide.forget();
        on_show.forget();
    }
}

#[cfg(test)]
mod tests {
    use super::{next_backoff, stream_url, BACKOFF_INITIAL_MS, BACKOFF_MAX_MS};

    #[test]
    fn stream_follows_the_configured_api_origin() {
        assert_eq!(
            stream_url("http://127.0.0.1:8097/", false),
            "http://127.0.0.1:8097/api/events/stream"
        );
        assert_eq!(stream_url("", false), "/api/events/stream");
    }

    #[test]
    fn a_reopen_asks_for_missed_events() {
        assert_eq!(stream_url("", true), "/api/events/stream?resume=true");
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        assert_eq!(next_backoff(BACKOFF_INITIAL_MS), 2_000);
        assert_eq!(next_backoff(20_000), BACKOFF_MAX_MS);
        assert_eq!(next_backoff(u32::MAX), BACKOFF_MAX_MS);
    }
}
