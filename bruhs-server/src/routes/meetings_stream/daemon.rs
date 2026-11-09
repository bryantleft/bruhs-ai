//! One reconnecting daemon subscription, polled independently of server events.
use bruhs_core::capture_client::DaemonEndpoint;
use futures_core::Stream;
use std::time::Duration;
use tokio_stream::StreamExt;

#[derive(Clone, Copy)]
pub(super) struct RetryPolicy {
    pub initial: Duration,
    pub maximum: Duration,
    pub headers_timeout: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_secs(1),
            maximum: Duration::from_secs(30),
            headers_timeout: Duration::from_secs(3),
        }
    }
}

// A broken upstream must not grow an unterminated frame without limit.
const MAX_BUFFER_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn events(daemon: DaemonEndpoint, retry: RetryPolicy) -> impl Stream<Item = String> {
    async_stream::stream! {
        let url = match daemon.url(&["events"]) {
            Ok(url) => url,
            Err(error) => {
                tracing::warn!("capture stream unavailable: {error}");
                return;
            }
        };
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(60 * 60))
            .build()
        {
            Ok(client) => client,
            Err(error) => {
                tracing::warn!("capture stream client unavailable: {error}");
                return;
            }
        };
        let mut delay = retry.initial;
        loop {
            // The token is re-read per attempt: a restarted daemon has a new one.
            let request = daemon.authorize(client.get(url.clone())).await;
            let response = tokio::time::timeout(retry.headers_timeout, request.send()).await;
            if let Ok(Ok(response)) = response {
                if response.status().is_success() {
                    // Downstream connections stay open while upstream reconnects.
                    // Tell them to reconcile the disconnected interval as well.
                    yield r#"{"kind":"stream_opened"}"#.to_string();
                    let mut bytes = response.bytes_stream();
                    let mut frames = FrameBuffer::default();
                    while let Some(Ok(chunk)) = bytes.next().await {
                        if frames.len().saturating_add(chunk.len()) > MAX_BUFFER_BYTES {
                            tracing::warn!("capture stream exceeded the frame buffer limit");
                            break;
                        }
                        frames.push(&chunk);
                        while let Some(frame) = frames.next_frame() {
                            if let Some(payload) = parse_sse_data(&frame) {
                                delay = retry.initial;
                                yield payload;
                            }
                        }
                    }
                }
            }
            // Dropping a subscriber cancels this timer/request; no detached retry task.
            tokio::time::sleep(delay).await;
            delay = delay.saturating_mul(2).min(retry.maximum);
        }
    }
}

/// Bytes of an SSE response split into frames at blank lines (`\n\n` or
/// `\r\n\r\n`). Remembers how far it has looked, so a frame arriving in
/// many small chunks is scanned once, not once per chunk.
#[derive(Default)]
struct FrameBuffer {
    buffer: Vec<u8>,
    /// Bytes before this offset hold no frame end.
    scanned: usize,
}

impl FrameBuffer {
    fn len(&self) -> usize {
        self.buffer.len()
    }

    fn push(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
    }

    /// The next complete frame, including its terminator.
    fn next_frame(&mut self) -> Option<Vec<u8>> {
        // A terminator can straddle the previous scan's end by up to three
        // bytes (`\r\n\r` | `\n`).
        let from = self.scanned.saturating_sub(3);
        match frame_end(&self.buffer, from) {
            Some(end) => {
                self.scanned = 0;
                Some(self.buffer.drain(..end).collect())
            }
            None => {
                self.scanned = self.buffer.len();
                None
            }
        }
    }
}

/// End (exclusive) of the first blank-line terminator at or after `from`.
fn frame_end(buffer: &[u8], from: usize) -> Option<usize> {
    (from..buffer.len()).find_map(|i| {
        let rest = &buffer[i..];
        if rest.starts_with(b"\n\n") {
            Some(i + 2)
        } else if rest.starts_with(b"\r\n\r\n") {
            Some(i + 4)
        } else {
            None
        }
    })
}

fn parse_sse_data(frame: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(frame).ok()?;
    let mut lines = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data:")
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
        })
        .peekable();
    lines.peek()?;
    Some(lines.collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sse_data_extracts_payload() {
        assert_eq!(
            parse_sse_data(b"data: {\"kind\":\"started\"}\n\n").as_deref(),
            Some("{\"kind\":\"started\"}")
        );
    }

    #[test]
    fn parse_sse_data_handles_no_space_after_colon() {
        assert_eq!(
            parse_sse_data(b"data:{\"kind\":\"ended\"}\n\n").as_deref(),
            Some("{\"kind\":\"ended\"}")
        );
    }

    #[test]
    fn frame_end_accepts_lf_and_crlf_blank_lines() {
        assert_eq!(frame_end(b"data: x\n\nrest", 0), Some(9));
        assert_eq!(frame_end(b"data: x\r\n\r\nrest", 0), Some(11));
        assert_eq!(frame_end(b"data: x\r\ndata: y\r\n", 0), None);
        assert_eq!(frame_end(b"no terminator", 0), None);
    }

    fn frames(chunks: &[&[u8]]) -> Vec<Option<String>> {
        let mut buffer = FrameBuffer::default();
        let mut out = Vec::new();
        for chunk in chunks {
            buffer.push(chunk);
            while let Some(frame) = buffer.next_frame() {
                out.push(parse_sse_data(&frame));
            }
        }
        out
    }

    /// Both line-ending styles split frames, including a terminator that
    /// arrives one byte at a time across chunks.
    #[test]
    fn frames_split_on_lf_and_crlf_across_chunks() {
        let lf: Vec<&[u8]> = vec![b"data: one\n", b"\ndata: two\n\n"];
        assert_eq!(frames(&lf), [Some("one".into()), Some("two".into())]);
        let crlf = b"data: one\r\n\r\ndata: two\r\n\r\n";
        let byte_at_a_time: Vec<&[u8]> = crlf.chunks(1).collect();
        assert_eq!(
            frames(&byte_at_a_time),
            [Some("one".into()), Some("two".into())]
        );
        let mixed: Vec<&[u8]> = vec![b": ping\r\n\r\ndata: {}\n\n"];
        assert_eq!(frames(&mixed), [None, Some("{}".into())]);
    }

    /// A long frame in small chunks is scanned once: the scan offset only
    /// moves forward until a frame completes.
    #[test]
    fn a_partial_frame_is_not_rescanned() {
        let mut buffer = FrameBuffer::default();
        for _ in 0..1000 {
            buffer.push(b"data: xxxxxxxx");
            assert!(buffer.next_frame().is_none());
            assert_eq!(buffer.scanned, buffer.len());
        }
        buffer.push(b"\n\n");
        assert!(buffer.next_frame().is_some());
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn data_lines_preserve_content_whitespace_and_ignore_comments() {
        assert_eq!(
            parse_sse_data(b": heartbeat\ndata:   one\ndata: two\n\n").as_deref(),
            Some("  one\ntwo")
        );
        assert_eq!(parse_sse_data(b": heartbeat\n\n"), None);
    }
}
