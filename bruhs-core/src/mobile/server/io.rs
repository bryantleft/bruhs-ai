//! Bound idle connections, including clients that send a partial TLS/HTTP frame.
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    time::{Instant, Sleep},
};
const IDLE: Duration = Duration::from_secs(30);
pub(super) struct IdleIo<T> {
    inner: T,
    timer: Pin<Box<Sleep>>,
}
impl<T> IdleIo<T> {
    pub(super) fn new(inner: T) -> Self {
        Self {
            inner,
            timer: Box::pin(tokio::time::sleep(IDLE)),
        }
    }
    fn check(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        use std::future::Future;
        if self.timer.as_mut().poll(cx).is_ready() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "mobile connection idle",
            ))
        } else {
            Ok(())
        }
    }
    fn touched(&mut self) {
        self.timer.as_mut().reset(Instant::now() + IDLE);
    }
}
impl<T: AsyncRead + Unpin> AsyncRead for IdleIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        let before = buf.filled().len();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        if buf.filled().len() > before {
            this.touched();
        }
        result
    }
}
impl<T: AsyncWrite + Unpin> AsyncWrite for IdleIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        this.check(cx)?;
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        if matches!(result, Poll::Ready(Ok(n)) if n > 0) {
            this.touched();
        }
        result
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        this.check(cx)?;
        Pin::new(&mut this.inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}
