//! A bruh's Computer: its state, the owner's requests and the live view.
//!
//! The live view is a WebSocket to the server's screencast route. It sends
//! no `Origin` header, as a native client, which the route allows. It runs
//! on the desktop's tokio runtime: frames and messages come out of
//! [`Screencast::events`], and acknowledgements and owner input queue to
//! the socket task. Dropping the [`Screencast`] closes the socket.
use super::{ServerClient, ServerError, REQUEST_TIMEOUT};
use bruhs_models::crew::{LIVE_VIEW_NEEDS_HTTP, LIVE_VIEW_UNREACHABLE};
use bruhs_models::local_tools::{
    screencast_url, ComputerInput, ComputerRequest, ComputerState, ScreencastClientMessage,
    ScreencastMessage, FRAME_ACK,
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{self, protocol::WebSocketConfig, Message};

/// The largest message the live view reads: one JPEG frame or a state.
const MAX_MESSAGE_BYTES: usize = 16 << 20;
/// Messages read ahead of the view. The server sends one frame at a time.
const EVENT_QUEUE: usize = 16;

/// What the live view hears from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreencastEvent {
    /// One JPEG frame. Call [`Screencast::ack`] once it is decoded.
    Frame(Vec<u8>),
    State(ComputerState),
    Error(String),
    /// The socket closed; open another to go on watching. A closed
    /// channel (`events.recv()` failing) means the same, since this last
    /// event is dropped when the queue is full.
    Closed,
}

/// What the view queues for the socket.
#[derive(Debug)]
enum Outgoing {
    Ack,
    Input(ComputerInput),
}

/// An open live view. Dropping it closes the socket.
pub struct Screencast {
    pub events: async_channel::Receiver<ScreencastEvent>,
    outgoing: async_channel::Sender<Outgoing>,
}

impl Screencast {
    /// The last frame is decoded: the server may send the next.
    pub(crate) fn ack(&self) {
        let _ = self.outgoing.try_send(Outgoing::Ack);
    }

    /// Owner input for the page.
    pub(crate) fn send(&self, input: ComputerInput) {
        let _ = self.outgoing.try_send(Outgoing::Input(input));
    }
}

impl Drop for Screencast {
    fn drop(&mut self) {
        self.outgoing.close();
    }
}

impl ServerClient {
    fn computer_url(&self, agent_id: &str) -> Result<reqwest::Url, ServerError> {
        let mut url = reqwest::Url::parse(&self.base)
            .map_err(|_| ServerError::Invalid("Invalid local server URL"))?;
        url.path_segments_mut()
            .map_err(|()| ServerError::Invalid("Invalid local server URL"))?
            .pop_if_empty()
            .extend(["api", "reactive", "agents", agent_id, "computer"]);
        Ok(url)
    }

    /// A bruh's Computer as the service has it now.
    pub async fn computer_state(&self, agent_id: &str) -> Result<ComputerState, ServerError> {
        let url = self.computer_url(agent_id)?;
        let request = "Read the bruh's computer";
        let response = Self::check(self.http()?.get(url).send().await, request).await?;
        Self::decode(response, request).await
    }

    /// Open, close, take control of, hand back or open a window for a
    /// bruh's Computer.
    pub async fn computer_request(
        &self,
        agent_id: &str,
        request: &ComputerRequest,
    ) -> Result<ComputerState, ServerError> {
        let url = self.computer_url(agent_id)?;
        let label = "Change the bruh's computer";
        let response =
            Self::check(self.http()?.post(url).json(request).send().await, label).await?;
        Self::decode(response, label).await
    }

    /// Open the live view of browser `session_id`. Must run on the tokio
    /// runtime, which then runs the socket.
    pub(crate) async fn screencast(&self, session_id: &str) -> Result<Screencast, ServerError> {
        let url = screencast_url(&self.base, session_id);
        // Built without TLS: the desktop talks to a local server.
        if !url.starts_with("ws://") {
            return Err(ServerError::Invalid(LIVE_VIEW_NEEDS_HTTP));
        }
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let connected = tokio::time::timeout(
            REQUEST_TIMEOUT,
            tokio_tungstenite::connect_async_with_config(url, Some(config), true),
        )
        .await;
        let socket = match connected {
            Ok(Ok((socket, _))) => socket,
            Ok(Err(tungstenite::Error::Http(response))) => {
                return Err(refused(&response));
            }
            Ok(Err(error)) => {
                tracing::debug!("Open the live view: {error}");
                return Err(ServerError::Invalid(LIVE_VIEW_UNREACHABLE));
            }
            Err(_) => {
                tracing::debug!("Open the live view: timed out");
                return Err(ServerError::Invalid(LIVE_VIEW_UNREACHABLE));
            }
        };
        let (events_tx, events) = async_channel::bounded(EVENT_QUEUE);
        let (outgoing, outgoing_rx) = async_channel::unbounded();
        tokio::spawn(pump(socket, events_tx, outgoing_rx));
        Ok(Screencast { events, outgoing })
    }
}

/// The server answered the upgrade with a refusal, such as four viewers
/// already watching; its body says why.
fn refused(response: &tungstenite::http::Response<Option<Vec<u8>>>) -> ServerError {
    let code = reqwest::StatusCode::from_u16(response.status().as_u16())
        .unwrap_or(reqwest::StatusCode::BAD_GATEWAY);
    let body = response
        .body()
        .as_deref()
        .map(|body| String::from_utf8_lossy(body).into_owned())
        .unwrap_or_default();
    ServerError::Status {
        request: "Open the live view".into(),
        code,
        body,
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Read frames and messages into `events`; write what the view queues.
/// Ends when either side closes.
async fn pump(
    socket: Socket,
    events: async_channel::Sender<ScreencastEvent>,
    outgoing: async_channel::Receiver<Outgoing>,
) {
    let (mut write, mut read) = socket.split();
    loop {
        tokio::select! {
            message = read.next() => {
                let event = match message {
                    Some(Ok(Message::Binary(frame))) => ScreencastEvent::Frame(Vec::from(frame)),
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ScreencastMessage>(&text) {
                            Ok(ScreencastMessage::State(state)) => ScreencastEvent::State(state),
                            Ok(ScreencastMessage::Error { error }) => ScreencastEvent::Error(error),
                            // A newer server's message this app doesn't know.
                            Ok(ScreencastMessage::Other) => continue,
                            Err(error) => {
                                tracing::debug!("Live view message skipped: {error}");
                                continue;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                };
                if events.send(event).await.is_err() {
                    break;
                }
            }
            queued = outgoing.recv() => {
                let message = match queued {
                    Ok(Outgoing::Ack) => Message::text(FRAME_ACK),
                    Ok(Outgoing::Input(input)) => {
                        match serde_json::to_string(&ScreencastClientMessage::Input { input }) {
                            Ok(text) => Message::text(text),
                            Err(error) => {
                                tracing::debug!("Live view input skipped: {error}");
                                continue;
                            }
                        }
                    }
                    // The view has gone.
                    Err(_) => {
                        let _ = tokio::time::timeout(REQUEST_TIMEOUT, write.close()).await;
                        return;
                    }
                };
                if write.send(message).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = events.try_send(ScreencastEvent::Closed);
}

#[cfg(test)]
mod tests {
    use super::{screencast_url, ScreencastEvent, ServerClient, ServerError};
    use bruhs_models::crew::{LIVE_VIEW_NEEDS_HTTP, LIVE_VIEW_UNREACHABLE};
    use bruhs_models::local_tools::{
        ComputerInput, ComputerRequest, ComputerState, Control, ScreencastClientMessage,
        ScreencastMessage, FRAME_ACK,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::{
        handshake::server::{Request, Response},
        Message,
    };

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn screencast_urls_swap_the_scheme_and_encode_the_session() {
        assert_eq!(
            screencast_url("http://127.0.0.1:3001/", "s 1"),
            "ws://127.0.0.1:3001/api/local-tools/browser/s%201/screencast"
        );
        assert_eq!(
            screencast_url("https://bruhs.localhost", "s"),
            "wss://bruhs.localhost/api/local-tools/browser/s/screencast"
        );
    }

    /// Answer one HTTP request with `status` and `body`, and hand back what
    /// was asked.
    async fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 16 * 1024];
            let read = socket.read(&mut buffer).await.unwrap();
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buffer[..read]).into_owned()
        });
        (base, asked)
    }

    #[test]
    fn computer_requests_go_to_the_agents_computer_route() {
        runtime().block_on(async {
            let (base, asked) = serve_once(
                "200 OK",
                r#"{"agent_id":"a b","session_id":"s","control":"owner"}"#,
            )
            .await;
            let client = ServerClient::new(&base, &base).unwrap();
            let state = client
                .computer_request("a b", &ComputerRequest::TakeControl)
                .await
                .unwrap();
            assert_eq!(state.control, Control::Owner);
            assert_eq!(state.session_id.as_deref(), Some("s"));
            let asked = asked.await.unwrap();
            assert!(
                asked.starts_with("POST /api/reactive/agents/a%20b/computer HTTP/1.1"),
                "{asked}"
            );
            assert!(asked.ends_with(r#"{"action":"take_control"}"#), "{asked}");

            let (base, asked) = serve_once("200 OK", r#"{"agent_id":"a"}"#).await;
            let client = ServerClient::new(&base, &base).unwrap();
            let state = client.computer_state("a").await.unwrap();
            assert_eq!(state.session_id, None);
            let asked = asked.await.unwrap();
            assert!(asked.starts_with("GET /api/reactive/agents/a/computer HTTP/1.1"));
        });
    }

    #[test]
    fn the_live_view_needs_a_plain_local_address() {
        runtime().block_on(async {
            let client =
                ServerClient::new("https://bruhs.localhost", "http://127.0.0.1:9").unwrap();
            let error = client.screencast("s").await.err().unwrap();
            assert_eq!(error.to_string(), LIVE_VIEW_NEEDS_HTTP);
            // Nothing listens here.
            let reservation = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", reservation.local_addr().unwrap());
            drop(reservation);
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.screencast("s").await.err().unwrap();
            assert_eq!(error.to_string(), LIVE_VIEW_UNREACHABLE);
        });
    }

    /// A refused upgrade keeps the server's reason, as other requests do.
    #[test]
    fn a_refused_live_view_says_why() {
        runtime().block_on(async {
            let (base, _asked) =
                serve_once("403 Forbidden", r#"{"error":"Four live views are open."}"#).await;
            let client = ServerClient::new(&base, &base).unwrap();
            let error = client.screencast("s").await.err().unwrap();
            assert!(
                matches!(
                    &error,
                    ServerError::Status { code, body, .. }
                        if code.as_u16() == 403 && body.contains("Four live views")
                ),
                "{error:?}"
            );
        });
    }

    #[test]
    fn the_live_view_reads_frames_writes_acks_and_input_and_closes_on_drop() {
        runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let (path_tx, path_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                #[expect(
                    clippy::result_large_err,
                    reason = "tungstenite's handshake callback returns its own error response"
                )]
                let heard = |request: &Request, response: Response| {
                    let origin = request.headers().get("origin").cloned();
                    let _ = path_tx.send((request.uri().path().to_string(), origin));
                    Ok(response)
                };
                let mut socket = tokio_tungstenite::accept_hdr_async(stream, heard)
                    .await
                    .unwrap();
                let state = ComputerState {
                    agent_id: "a".into(),
                    session_id: Some("s 1".into()),
                    ..ComputerState::default()
                };
                let text = serde_json::to_string(&ScreencastMessage::State(state)).unwrap();
                socket.send(Message::text(text)).await.unwrap();
                socket
                    .send(Message::text(r#"{"type":"newer_message"}"#))
                    .await
                    .unwrap();
                socket.send(Message::binary(vec![1, 2, 3])).await.unwrap();
                let error = ScreencastMessage::Error {
                    error: "Take control to use the page.".into(),
                };
                let text = serde_json::to_string(&error).unwrap();
                socket.send(Message::text(text)).await.unwrap();
                let mut heard = Vec::new();
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Text(text) => heard.push(text.to_string()),
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                heard
            });
            let client = ServerClient::new(&base, &base).unwrap();
            let screencast = client.screencast("s 1").await.unwrap();
            let (path, origin) = path_rx.await.unwrap();
            assert_eq!(path, "/api/local-tools/browser/s%201/screencast");
            assert_eq!(origin, None, "a native client sends no Origin");
            let ScreencastEvent::State(state) = screencast.events.recv().await.unwrap() else {
                panic!("the state comes first");
            };
            assert_eq!(state.session_id.as_deref(), Some("s 1"));
            assert_eq!(
                screencast.events.recv().await.unwrap(),
                ScreencastEvent::Frame(vec![1, 2, 3]),
                "an unknown message is skipped"
            );
            assert_eq!(
                screencast.events.recv().await.unwrap(),
                ScreencastEvent::Error("Take control to use the page.".into())
            );
            screencast.ack();
            let input = ComputerInput::Text { text: "hi".into() };
            screencast.send(input.clone());
            drop(screencast);
            let heard = server.await.unwrap();
            let sent = serde_json::to_string(&ScreencastClientMessage::Input { input }).unwrap();
            assert_eq!(heard, [FRAME_ACK.to_string(), sent], "then it closed");
        });
    }

    /// The server closing ends the view with `Closed`.
    #[test]
    fn a_closed_live_view_says_so() {
        runtime().block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                socket.close(None).await.unwrap();
            });
            let client = ServerClient::new(&base, &base).unwrap();
            let screencast = client.screencast("s").await.unwrap();
            assert_eq!(
                screencast.events.recv().await.unwrap(),
                ScreencastEvent::Closed
            );
        });
    }
}
