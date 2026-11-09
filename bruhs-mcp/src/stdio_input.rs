//! Bounded newline framing; rejected input never becomes a partial command.
use std::io;
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// Includes the newline, when present. The private bridge has its own limit.
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

pub enum Frame<'a> {
    Message(&'a [u8]),
    Oversized,
}

pub struct StdioInput<R> {
    reader: R,
    buffer: Vec<u8>,
    discarding: bool,
}

impl<R: AsyncBufRead + Unpin> StdioInput<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::with_capacity(MAX_REQUEST_BYTES),
            discarding: false,
        }
    }

    pub async fn next(&mut self) -> io::Result<Option<Frame<'_>>> {
        self.buffer.clear();
        loop {
            let chunk = self.reader.fill_buf().await?;
            if chunk.is_empty() {
                return Ok((!self.buffer.is_empty()).then_some(Frame::Message(&self.buffer)));
            }
            let newline = chunk.iter().position(|byte| *byte == b'\n');
            let consumed = newline.map_or(chunk.len(), |index| index + 1);
            if self.discarding {
                self.reader.consume(consumed);
                self.discarding = newline.is_none();
                continue;
            }
            if consumed > MAX_REQUEST_BYTES - self.buffer.len() {
                self.reader.consume(consumed);
                self.discarding = newline.is_none();
                self.buffer.clear();
                // Respond immediately even if a client has not sent a newline.
                // The next call drains this frame before accepting another one.
                return Ok(Some(Frame::Oversized));
            }
            self.buffer.extend_from_slice(&chunk[..consumed]);
            self.reader.consume(consumed);
            if newline.is_some() {
                return Ok(Some(Frame::Message(&self.buffer)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn exact_limit_and_final_unterminated_frame_are_accepted() {
        let mut bytes = vec![b' '; MAX_REQUEST_BYTES - 1];
        bytes.extend_from_slice(b"\nlast");
        let mut input = StdioInput::new(BufReader::new(bytes.as_slice()));
        assert!(
            matches!(input.next().await.unwrap(), Some(Frame::Message(line)) if line.len() == MAX_REQUEST_BYTES)
        );
        assert!(matches!(
            input.next().await.unwrap(),
            Some(Frame::Message(b"last"))
        ));
        assert!(input.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn oversized_frame_is_rejected_once_without_retaining_its_tail() {
        let mut bytes = vec![b'x'; MAX_REQUEST_BYTES * 20];
        bytes.extend_from_slice(b"\nnext\n");
        let mut input = StdioInput::new(BufReader::with_capacity(17, bytes.as_slice()));
        assert!(matches!(
            input.next().await.unwrap(),
            Some(Frame::Oversized)
        ));
        assert!(matches!(
            input.next().await.unwrap(),
            Some(Frame::Message(b"next\n"))
        ));
        assert!(input.next().await.unwrap().is_none());
        assert_eq!(input.buffer.capacity(), MAX_REQUEST_BYTES);
    }

    #[tokio::test]
    async fn oversize_is_reported_before_newline_or_eof() {
        let (mut writer, reader) = tokio::io::duplex(MAX_REQUEST_BYTES + 1);
        writer
            .write_all(&vec![b'x'; MAX_REQUEST_BYTES + 1])
            .await
            .unwrap();
        let mut input = StdioInput::new(BufReader::new(reader));
        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), input.next())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(frame, Some(Frame::Oversized)));
        writer.write_all(b"discarded\nnext\n").await.unwrap();
        assert!(matches!(
            input.next().await.unwrap(),
            Some(Frame::Message(b"next\n"))
        ));
    }

    #[tokio::test]
    async fn oversized_final_frame_is_not_parsed_again_at_eof() {
        let bytes = vec![b'x'; MAX_REQUEST_BYTES + 1];
        let mut input = StdioInput::new(BufReader::new(bytes.as_slice()));
        assert!(matches!(
            input.next().await.unwrap(),
            Some(Frame::Oversized)
        ));
        assert!(input.next().await.unwrap().is_none());
    }
}
