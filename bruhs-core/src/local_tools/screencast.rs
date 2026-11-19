//! The live view of a bruh's browser: Chrome's screencast on a second
//! connection to the page, fanned out to at most [`MAX_VIEWERS`] viewers.
//!
//! The hub keeps only the newest frame. Each viewer has at most one frame
//! in flight and skips whatever came while it was busy, so a slow viewer
//! never holds back Chrome or another viewer, and each holds at most one
//! frame.

use std::{sync::Arc, time::Duration};

use base64::Engine;
use bruhs_models::{
    crew::{LIVE_VIEW_ENDED, MAX_VIEWERS, MIN_FRAME_MS, SCREENCAST_QUALITY, VIEWERS_FULL},
    local_tools::{ComputerState, Viewport, COMPUTER_VIEWPORT},
    num::ToIntSaturating,
};
use serde_json::{json, Value};
use tokio::{sync::watch, time::Instant};

use super::{browser::Cdp, ProcessPolicy, ToolError};

/// How long an unwatched screencast keeps its connection, so a viewer that
/// comes straight back doesn't reconnect.
const UNWATCHED_END: Duration = Duration::from_secs(30);
/// How long a screencast that lost its page waits before connecting again,
/// while it is still watched.
const RECONNECT_AFTER: Duration = Duration::from_secs(1);

#[derive(Default)]
struct Latest {
    /// Bumped with each frame.
    sequence: u64,
    frame: Option<Arc<[u8]>>,
    ended: bool,
}

/// The newest frame of one bruh's browser and who watches it.
pub(super) struct ScreencastHub {
    /// The largest frame passed on, in bytes.
    budget: usize,
    latest: watch::Sender<Latest>,
    viewers: watch::Sender<usize>,
}

impl ScreencastHub {
    pub(super) fn new(budget: usize) -> Arc<Self> {
        Arc::new(Self {
            budget,
            latest: watch::channel(Latest::default()).0,
            viewers: watch::channel(0).0,
        })
    }

    /// Make `frame` the newest. A frame over the budget is dropped.
    pub(super) fn publish(&self, frame: Vec<u8>) {
        if frame.len() > self.budget {
            tracing::debug!(bytes = frame.len(), "dropped a live view frame over budget");
            return;
        }
        let frame: Arc<[u8]> = frame.into();
        self.latest.send_if_modified(|latest| {
            if latest.ended {
                return false;
            }
            latest.sequence = latest.sequence.wrapping_add(1);
            latest.frame = Some(frame);
            true
        });
    }

    /// A new viewer, which starts at the newest frame. At most
    /// [`MAX_VIEWERS`] at once.
    pub(super) fn viewer(self: &Arc<Self>) -> Result<Viewer, ToolError> {
        if self.latest.borrow().ended {
            return Err(ToolError::NotFound(LIVE_VIEW_ENDED.into()));
        }
        let mut admitted = false;
        self.viewers.send_if_modified(|count| {
            admitted = *count < MAX_VIEWERS;
            if admitted {
                *count = count.saturating_add(1);
            }
            admitted
        });
        if !admitted {
            return Err(ToolError::Refused(VIEWERS_FULL.into()));
        }
        Ok(Viewer {
            hub: Arc::clone(self),
            latest: self.latest.subscribe(),
            taken: 0,
            unacked: false,
        })
    }

    pub(super) fn watched(&self) -> bool {
        *self.viewers.borrow() > 0
    }

    /// The viewer count as it changes, which starts and stops the screencast.
    pub(super) fn watching(&self) -> watch::Receiver<usize> {
        self.viewers.subscribe()
    }

    /// No frame comes any more: every viewer's next frame is `None`.
    pub(super) fn end(&self) {
        self.latest.send_modify(|latest| {
            latest.ended = true;
            latest.frame = None;
        });
    }
}

/// One live view of a bruh's browser. Dropping it leaves.
pub struct Viewer {
    hub: Arc<ScreencastHub>,
    latest: watch::Receiver<Latest>,
    /// The sequence of the last frame this viewer took.
    taken: u64,
    unacked: bool,
}

impl Viewer {
    /// The newest frame this viewer hasn't had. Waits while the last one is
    /// unacked or nothing is new. `None` once the live view has ended.
    /// Cancel safe: a frame is taken only as it is returned.
    pub async fn next_frame(&mut self) -> Option<Arc<[u8]>> {
        loop {
            let frame = {
                let latest = self.latest.borrow_and_update();
                if latest.ended {
                    return None;
                }
                let fresh = !self.unacked && latest.sequence != self.taken;
                match &latest.frame {
                    Some(frame) if fresh => Some((latest.sequence, Arc::clone(frame))),
                    _ => None,
                }
            };
            if let Some((sequence, frame)) = frame {
                self.taken = sequence;
                self.unacked = true;
                return Some(frame);
            }
            if self.latest.changed().await.is_err() {
                return None;
            }
        }
    }

    /// The viewer has shown its frame and can take the next.
    pub fn ack(&mut self) {
        self.unacked = false;
    }
}

impl std::fmt::Debug for Viewer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Viewer")
            .field("taken", &self.taken)
            .field("unacked", &self.unacked)
            .finish_non_exhaustive()
    }
}

impl Drop for Viewer {
    fn drop(&mut self) {
        self.hub
            .viewers
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

/// A live view of a bruh's browser: its frames and its state as they change.
pub struct ComputerWatch {
    pub agent_id: String,
    pub viewer: Viewer,
    pub state: watch::Receiver<ComputerState>,
}

/// An open page's screencast, run whenever someone watches. Dropping it
/// stops the screencast and its connection.
pub(super) struct ScreencastTask(tokio::task::JoinHandle<()>);

impl ScreencastTask {
    /// Follow the page at `socket_url` for as long as it is open: connect
    /// while watched, and again if the connection drops while watched.
    pub(super) fn start(
        socket_url: String,
        hub: Arc<ScreencastHub>,
        state: watch::Sender<ComputerState>,
        policy: ProcessPolicy,
    ) -> Self {
        Self(tokio::spawn(async move {
            let mut watching = hub.watching();
            loop {
                if watching.wait_for(|count| *count > 0).await.is_err() {
                    return;
                }
                run_screencast(socket_url.clone(), Arc::clone(&hub), state.clone(), policy).await;
                if hub.watched() {
                    tokio::time::sleep(RECONNECT_AFTER).await;
                }
            }
        }))
    }
}

impl Drop for ScreencastTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// One CDP connection to a page target: its screencast while watched, its
/// main-frame address, and while headed its size from the frames. Ends
/// when the page closes or nobody has watched for [`UNWATCHED_END`].
pub(super) async fn run_screencast(
    socket_url: String,
    hub: Arc<ScreencastHub>,
    state: watch::Sender<ComputerState>,
    policy: ProcessPolicy,
) {
    let result = async {
        let mut cdp = Cdp::connect(&socket_url, policy).await?;
        stream(&mut cdp, &hub, &state).await
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(%error, "live view connection ended");
    }
}

/// The screencast's read loop. Every message is read here and replies are
/// matched by id, so no frame or navigation is lost to a waiting call.
async fn stream(
    cdp: &mut Cdp,
    hub: &ScreencastHub,
    state: &watch::Sender<ComputerState>,
) -> Result<(), ToolError> {
    let mut requests = Requests::default();
    requests.sent(cdp.send("Page.enable", json!({})).await?, "Page.enable");
    let mut watching = hub.watching();
    let mut streaming = false;
    let mut unwatched_since = Some(Instant::now());
    // The frame Chrome waits on, by its session id.
    let mut unacked: Option<Value> = None;
    let mut last_ack: Option<Instant> = None;
    let mut main_frame: Option<String> = None;
    let gap = Duration::from_millis(MIN_FRAME_MS);
    loop {
        let watched = *watching.borrow_and_update() > 0;
        if watched != streaming {
            streaming = watched;
            if watched {
                let start = json!({
                    "format": "jpeg",
                    "quality": SCREENCAST_QUALITY,
                    "maxWidth": COMPUTER_VIEWPORT.width,
                    "maxHeight": COMPUTER_VIEWPORT.height,
                });
                let id = cdp.send("Page.startScreencast", start).await?;
                requests.sent(id, "Page.startScreencast");
                unwatched_since = None;
            } else {
                let id = cdp.send("Page.stopScreencast", json!({})).await?;
                requests.sent(id, "Page.stopScreencast");
                unacked = None;
                unwatched_since = Some(Instant::now());
            }
        }
        let now = Instant::now();
        let ack_at = last_ack.map_or(now, |at| (at + gap).max(now));
        let end_at = unwatched_since.map_or(now, |since| since + UNWATCHED_END);
        tokio::select! {
            changed = watching.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
            }
            message = cdp.next_message() => {
                let message = message?;
                match message["method"].as_str() {
                    Some("Page.screencastFrame") if streaming => {
                        let params = &message["params"];
                        show_frame(hub, state, params);
                        unacked = Some(params["sessionId"].clone());
                    }
                    Some("Page.frameNavigated") => {
                        let frame = &message["params"]["frame"];
                        if frame["parentId"].is_null() {
                            main_frame = frame["id"].as_str().map(str::to_owned);
                            follow(state, frame["url"].as_str());
                        }
                    }
                    Some("Page.navigatedWithinDocument") => {
                        let params = &message["params"];
                        if main_frame.is_some() && params["frameId"].as_str() == main_frame.as_deref() {
                            follow(state, params["url"].as_str());
                        }
                    }
                    Some(_) => {}
                    None => requests.answered(&message),
                }
            }
            () = tokio::time::sleep_until(ack_at), if unacked.is_some() => {
                if let Some(session) = unacked.take() {
                    let id = cdp
                        .send("Page.screencastFrameAck", json!({"sessionId": session}))
                        .await?;
                    requests.sent(id, "Page.screencastFrameAck");
                    last_ack = Some(Instant::now());
                }
            }
            () = tokio::time::sleep_until(end_at), if unwatched_since.is_some() => return Ok(()),
        }
    }
}

/// Publish a screencast frame, and while headed take the view's size from
/// it: the window may have been resized since it was measured.
fn show_frame(hub: &ScreencastHub, state: &watch::Sender<ComputerState>, params: &Value) {
    let frame = params["data"]
        .as_str()
        .map(|data| base64::engine::general_purpose::STANDARD.decode(data));
    match frame {
        Some(Ok(frame)) if !frame.is_empty() => hub.publish(frame),
        _ => tracing::debug!("Chrome sent an unreadable live view frame"),
    }
    let side = |name: &str| {
        params["metadata"][name]
            .as_f64()
            .map(|px| px.round().to_u32_saturating())
            .filter(|px| *px > 0)
    };
    if let (Some(width), Some(height)) = (side("deviceWidth"), side("deviceHeight")) {
        let size = Viewport { width, height };
        state.send_if_modified(|state| {
            let resized = state.headed && state.viewport != size;
            if resized {
                state.viewport = size;
            }
            resized
        });
    }
}

/// The page's main frame is now at `url`.
fn follow(state: &watch::Sender<ComputerState>, url: Option<&str>) {
    let Some(url) = url else {
        return;
    };
    state.send_if_modified(|state| {
        let moved = state.url.as_deref() != Some(url);
        if moved {
            state.url = Some(url.to_owned());
        }
        moved
    });
}

/// The screencast's own commands awaiting replies, so a refusal is logged
/// under its name. Bounded by the commands in flight.
#[derive(Default)]
struct Requests(Vec<(u64, &'static str)>);

impl Requests {
    fn sent(&mut self, id: u64, method: &'static str) {
        self.0.push((id, method));
    }

    fn answered(&mut self, reply: &Value) {
        let Some(id) = reply["id"].as_u64() else {
            return;
        };
        if let Some(index) = self.0.iter().position(|(sent, _)| *sent == id) {
            let (_, method) = self.0.swap_remove(index);
            if let Some(error) = reply.get("error") {
                tracing::warn!(%method, %error, "Chrome refused a live view command");
            }
        }
    }
}

#[cfg(test)]
pub(super) mod fake_chrome {
    //! A page target that answers every command and sends the events a test
    //! gives it, logging what each connection sent.

    use std::{
        sync::{Arc, Mutex, PoisonError},
        time::Duration,
    };

    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::{net::TcpListener, sync::mpsc, time::Instant};
    use tokio_tungstenite::tungstenite::Message;

    const CLOSED: &str = "<closed>";

    #[derive(Debug, Clone)]
    pub struct Call {
        pub at: Instant,
        pub connection: usize,
        pub method: String,
        pub params: Value,
    }

    enum Command {
        Event(Value),
        HangUp,
    }

    type Connections = Arc<Mutex<Vec<mpsc::UnboundedSender<Command>>>>;

    pub struct FakeChrome {
        pub url: String,
        log: Vec<Call>,
        calls: mpsc::UnboundedReceiver<Call>,
        connections: Connections,
        server: tokio::task::JoinHandle<()>,
    }

    impl Drop for FakeChrome {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    impl FakeChrome {
        pub async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let (record, calls) = mpsc::unbounded_channel();
            let connections = Connections::default();
            let server = tokio::spawn(serve(listener, record, Arc::clone(&connections)));
            Self {
                url,
                log: Vec::new(),
                calls,
                connections,
                server,
            }
        }

        /// Read calls until `check` holds, failing after five seconds.
        pub async fn wait_for(&mut self, what: &str, check: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !check(self) {
                match tokio::time::timeout_at(deadline, self.calls.recv()).await {
                    Ok(Some(call)) => self.log.push(call),
                    _ => panic!("never saw {what}: {:#?}", self.log),
                }
            }
        }

        /// Wait `time`, then take in every call so far.
        pub async fn settle(&mut self, time: Duration) {
            tokio::time::sleep(time).await;
            while let Ok(call) = self.calls.try_recv() {
                self.log.push(call);
            }
        }

        pub fn calls(&self, method: &str) -> Vec<Call> {
            self.log
                .iter()
                .filter(|call| call.method == method)
                .cloned()
                .collect()
        }

        pub fn count(&self, method: &str) -> usize {
            self.calls(method).len()
        }

        /// Calls of `method` on the `connection`th connection.
        pub fn count_on(&self, connection: usize, method: &str) -> usize {
            self.log
                .iter()
                .filter(|call| call.connection == connection && call.method == method)
                .count()
        }

        /// Whether the client closed the `connection`th connection.
        pub fn closed(&self, connection: usize) -> bool {
            self.count_on(connection, CLOSED) > 0
        }

        /// Send `event` on the latest connection.
        pub fn send(&self, event: Value) {
            self.latest(Command::Event(event));
        }

        /// Close the latest connection, as when the page goes.
        pub fn hang_up(&self) {
            self.latest(Command::HangUp);
        }

        fn latest(&self, command: Command) {
            let connections = self
                .connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let latest = connections.last().expect("a connection").clone();
            drop(connections);
            let _ = latest.send(command);
        }
    }

    async fn serve(
        listener: TcpListener,
        record: mpsc::UnboundedSender<Call>,
        connections: Connections,
    ) {
        // Dropped with the server, which ends every connection.
        let mut tasks = tokio::task::JoinSet::new();
        for connection in 0.. {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let (commands, received) = mpsc::unbounded_channel();
            connections
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(commands);
            tasks.spawn(answer(stream, connection, record.clone(), received));
        }
    }

    async fn answer(
        stream: tokio::net::TcpStream,
        connection: usize,
        record: mpsc::UnboundedSender<Call>,
        mut commands: mpsc::UnboundedReceiver<Command>,
    ) {
        let Ok(mut socket) = tokio_tungstenite::accept_async(stream).await else {
            return;
        };
        loop {
            tokio::select! {
                message = socket.next() => {
                    let Some(Ok(Message::Text(text))) = message else {
                        break;
                    };
                    let request: Value = serde_json::from_str(&text).unwrap();
                    let _ = record.send(Call {
                        at: Instant::now(),
                        connection,
                        method: request["method"].as_str().unwrap_or_default().into(),
                        params: request["params"].clone(),
                    });
                    let reply = json!({"id": request["id"], "result": {}});
                    if socket.send(Message::Text(reply.to_string().into())).await.is_err() {
                        break;
                    }
                }
                command = commands.recv() => match command {
                    Some(Command::Event(event)) => {
                        if socket.send(Message::Text(event.to_string().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Command::HangUp) | None => {
                        let _ = socket.close(None).await;
                        break;
                    }
                },
            }
        }
        let _ = record.send(Call {
            at: Instant::now(),
            connection,
            method: CLOSED.into(),
            params: Value::Null,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use base64::Engine;
    use bruhs_models::{
        crew::{MIN_FRAME_MS, VIEWERS_FULL},
        local_tools::{ComputerState, Viewport, COMPUTER_VIEWPORT},
    };
    use serde_json::{json, Value};
    use tokio::sync::watch;

    use super::{fake_chrome::FakeChrome, run_screencast, ScreencastHub};
    use crate::local_tools::{ProcessPolicy, ToolError};

    const WAIT: Duration = Duration::from_millis(20);

    fn frame(session: u64, data: &[u8]) -> Value {
        sized_frame(session, data, 1280.0, 800.0)
    }

    fn sized_frame(session: u64, data: &[u8], width: f64, height: f64) -> Value {
        json!({
            "method": "Page.screencastFrame",
            "params": {
                "data": base64::engine::general_purpose::STANDARD.encode(data),
                "metadata": {"deviceWidth": width, "deviceHeight": height, "offsetTop": 0, "pageScaleFactor": 1, "scrollOffsetX": 0, "scrollOffsetY": 0},
                "sessionId": session,
            },
        })
    }

    #[tokio::test]
    async fn a_viewer_waits_for_its_ack_and_then_gets_the_newest_frame_not_a_backlog() {
        let hub = ScreencastHub::new(1 << 20);
        let mut viewer = hub.viewer().unwrap();
        hub.publish(vec![1]);
        assert_eq!(&*viewer.next_frame().await.unwrap(), &[1]);
        hub.publish(vec![2]);
        hub.publish(vec![3]);
        assert!(
            tokio::time::timeout(WAIT, viewer.next_frame())
                .await
                .is_err(),
            "a frame is still unacked"
        );
        viewer.ack();
        assert_eq!(&*viewer.next_frame().await.unwrap(), &[3]);
        viewer.ack();
        assert!(
            tokio::time::timeout(WAIT, viewer.next_frame())
                .await
                .is_err(),
            "nothing new"
        );
    }

    #[tokio::test]
    async fn viewers_are_independent_and_a_fifth_is_refused() {
        let hub = ScreencastHub::new(4);
        let mut fast = hub.viewer().unwrap();
        let mut slow = hub.viewer().unwrap();
        hub.publish(vec![1]);
        assert_eq!(&*slow.next_frame().await.unwrap(), &[1]);
        // The slow viewer never acks; the fast one keeps getting frames.
        for n in 1..=3u8 {
            if n > 1 {
                hub.publish(vec![n]);
            }
            assert_eq!(&*fast.next_frame().await.unwrap(), &[n]);
            fast.ack();
        }
        assert!(tokio::time::timeout(WAIT, slow.next_frame()).await.is_err());
        let third = hub.viewer().unwrap();
        let _fourth = hub.viewer().unwrap();
        assert_eq!(*hub.watching().borrow(), 4);
        assert!(hub.watched());
        let error = hub.viewer().unwrap_err();
        assert!(
            matches!(&error, ToolError::Refused(message) if message == VIEWERS_FULL),
            "{error:?}"
        );
        drop(third);
        assert_eq!(*hub.watching().borrow(), 3);
        let _again = hub.viewer().unwrap();
        // A frame over the budget is never delivered.
        hub.publish(vec![0; 5]);
        assert!(tokio::time::timeout(WAIT, fast.next_frame()).await.is_err());
        hub.publish(vec![9]);
        assert_eq!(&*fast.next_frame().await.unwrap(), &[9]);
    }

    #[tokio::test]
    async fn a_new_viewer_sees_the_current_frame_and_every_viewer_hears_the_end() {
        let hub = ScreencastHub::new(1 << 20);
        hub.publish(vec![7]);
        let mut late = hub.viewer().unwrap();
        assert_eq!(&*late.next_frame().await.unwrap(), &[7]);
        let mut fresh = hub.viewer().unwrap();
        hub.end();
        // Ended, even with a frame unacked.
        assert!(late.next_frame().await.is_none());
        assert!(fresh.next_frame().await.is_none());
        assert!(hub.viewer().is_err());
        drop((late, fresh));
        assert!(!hub.watched());
    }

    #[tokio::test]
    async fn chrome_frames_are_acked_no_faster_than_ten_a_second_and_only_while_watched() {
        let mut chrome = FakeChrome::start().await;
        let hub = ScreencastHub::new(1 << 20);
        let (state, _) = watch::channel(ComputerState::default());
        let task = tokio::spawn(run_screencast(
            chrome.url.clone(),
            Arc::clone(&hub),
            state,
            ProcessPolicy::default(),
        ));
        chrome
            .wait_for("Page.enable", |chrome| chrome.count("Page.enable") == 1)
            .await;
        chrome.settle(Duration::from_millis(50)).await;
        assert_eq!(chrome.count("Page.startScreencast"), 0, "nobody watches");
        // This viewer never acks while Chrome sends frames.
        let mut viewer = hub.viewer().unwrap();
        chrome
            .wait_for("the screencast to start", |chrome| {
                chrome.count("Page.startScreencast") == 1
            })
            .await;
        assert_eq!(
            chrome.calls("Page.startScreencast")[0].params,
            json!({"format": "jpeg", "quality": 70, "maxWidth": 1280, "maxHeight": 800})
        );
        chrome.send(frame(1, b"one"));
        tokio::time::sleep(Duration::from_millis(10)).await;
        chrome.send(frame(2, b"two"));
        chrome
            .wait_for("two acks", |chrome| {
                chrome.count("Page.screencastFrameAck") == 2
            })
            .await;
        let acks = chrome.calls("Page.screencastFrameAck");
        assert_eq!(acks[0].params, json!({"sessionId": 1}));
        assert_eq!(acks[1].params, json!({"sessionId": 2}));
        let gap = acks[1].at.duration_since(acks[0].at);
        // Less a loopback hop, which can delay the first ack's arrival.
        assert!(
            gap >= Duration::from_millis(MIN_FRAME_MS - 2),
            "acked {gap:?} apart"
        );
        assert_eq!(&*viewer.next_frame().await.unwrap(), b"two");
        drop(viewer);
        chrome
            .wait_for("the screencast to stop", |chrome| {
                chrome.count("Page.stopScreencast") == 1
            })
            .await;
        task.abort();
    }

    #[tokio::test]
    async fn the_url_follows_main_frame_navigations() {
        let mut chrome = FakeChrome::start().await;
        let hub = ScreencastHub::new(1 << 20);
        let (state, mut states) = watch::channel(ComputerState::default());
        let task = tokio::spawn(run_screencast(
            chrome.url.clone(),
            hub,
            state,
            ProcessPolicy::default(),
        ));
        chrome
            .wait_for("Page.enable", |chrome| chrome.count("Page.enable") == 1)
            .await;
        chrome.send(json!({"method": "Page.frameNavigated", "params": {"frame": {"id": "child", "parentId": "main", "url": "https://ads.test/"}}}));
        chrome.send(json!({"method": "Page.frameNavigated", "params": {"frame": {"id": "main", "url": "https://a.test/"}}}));
        let seen = tokio::time::timeout(
            Duration::from_secs(5),
            states.wait_for(|state| state.url.is_some()),
        )
        .await
        .unwrap()
        .unwrap()
        .url
        .clone();
        assert_eq!(seen.as_deref(), Some("https://a.test/"));
        chrome.send(json!({"method": "Page.navigatedWithinDocument", "params": {"frameId": "child", "url": "https://ads.test/#x"}}));
        chrome.send(json!({"method": "Page.navigatedWithinDocument", "params": {"frameId": "main", "url": "https://a.test/#inbox"}}));
        let seen = tokio::time::timeout(
            Duration::from_secs(5),
            states.wait_for(|state| state.url.as_deref() != Some("https://a.test/")),
        )
        .await
        .unwrap()
        .unwrap()
        .url
        .clone();
        assert_eq!(seen.as_deref(), Some("https://a.test/#inbox"));
        task.abort();
    }

    #[tokio::test]
    async fn a_window_is_sized_from_its_frames() {
        for headed in [true, false] {
            let mut chrome = FakeChrome::start().await;
            let hub = ScreencastHub::new(1 << 20);
            let (state, states) = watch::channel(ComputerState {
                headed,
                ..ComputerState::default()
            });
            let task = tokio::spawn(run_screencast(
                chrome.url.clone(),
                Arc::clone(&hub),
                state,
                ProcessPolicy::default(),
            ));
            let mut viewer = hub.viewer().unwrap();
            chrome
                .wait_for("the screencast to start", |chrome| {
                    chrome.count("Page.startScreencast") == 1
                })
                .await;
            chrome.send(sized_frame(1, b"x", 1000.0, 640.0));
            assert_eq!(&*viewer.next_frame().await.unwrap(), b"x");
            chrome
                .wait_for("the ack", |chrome| {
                    chrome.count("Page.screencastFrameAck") == 1
                })
                .await;
            let viewport = states.borrow().viewport;
            if headed {
                assert_eq!(
                    viewport,
                    Viewport {
                        width: 1000,
                        height: 640
                    }
                );
            } else {
                assert_eq!(viewport, COMPUTER_VIEWPORT, "headless stays 1280 by 800");
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn the_screencast_ends_with_its_page() {
        let mut chrome = FakeChrome::start().await;
        let hub = ScreencastHub::new(1 << 20);
        let (state, _) = watch::channel(ComputerState::default());
        let task = tokio::spawn(run_screencast(
            chrome.url.clone(),
            Arc::clone(&hub),
            state,
            ProcessPolicy::default(),
        ));
        let _viewer = hub.viewer().unwrap();
        chrome
            .wait_for("the screencast to start", |chrome| {
                chrome.count("Page.startScreencast") == 1
            })
            .await;
        chrome.hang_up();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the screencast ends when its page goes")
            .unwrap();
    }
}
