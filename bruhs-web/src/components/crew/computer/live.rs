//! The live view's socket: frames shown through object URLs and
//! acknowledged once decoded, state and error messages handed to the tab,
//! and reconnects after a capped backoff.

use std::time::Duration;

use bruhs_models::crew::LIVE_VIEW_UNREACHABLE;
use bruhs_models::local_tools::{
    Backoff, ComputerInput, ComputerState, ScreencastClientMessage, ScreencastMessage,
    BACKOFF_MAX_MS, FRAME_ACK,
};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// What the live view tells the tab.
#[derive(Clone, Copy)]
pub(super) struct LiveEvents {
    /// The service's state, newer than any request in flight.
    pub state: Callback<ComputerState>,
    /// The stream failed with this message, or (`None`) works again after
    /// its failure.
    pub error: Callback<Option<String>>,
    /// The socket closed and its wait is over: fetch the state again, then
    /// call [`LiveView::retry`] with this token.
    pub closed: Callback<u64>,
}

/// What a closed socket shows: one that never opened couldn't reach the
/// server. One that opened has said why itself, if it was refused.
fn closed_error(opened: bool) -> Option<&'static str> {
    (!opened).then_some(LIVE_VIEW_UNREACHABLE)
}

fn revoke(url: &str) {
    let _ = web_sys::Url::revoke_object_url(url);
}

/// A live view socket and the handlers it calls, kept for its life.
struct Socket {
    socket: web_sys::WebSocket,
    _open: Closure<dyn FnMut(web_sys::Event)>,
    _message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _close: Closure<dyn FnMut(web_sys::CloseEvent)>,
}

impl Socket {
    /// Close without running the close handler, so nothing reconnects.
    fn close(&self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        let _ = self.socket.close();
    }

    fn send(&self, text: &str) {
        if self.socket.ready_state() == web_sys::WebSocket::OPEN {
            let _ = self.socket.send_with_str(text);
        }
    }
}

/// One tab's live view: its socket, the frame on screen and the retry.
#[derive(Clone, Copy)]
pub(super) struct LiveView {
    img: NodeRef<html::Img>,
    socket: StoredValue<Option<Socket>, LocalStorage>,
    /// Bumped whenever a socket goes, so its late handlers do nothing.
    generation: StoredValue<u64>,
    /// Bumped whenever a socket goes or the frame is cleared, so a frame
    /// still decoding is dropped.
    epoch: StoredValue<u64>,
    shown_url: StoredValue<Option<String>>,
    backoff: StoredValue<Backoff>,
    retry: StoredValue<Option<TimeoutHandle>, LocalStorage>,
    /// The tab's error came from the stream, so a frame clears it.
    stream_failed: StoredValue<bool>,
    /// The current socket has opened.
    opened: StoredValue<bool>,
    /// A frame is on screen; until then the view shows nothing.
    pub has_frame: RwSignal<bool>,
    pub connected: RwSignal<bool>,
    /// Bumped when a closed view should connect again.
    pub reconnect: RwSignal<u64>,
}

impl LiveView {
    pub(super) fn new(img: NodeRef<html::Img>) -> Self {
        Self {
            img,
            socket: StoredValue::new_local(None),
            generation: StoredValue::new(0),
            epoch: StoredValue::new(0),
            shown_url: StoredValue::new(None),
            backoff: StoredValue::new(Backoff::default()),
            retry: StoredValue::new_local(None),
            stream_failed: StoredValue::new(false),
            opened: StoredValue::new(false),
            has_frame: RwSignal::new(false),
            connected: RwSignal::new(false),
            reconnect: RwSignal::new(0),
        }
    }

    /// Whether a socket is open or connecting.
    pub(super) fn running(self) -> bool {
        self.socket.try_with_value(Option::is_some).unwrap_or(false)
    }

    /// The tab's error no longer comes from the stream.
    pub(super) fn forget_error(self) {
        let _ = self.stream_failed.try_set_value(false);
    }

    /// Whether the tab's error came from the stream, which no longer
    /// speaks for it.
    pub(super) fn take_error(self) -> bool {
        self.stream_failed
            .try_update_value(std::mem::take)
            .unwrap_or(false)
    }

    /// The stream failed with `error`, which a frame clears.
    fn fail(self, error: &str, events: LiveEvents) {
        let _ = self.stream_failed.try_set_value(true);
        events.error.run(Some(error.to_string()));
    }

    /// Wait out the backoff, then tell the tab to look again (`mine`, the
    /// socket's generation, unless the view moved on since).
    fn wait_to_retry(self, mine: u64, events: LiveEvents) {
        let wait = self
            .backoff
            .try_update_value(Backoff::closed)
            .unwrap_or(BACKOFF_MAX_MS);
        let handle = set_timeout_with_handle(
            move || {
                if self.generation.try_get_value() == Some(mine) {
                    events.closed.run(mine);
                }
            },
            Duration::from_millis(wait),
        );
        if let Ok(handle) = handle {
            let _ = self.retry.try_set_value(Some(handle));
        }
    }

    /// Send owner input over the socket.
    pub(super) fn send(self, input: ComputerInput) {
        let message = ScreencastClientMessage::Input { input };
        if let Ok(text) = serde_json::to_string(&message) {
            self.send_text(&text);
        }
    }

    fn send_text(self, text: &str) {
        self.socket.with_value(|socket| {
            if let Some(socket) = socket {
                socket.send(text);
            }
        });
    }

    /// Connect again after a close, unless the view moved on since.
    pub(super) fn retry(self, token: u64) {
        if self.generation.try_get_value() == Some(token) {
            let _ = self
                .reconnect
                .try_update(|value| *value = value.wrapping_add(1));
        }
    }

    /// Clear the frame on screen and revoke its URL.
    pub(super) fn clear_frame(self) {
        self.bump_epoch();
        let _ = self.has_frame.try_set(false);
        if let Some(Some(url)) = self.shown_url.try_update_value(Option::take) {
            revoke(&url);
        }
        if let Some(Some(img)) = self.img.try_get_untracked() {
            let _ = img.remove_attribute("src");
        }
    }

    fn bump_epoch(self) {
        self.epoch
            .try_update_value(|value| *value = value.wrapping_add(1));
    }

    /// Close the socket, if any, and cancel a pending retry.
    pub(super) fn disconnect(self) {
        self.generation
            .try_update_value(|value| *value = value.wrapping_add(1));
        self.bump_epoch();
        if let Some(Some(handle)) = self.retry.try_update_value(Option::take) {
            handle.clear();
        }
        if let Some(Some(socket)) = self.socket.try_update_value(Option::take) {
            socket.close();
        }
        let _ = self.connected.try_set(false);
    }

    /// Show a frame, and acknowledge it once decoded: decoding works in a
    /// hidden tab, where painting waits.
    fn show_frame(self, blob: &web_sys::Blob, token: u64) {
        let ack = move || {
            if self.generation.try_get_value() == Some(token) {
                self.send_text(FRAME_ACK);
            }
        };
        let (Ok(url), Some(Some(img))) = (
            web_sys::Url::create_object_url_with_blob(blob),
            self.img.try_get_untracked(),
        ) else {
            ack();
            return;
        };
        let shown_epoch = self.epoch.get_value();
        img.set_src(&url);
        let decoded = wasm_bindgen_futures::JsFuture::from(img.decode());
        wasm_bindgen_futures::spawn_local(async move {
            // A frame that fails to decode is still acknowledged, so the
            // next one comes.
            let _ = decoded.await;
            if self.epoch.try_get_value() == Some(shown_epoch) {
                let _ = self.has_frame.try_set(true);
                if let Some(Some(previous)) = self
                    .shown_url
                    .try_update_value(|shown| shown.replace(url.clone()))
                {
                    if previous != url {
                        revoke(&previous);
                    }
                }
            } else {
                revoke(&url);
            }
            ack();
        });
    }

    /// Open the live view of browser `session`, replacing any socket.
    pub(super) fn connect(self, session: &str, events: LiveEvents) {
        self.disconnect();
        // A new socket's first frame resets the wait again.
        self.backoff.update_value(Backoff::new_socket);
        self.opened.set_value(false);
        let mine = self.generation.get_value();
        let Ok(socket) = web_sys::WebSocket::new(&crate::api::screencast_url(session)) else {
            self.fail(LIVE_VIEW_UNREACHABLE, events);
            self.wait_to_retry(mine, events);
            return;
        };
        socket.set_binary_type(web_sys::BinaryType::Blob);
        let current = move || self.generation.try_get_value() == Some(mine);
        let open = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            if current() {
                self.opened.set_value(true);
                self.connected.set(true);
            }
        });
        let message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                if !current() {
                    return;
                }
                let payload = event.data();
                if let Some(text) = payload.as_string() {
                    match serde_json::from_str::<ScreencastMessage>(&text) {
                        Ok(ScreencastMessage::State(next)) => events.state.run(next),
                        Ok(ScreencastMessage::Error { error }) => self.fail(&error, events),
                        // A newer server's message this app doesn't know.
                        Ok(ScreencastMessage::Other) | Err(_) => {}
                    }
                } else if let Ok(blob) = payload.dyn_into::<web_sys::Blob>() {
                    let first = self
                        .backoff
                        .try_update_value(Backoff::frame)
                        .unwrap_or(false);
                    if first && self.stream_failed.get_value() {
                        self.stream_failed.set_value(false);
                        events.error.run(None);
                    }
                    self.show_frame(&blob, mine);
                }
            },
        );
        let close =
            Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |_: web_sys::CloseEvent| {
                if !current() {
                    return;
                }
                self.connected.set(false);
                if let Some(error) = closed_error(self.opened.get_value()) {
                    self.fail(error, events);
                }
                self.wait_to_retry(mine, events);
            });
        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        self.socket.set_value(Some(Socket {
            socket,
            _open: open,
            _message: message,
            _close: close,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::closed_error;
    use bruhs_models::crew::LIVE_VIEW_UNREACHABLE;

    /// A socket that closes before it opens (the server is down, or it
    /// refused the upgrade) says the live view can't reach the server; one
    /// that opened has said why itself.
    #[test]
    fn a_socket_closed_before_it_opened_says_it_cant_reach_the_server() {
        assert_eq!(closed_error(false), Some(LIVE_VIEW_UNREACHABLE));
        assert_eq!(closed_error(true), None);
    }
}
