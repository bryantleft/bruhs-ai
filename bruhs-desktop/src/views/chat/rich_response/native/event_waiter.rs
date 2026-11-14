//! Wait for GLib's actual descriptors/deadline without dispatching GTK off-thread.

use std::io::{self, Write as _};
use std::os::{fd::AsRawFd as _, unix::net::UnixStream};
use std::sync::mpsc;
use std::thread::JoinHandle;

pub(super) struct PollSpec {
    pub(super) descriptors: Vec<libc::pollfd>,
    pub(super) timeout_ms: i32,
}

struct WaitRequest {
    spec: PollSpec,
    ready: async_channel::Sender<Result<(), String>>,
}

pub(super) struct EventWaiter {
    requests: Option<mpsc::Sender<WaitRequest>>,
    cancel: UnixStream,
    worker: Option<JoinHandle<()>>,
}

impl EventWaiter {
    pub(super) fn new() -> Result<Self, String> {
        let (cancel, canceled) = UnixStream::pair().map_err(|error| error.to_string())?;
        let (requests, receive) = mpsc::channel::<WaitRequest>();
        let worker = std::thread::Builder::new()
            .name("bruhs-gtk-events".into())
            .spawn(move || {
                for request in receive {
                    let mut descriptors = request.spec.descriptors;
                    let cancel_index = descriptors.len();
                    descriptors.push(libc::pollfd {
                        fd: canceled.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    });
                    let result = poll(&mut descriptors, request.spec.timeout_ms);
                    if descriptors[cancel_index].revents != 0 {
                        break;
                    }
                    let _ = request.ready.try_send(result);
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            requests: Some(requests),
            cancel,
            worker: Some(worker),
        })
    }

    pub(super) fn wait(
        &self,
        spec: PollSpec,
    ) -> Result<async_channel::Receiver<Result<(), String>>, String> {
        let (ready, receive) = async_channel::bounded(1);
        self.requests
            .as_ref()
            .ok_or("The GTK event dispatcher has stopped")?
            .send(WaitRequest { spec, ready })
            .map_err(|error| error.to_string())?;
        Ok(receive)
    }
}

impl Drop for EventWaiter {
    fn drop(&mut self) {
        self.requests.take();
        // The worker only blocks in recv or poll. Closing the request sender
        // and signaling this owned socket wake both cases, including timeout=-1.
        let _ = self.cancel.write_all(&[1]);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn poll(descriptors: &mut [libc::pollfd], timeout_ms: i32) -> Result<(), String> {
    let count = descriptors
        .len()
        .try_into()
        .map_err(|error: std::num::TryFromIntError| error.to_string())?;
    loop {
        // SAFETY: the initialized pollfd slice remains owned by this worker
        // until poll returns; count is its checked length. No GTK API is called.
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), count, timeout_ms) };
        if result >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_waits_for_source_readiness_without_a_periodic_timer() {
        let (mut write, read) = UnixStream::pair().unwrap();
        let waiter = EventWaiter::new().unwrap();
        let ready = waiter
            .wait(PollSpec {
                descriptors: vec![libc::pollfd {
                    fd: read.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                }],
                timeout_ms: -1,
            })
            .unwrap();
        assert!(matches!(
            ready.try_recv(),
            Err(async_channel::TryRecvError::Empty)
        ));
        write.write_all(&[1]).unwrap();
        ready.recv_blocking().unwrap().unwrap();
    }

    #[test]
    fn dropping_the_last_host_cancels_an_indefinite_source_wait() {
        let waiter = EventWaiter::new().unwrap();
        let ready = waiter
            .wait(PollSpec {
                descriptors: Vec::new(),
                timeout_ms: -1,
            })
            .unwrap();
        drop(waiter);
        assert!(ready.recv_blocking().is_err());
    }

    #[test]
    fn source_deadlines_wake_without_input() {
        let waiter = EventWaiter::new().unwrap();
        let ready = waiter
            .wait(PollSpec {
                descriptors: Vec::new(),
                timeout_ms: 0,
            })
            .unwrap();
        ready.recv_blocking().unwrap().unwrap();
    }
}
