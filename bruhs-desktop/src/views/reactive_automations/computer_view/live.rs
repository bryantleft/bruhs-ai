//! The live view's socket: frames decoded off the main thread and
//! acknowledged once decoded, state and error messages applied to the tab,
//! and reconnects after the shared [`Backoff`].
use super::ComputerView;
#[cfg(not(test))]
use crate::app_state::spawn_tokio;
use crate::server_client::{Screencast, ScreencastEvent};
use bruhs_models::local_tools::Backoff;
use gpui::{Context, RenderImage, Task};
use std::{sync::Arc, time::Duration};

/// The largest frame the view decodes, in pixels a side.
const MAX_FRAME_SIDE: u32 = 4_096;

/// Decode one JPEG frame into GPUI's BGRA image, as the whiteboard decodes
/// a raster (`whiteboard/catalog/files.rs`).
fn decode_frame(bytes: &[u8]) -> Result<Arc<RenderImage>, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_FRAME_SIDE);
    limits.max_image_height = Some(MAX_FRAME_SIDE);
    limits.max_alloc = Some(bruhs_models::logic::attachments::MAX_IMAGE_TOTAL_BYTES);
    reader.limits(limits);
    let mut rgba = reader
        .decode()
        .map_err(|error| error.to_string())?
        .to_rgba8();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

/// The live view's socket and its reconnects.
#[derive(Default)]
pub(super) struct Live {
    /// The browser session this view watches, while it is connected,
    /// connecting or waiting to retry.
    pub(super) session: Option<String>,
    pub(super) screencast: Option<Screencast>,
    /// Opening the socket, then reading its events.
    pub(super) task: Option<Task<()>>,
    pub(super) retry: Option<Task<()>>,
    /// Bumped whenever a socket goes, so its late events do nothing.
    pub(super) generation: u64,
    pub(super) backoff: Backoff,
    pub(super) connected: bool,
}

impl ComputerView {
    /// Open the live view of browser `session`, replacing any socket.
    pub(super) fn connect(&mut self, session: String, cx: &mut Context<'_, Self>) {
        self.disconnect();
        self.live.session = Some(session.clone());
        #[cfg(test)]
        {
            // Tests never reach a server; they apply events themselves.
            let _ = (session, cx);
            self.live.connected = true;
        }
        #[cfg(not(test))]
        {
            let generation = self.live.generation;
            let client = self.client.clone();
            let opening = spawn_tokio(cx, async move { client.screencast(&session).await });
            self.live.task = Some(cx.spawn(async move |this, cx| {
                let events = match opening.await {
                    Ok(screencast) => {
                        let events = screencast.events.clone();
                        let kept = this.update(cx, |this, cx| {
                            if this.live.generation != generation {
                                return false;
                            }
                            this.live.screencast = Some(screencast);
                            this.live.connected = true;
                            cx.notify();
                            true
                        });
                        if !matches!(kept, Ok(true)) {
                            return;
                        }
                        events
                    }
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.live.generation == generation {
                                this.stream_failed = true;
                                this.error = Some(error.to_string());
                                this.closed(generation, cx);
                                cx.notify();
                            }
                        });
                        return;
                    }
                };
                loop {
                    // A closed channel means the socket has gone too.
                    let event = events.recv().await.unwrap_or(ScreencastEvent::Closed);
                    let closed = event == ScreencastEvent::Closed;
                    let current = this.update(cx, |this, cx| {
                        if this.live.generation != generation {
                            return false;
                        }
                        this.apply_event(event, cx);
                        true
                    });
                    if closed || !matches!(current, Ok(true)) {
                        break;
                    }
                }
            }));
        }
    }

    /// Close the socket, if any, and cancel a pending retry.
    pub(super) fn disconnect(&mut self) {
        self.live.generation = self.live.generation.wrapping_add(1);
        self.live.retry = None;
        self.live.task = None;
        self.live.screencast = None;
        self.live.session = None;
        self.live.connected = false;
        self.live.backoff.new_socket();
        // A frame of the old socket still decoding is dropped.
        self.epoch = self.epoch.wrapping_add(1);
        self.held = None;
        self.pending_move = None;
        self.pending_wheel = None;
    }

    /// Clear the frame on screen; a frame still decoding is dropped.
    pub(super) fn clear_frame(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.next_frame = None;
        let old = self.frame.take();
        self.retire(old);
    }

    /// The socket closed: wait, read the state again, then connect to what
    /// it says.
    fn closed(&mut self, generation: u64, cx: &mut Context<'_, Self>) {
        if generation != self.live.generation || self.live.session.is_none() {
            return;
        }
        self.live.screencast = None;
        self.live.connected = false;
        let wait = self.live.backoff.closed();
        self.live.retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(wait))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.live.generation == generation {
                    this.live.retry = None;
                    this.load(Some(generation), cx);
                }
            });
        }));
    }

    /// One event from the live view.
    pub(super) fn apply_event(&mut self, event: ScreencastEvent, cx: &mut Context<'_, Self>) {
        match event {
            ScreencastEvent::State(next) => {
                self.revision = self.revision.wrapping_add(1);
                self.state = Some(next);
                self.sync(cx);
            }
            ScreencastEvent::Error(error) => {
                self.stream_failed = true;
                self.error = Some(error);
            }
            ScreencastEvent::Frame(bytes) => {
                if self.live.backoff.frame() && self.stream_failed {
                    self.stream_failed = false;
                    self.error = None;
                }
                if self.decoding {
                    self.next_frame = Some(bytes);
                } else {
                    self.decode(bytes, cx);
                }
            }
            ScreencastEvent::Closed => self.closed(self.live.generation, cx),
        }
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn apply_event_for_test(
        &mut self,
        event: ScreencastEvent,
        _window: &mut gpui::Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.apply_event(event, cx);
    }

    /// Decode a frame off the main thread.
    fn decode(&mut self, bytes: Vec<u8>, cx: &mut Context<'_, Self>) {
        self.decoding = true;
        let (generation, epoch) = (self.live.generation, self.epoch);
        cx.spawn(async move |this, cx| {
            let decoded = cx
                .background_executor()
                .spawn(async move { decode_frame(&bytes) })
                .await;
            let _ = this.update(cx, |this, cx| this.decoded(generation, epoch, decoded, cx));
        })
        .detach();
    }

    /// A frame is decoded: show it, acknowledge it (a frame that failed to
    /// decode too, so the next comes) and decode the newest one waiting.
    fn decoded(
        &mut self,
        generation: u64,
        epoch: u64,
        decoded: Result<Arc<RenderImage>, String>,
        cx: &mut Context<'_, Self>,
    ) {
        self.decoding = false;
        if epoch == self.epoch {
            match decoded {
                Ok(image) => {
                    let old = self.frame.replace(image);
                    self.retire(old);
                }
                Err(error) => tracing::debug!("Live view frame skipped: {error}"),
            }
        }
        if generation == self.live.generation {
            self.ack();
        }
        if let Some(next) = self.next_frame.take() {
            self.decode(next, cx);
        }
        cx.notify();
    }

    /// Let go of a frame no longer shown: one that was drawn waits for the
    /// next draw to leave the atlas.
    pub(super) fn retire(&mut self, old: Option<Arc<RenderImage>>) {
        if std::mem::take(&mut self.frame_drawn) {
            self.retired.extend(old);
        }
    }

    fn ack(&mut self) {
        #[cfg(test)]
        {
            self.acks += 1;
        }
        if let Some(screencast) = &self.live.screencast {
            screencast.ack();
        }
    }
}
