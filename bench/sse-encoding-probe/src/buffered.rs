use std::fmt::Write as _;
use std::io::{self, BufWriter, Write as _};

use axum::response::sse::{Event, EventDataWriter};
use serde::Serialize;

/// A byte writer can be flushed midway through a UTF-8 scalar. Carry at most
/// three pending bytes until the scalar is complete; never replace invalid input.
struct Utf8EventWriter<const SIMD: bool> {
    event: EventDataWriter,
    pending: [u8; 4],
    pending_len: usize,
}

impl<const SIMD: bool> Utf8EventWriter<SIMD> {
    fn new() -> Self {
        Self {
            event: Event::default().into_data_writer(),
            pending: [0; 4],
            pending_len: 0,
        }
    }

    fn into_event(self) -> io::Result<Event> {
        if self.pending_len != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete UTF-8 at end of SSE JSON",
            ));
        }
        Ok(self.event.into_event())
    }
}

impl<const SIMD: bool> io::Write for Utf8EventWriter<SIMD> {
    fn write(&mut self, mut bytes: &[u8]) -> io::Result<usize> {
        let consumed = bytes.len();
        while self.pending_len != 0 && !bytes.is_empty() {
            assert!(
                self.pending_len < 4,
                "a UTF-8 scalar completes within four bytes"
            );
            self.pending[self.pending_len] = bytes[0];
            self.pending_len += 1;
            bytes = &bytes[1..];
            match std::str::from_utf8(&self.pending[..self.pending_len]) {
                Ok(text) => {
                    self.event.write_str(text).map_err(io::Error::other)?;
                    self.pending_len = 0;
                }
                Err(error) if error.error_len().is_some() => {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, error));
                }
                Err(_) => {}
            }
        }
        if self.pending_len != 0 {
            return Ok(consumed);
        }
        match checked_utf8::<SIMD>(bytes) {
            Ok(text) => self.event.write_str(text).map_err(io::Error::other)?,
            Err(error) => {
                if error.error_len().is_some() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, error));
                }
                let valid = error.valid_up_to();
                let text =
                    std::str::from_utf8(&bytes[..valid]).expect("UTF-8 decoder's validated prefix");
                self.event.write_str(text).map_err(io::Error::other)?;
                let tail = &bytes[valid..];
                assert!(
                    tail.len() < 4,
                    "an incomplete UTF-8 scalar has at most three bytes"
                );
                self.pending[..tail.len()].copy_from_slice(tail);
                self.pending_len = tail.len();
            }
        }
        Ok(consumed)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.pending_len == 0 {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "incomplete UTF-8 at end of SSE JSON",
            ))
        }
    }
}

fn checked_utf8<const SIMD: bool>(bytes: &[u8]) -> Result<&str, std::str::Utf8Error> {
    if SIMD && let Ok(text) = simdutf8::basic::from_utf8(bytes) {
        return Ok(text);
    }
    // The standard decoder supplies exact valid-prefix/error offsets for the
    // uncommon invalid or incomplete chunk; both methods keep identical errors.
    std::str::from_utf8(bytes)
}

pub(super) fn event<T: Serialize + ?Sized, const SIMD: bool>(
    data: &T,
    capacity: usize,
) -> Result<Event, String> {
    let mut writer = BufWriter::with_capacity(capacity, Utf8EventWriter::<SIMD>::new());
    serde_json::to_writer(&mut writer, data).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())?;
    writer
        .into_inner()
        .map_err(|error| error.to_string())?
        .into_event()
        .map_err(|error| error.to_string())
}

async fn contracts_for<const SIMD: bool>() {
    for text in ["", "ASCII", "aΩ😀中z", "line\r\nΩ\0😀"] {
        let expected = super::frame_bytes(Event::default().data(text)).await;
        for width in 1..=text.len().max(1) {
            let mut writer = Utf8EventWriter::<SIMD>::new();
            for chunk in text.as_bytes().chunks(width) {
                writer
                    .write_all(chunk)
                    .expect("arbitrary UTF-8 chunk boundaries");
            }
            writer.flush().expect("all UTF-8 scalars complete");
            assert_eq!(
                super::frame_bytes(writer.into_event().expect("event")).await,
                expected
            );
        }
    }
    for invalid in [&b"\xff"[..], &b"\xe2\x28\xa1"[..], &b"\xf0\x9f"[..]] {
        let mut writer = Utf8EventWriter::<SIMD>::new();
        let wrote = writer.write_all(invalid);
        assert!(
            wrote.is_err() || writer.flush().is_err(),
            "invalid/incomplete UTF-8 must fail"
        );
    }
}

pub(super) async fn contracts() {
    contracts_for::<false>().await;
    contracts_for::<true>().await;
}
