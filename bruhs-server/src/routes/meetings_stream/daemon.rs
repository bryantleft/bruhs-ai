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
                            if let Some(payload) = parse_sse_data(frame) {
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
    consumed: usize,
    /// Bytes before this offset hold no frame end.
    scanned: usize,
}

impl FrameBuffer {
    fn len(&self) -> usize {
        self.buffer.len() - self.consumed
    }

    fn push(&mut self, chunk: &[u8]) {
        // Move an incomplete tail once per chunk, rather than after every frame.
        if self.consumed != 0 {
            self.buffer.drain(..self.consumed);
            self.scanned -= self.consumed;
            self.consumed = 0;
        }
        self.buffer.extend_from_slice(chunk);
    }

    /// The next complete frame, including its terminator.
    fn next_frame(&mut self) -> Option<&[u8]> {
        let remaining = &self.buffer[self.consumed..];
        let from = self.scanned - self.consumed;
        match frame_end(remaining, from) {
            Some(end) => {
                let start = self.consumed;
                self.consumed += end;
                self.scanned = self.consumed;
                Some(&self.buffer[start..self.consumed])
            }
            None => {
                self.scanned = self.buffer.len();
                None
            }
        }
    }
}

/// Scan new bytes, checking a terminator only when its final LF arrives.
fn frame_end(buffer: &[u8], from: usize) -> Option<usize> {
    buffer
        .get(from..)?
        .iter()
        .enumerate()
        .find_map(|(i, &byte)| {
            if byte != b'\n' {
                return None;
            }
            let end = from + i + 1;
            let prefix = &buffer[..end];
            (prefix.ends_with(b"\n\n") || prefix.ends_with(b"\r\n\r\n")).then_some(end)
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
                out.push(parse_sse_data(frame));
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

    fn reference_frames(mut input: &[u8]) -> (Vec<Vec<u8>>, usize) {
        let mut frames = Vec::new();
        loop {
            let lf = input.windows(2).position(|bytes| bytes == b"\n\n");
            let crlf = input.windows(4).position(|bytes| bytes == b"\r\n\r\n");
            let end = match (lf, crlf) {
                (Some(lf), Some(crlf)) if crlf < lf => crlf + 4,
                (Some(lf), _) => lf + 2,
                (None, Some(crlf)) => crlf + 4,
                (None, None) => break,
            };
            frames.push(input[..end].to_vec());
            input = &input[end..];
        }
        (frames, input.len())
    }

    #[test]
    fn fragmented_binary_input_matches_the_whole_input_framing_oracle() {
        for length in 0..=6_u32 {
            for encoded in 0..4_usize.pow(length) {
                let mut encoded = encoded;
                let mut input: Vec<_> = (0..length)
                    .map(|_| {
                        let byte = [b'x', b'\r', b'\n', 0xff][encoded % 4];
                        encoded /= 4;
                        byte
                    })
                    .collect();
                input.extend_from_slice(b"\n\n");
                let expected = reference_frames(&input);
                for chunk_bytes in [1, 2, 3, input.len()] {
                    let mut buffer = FrameBuffer::default();
                    let mut actual = Vec::new();
                    for chunk in input.chunks(chunk_bytes) {
                        buffer.push(chunk);
                        while let Some(frame) = buffer.next_frame() {
                            actual.push(frame.to_vec());
                        }
                        assert!(buffer.next_frame().is_none());
                    }
                    assert_eq!(actual, expected.0, "input {input:?}, chunk {chunk_bytes}");
                    assert_eq!(buffer.len(), expected.1);
                }
            }
        }
    }
}

#[cfg(test)]
mod performance;
