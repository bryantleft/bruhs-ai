//! Reject oversized backend frames before tokio-postgres allocates their payload.
//! Postgres backend messages are one tag byte followed by a length including its four bytes.
use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Instant,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::TcpStream,
};

pub(crate) struct BudgetStream {
    socket: TcpStream,
    header: [u8; 5],
    received: usize,
    emitted: usize,
    remaining: usize,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}
impl BudgetStream {
    pub(crate) fn new(socket: TcpStream, cancelled: Arc<AtomicBool>) -> Self {
        Self {
            socket,
            header: [0; 5],
            received: 0,
            emitted: 0,
            remaining: 0,
            cancelled,
            deadline: Instant::now() + super::QUERY_TIMEOUT,
        }
    }
}
impl AsyncRead for BudgetStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.cancelled.load(Ordering::Acquire) || Instant::now() >= this.deadline {
            return Poll::Ready(Err(io::Error::other("Database query cancelled.")));
        }
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        while this.received < 5 {
            let mut buffer = ReadBuf::new(&mut this.header[this.received..]);
            match Pin::new(&mut this.socket).poll_read(cx, &mut buffer) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Ready(Ok(())) => {
                    let count = buffer.filled().len();
                    if count == 0 {
                        return Poll::Ready(if this.received == 0 {
                            Ok(())
                        } else {
                            Err(io::Error::from(io::ErrorKind::UnexpectedEof))
                        });
                    }
                    this.received += count;
                }
            }
            if this.received == 5 {
                let length = u32::from_be_bytes([
                    this.header[1],
                    this.header[2],
                    this.header[3],
                    this.header[4],
                ]);
                let length = usize::try_from(length).map_err(io::Error::other)?;
                if !(4..=bruhs_models::database::RESULT_BYTES).contains(&length) {
                    return Poll::Ready(Err(io::Error::other("Postgres returned a frame exceeding the 32 KiB result budget. Select smaller values.")));
                }
                this.remaining = length - 4;
            }
        }
        if this.emitted < 5 {
            let count = (5 - this.emitted).min(output.remaining());
            output.put_slice(&this.header[this.emitted..this.emitted + count]);
            this.emitted += count;
            if this.remaining == 0 && this.emitted == 5 {
                this.received = 0;
                this.emitted = 0;
            }
            return Poll::Ready(Ok(()));
        }
        let limit = this.remaining.min(output.remaining());
        let mut body = ReadBuf::new(output.initialize_unfilled_to(limit));
        match Pin::new(&mut this.socket).poll_read(cx, &mut body) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {
                let count = body.filled().len();
                if count == 0 {
                    return Poll::Ready(Err(io::ErrorKind::UnexpectedEof.into()));
                }
                output.advance(count);
                this.remaining -= count;
                if this.remaining == 0 {
                    this.received = 0;
                    this.emitted = 0;
                }
                Poll::Ready(Ok(()))
            }
        }
    }
}
impl AsyncWrite for BudgetStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().socket).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().socket).poll_shutdown(cx)
    }
}
