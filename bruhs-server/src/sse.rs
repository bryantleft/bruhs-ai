//! Lean, zero-temp-buffer Server-Sent Events (SSE) framing for `bruhs-server`.
//!
//! Replaces the vendored `third_party/axum` SSE fork: JSON payloads stream
//! directly through a bounded `BufWriter` into an owned `Vec<u8>` frame buffer
//! and convert zero-copy into `axum::body::Bytes`.

use std::{
    convert::Infallible,
    future::Future as _,
    io::{BufWriter, Write as _},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    body::{Body, Bytes},
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
};
use futures_core::Stream;
use serde::Serialize;

const DEFAULT_KEEP_ALIVE_SECS: u64 = 15;
const KEEP_ALIVE_BYTES: &[u8] = b":\n\n";
const JSON_BUFFER_BYTES: usize = 8 * 1024;

/// A single SSE frame being assembled into wire bytes.
#[derive(Debug, Default, Clone)]
pub struct Event {
    buffer: Vec<u8>,
    has_data: bool,
    has_event: bool,
}

impl Event {
    /// Set the `data:` field from a UTF-8 string.
    pub fn data<T: AsRef<str>>(self, data: T) -> Self {
        let mut writer = self.into_data_writer();
        writer.write_buf(data.as_ref().as_bytes());
        writer.into_event()
    }

    /// Serialize `data` as JSON directly into the SSE frame using an 8 KiB
    /// staging buffer (no full intermediate JSON `String`).
    pub fn json_data<T: Serialize>(self, data: T) -> Result<Self, serde_json::Error> {
        self.json_data_buffered(&data, JSON_BUFFER_BYTES)
    }

    /// Serialize `data` as JSON with an explicit staging buffer capacity.
    pub fn json_data_buffered<T: Serialize + ?Sized>(
        self,
        data: &T,
        capacity: usize,
    ) -> Result<Self, serde_json::Error> {
        let mut writer = self.into_data_writer();
        if capacity == 0 {
            serde_json::to_writer(writer.as_io_write(), data)?;
        } else {
            let mut buffered = BufWriter::with_capacity(capacity, writer.as_io_write());
            serde_json::to_writer(&mut buffered, data)?;
            if let Err(err) = buffered.flush() {
                return Err(serde_json::Error::io(err));
            }
        }
        Ok(writer.into_event())
    }

    /// Set the `event:` field name.
    pub fn event<T: AsRef<str>>(mut self, name: T) -> Self {
        let name = name.as_ref();
        assert!(
            !self.has_event,
            "Called `Event::event` multiple times on the same SSE event"
        );
        assert!(
            !name.as_bytes().iter().any(|b| matches!(b, b'\r' | b'\n')),
            "SSE event name cannot contain newlines or carriage returns"
        );
        self.has_event = true;
        self.buffer.extend_from_slice(b"event: ");
        self.buffer.extend_from_slice(name.as_bytes());
        self.buffer.push(b'\n');
        self
    }

    /// Begin streaming raw UTF-8 `data:` bytes directly into this frame.
    pub fn into_data_writer(self) -> EventDataWriter {
        EventDataWriter {
            event: self,
            data_written: false,
        }
    }

    /// Finalize this frame with its terminating blank line (`\n`) and return
    /// zero-copy `Bytes`.
    pub fn finalize(mut self) -> Bytes {
        self.buffer.push(b'\n');
        Bytes::from(self.buffer)
    }
}

/// Streaming byte writer that inserts SSE `data: ` line prefixes on the fly.
pub struct EventDataWriter {
    event: Event,
    data_written: bool,
}

impl EventDataWriter {
    /// Borrow this writer as [`std::io::Write`].
    pub fn as_io_write(&mut self) -> impl std::io::Write + '_ {
        struct Writer<'a>(&'a mut EventDataWriter);
        impl std::io::Write for Writer<'_> {
            #[inline]
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                Ok(self.0.write_buf(buf))
            }

            #[inline]
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        Writer(self)
    }

    /// Finish writing data and return the [`Event`].
    pub fn into_event(mut self) -> Event {
        if self.data_written {
            self.event.buffer.push(b'\n');
        }
        self.event
    }

    fn write_buf(&mut self, buf: &[u8]) -> usize {
        if buf.is_empty() {
            return 0;
        }
        if !std::mem::replace(&mut self.data_written, true) {
            assert!(
                !self.event.has_data,
                "Called `Event::data*` multiple times on the same SSE event"
            );
            self.event.has_data = true;
            self.event.buffer.extend_from_slice(b"data: ");
        }

        // Fast scan for `\n` or `\r` so ordinary single-line JSON copies in one
        // contiguous memcpy.
        let mut start = 0;
        for (idx, &byte) in buf.iter().enumerate() {
            if byte == b'\n' || byte == b'\r' {
                self.event.buffer.extend_from_slice(&buf[start..=idx]);
                self.event.buffer.extend_from_slice(b"data: ");
                start = idx + 1;
            }
        }
        self.event.buffer.extend_from_slice(&buf[start..]);
        buf.len()
    }
}

/// Keep-alive configuration for an [`Sse`] response stream.
#[derive(Debug, Clone)]
pub struct KeepAlive {
    interval: Duration,
}

impl KeepAlive {
    pub fn new() -> Self {
        Self {
            interval: Duration::from_secs(DEFAULT_KEEP_ALIVE_SECS),
        }
    }
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::new()
    }
}

/// An HTTP `text/event-stream` response backed by an arbitrary stream of [`Event`]s.
pub struct Sse<S> {
    stream: S,
    keep_alive: Option<KeepAlive>,
}

impl<S> Sse<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            keep_alive: None,
        }
    }

    pub fn keep_alive(mut self, keep_alive: KeepAlive) -> Self {
        self.keep_alive = Some(keep_alive);
        self
    }
}

struct SseBodyStream<S> {
    stream: Pin<Box<S>>,
    keep_alive: Option<(Duration, Pin<Box<tokio::time::Sleep>>)>,
}

impl<S> Stream for SseBodyStream<S>
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    type Item = Result<Bytes, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        match this.stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(event))) => {
                if let Some((interval, sleep)) = &mut this.keep_alive {
                    sleep
                        .as_mut()
                        .reset(tokio::time::Instant::now() + *interval);
                }
                Poll::Ready(Some(Ok(event.finalize())))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                if let Some((interval, sleep)) = &mut this.keep_alive {
                    if sleep.as_mut().poll(cx).is_ready() {
                        sleep
                            .as_mut()
                            .reset(tokio::time::Instant::now() + *interval);
                        return Poll::Ready(Some(Ok(Bytes::from_static(KEEP_ALIVE_BYTES))));
                    }
                }
                Poll::Pending
            }
        }
    }
}

impl<S> IntoResponse for Sse<S>
where
    S: Stream<Item = Result<Event, Infallible>> + Send + 'static,
{
    fn into_response(self) -> Response {
        let keep_alive = self.keep_alive.map(|ka| {
            let sleep = Box::pin(tokio::time::sleep(ka.interval));
            (ka.interval, sleep)
        });
        let body_stream = SseBodyStream {
            stream: Box::pin(self.stream),
            keep_alive,
        };
        let mut response = Response::new(Body::from_stream(body_stream));
        let headers = response.headers_mut();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        response
    }
}
