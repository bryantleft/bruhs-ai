//! Join failed recorder startups before releasing the capture reservation.
//! Readiness carries no error: the worker result is the single error authority.

use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;

#[derive(Debug)]
pub enum StartupError<E> {
    Worker(E),
    Panicked,
    ExitedBeforeReady,
}

pub fn await_ready<E>(
    ready: Receiver<()>,
    worker: JoinHandle<Result<(), E>>,
) -> Result<JoinHandle<Result<(), E>>, StartupError<E>> {
    if ready.recv().is_ok() {
        return Ok(worker);
    }
    match worker.join() {
        Ok(Err(error)) => Err(StartupError::Worker(error)),
        Ok(Ok(())) => Err(StartupError::ExitedBeforeReady),
        Err(_) => Err(StartupError::Panicked),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Failure {
        Permission,
        DeviceUnavailable,
        DiskFull,
    }

    #[test]
    fn original_startup_failure_is_preserved() {
        for error in [
            Failure::Permission,
            Failure::DeviceUnavailable,
            Failure::DiskFull,
        ] {
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let worker = std::thread::spawn(move || {
                let _ready = ready_tx;
                Err(error)
            });
            assert!(
                matches!(await_ready(ready_rx, worker), Err(StartupError::Worker(actual)) if actual == error)
            );
        }
    }

    #[test]
    fn readiness_returns_ownership_without_waiting_for_recording_to_stop() {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            ready_tx.send(()).unwrap();
            stop_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok::<_, Failure>(())
        });
        let worker = await_ready(ready_rx, worker).unwrap();
        stop_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Ok(()));
    }

    #[test]
    fn startup_panic_is_distinct_from_device_errors() {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || -> Result<(), Failure> {
            let _ready = ready_tx;
            panic!("synthetic startup panic");
        });
        assert!(matches!(
            await_ready(ready_rx, worker),
            Err(StartupError::Panicked)
        ));
    }

    #[test]
    fn success_without_readiness_is_not_a_started_recorder() {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let _ready = ready_tx;
            Ok::<_, Failure>(())
        });
        assert!(matches!(
            await_ready(ready_rx, worker),
            Err(StartupError::ExitedBeforeReady)
        ));
    }

    #[test]
    fn disconnected_readiness_waits_for_resource_cleanup() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let cleaned = Arc::new(AtomicBool::new(false));
        let resource = Resource(Arc::clone(&cleaned));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _resource = resource;
            drop(ready_tx);
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            Err(Failure::DeviceUnavailable)
        });
        let waiter =
            std::thread::spawn(move || result_tx.send(await_ready(ready_rx, worker)).unwrap());
        assert!(matches!(
            result_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release_tx.send(()).unwrap();
        let result = result_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(cleaned.load(Ordering::SeqCst));
        assert!(matches!(
            result,
            Err(StartupError::Worker(Failure::DeviceUnavailable))
        ));
        waiter.join().unwrap();
    }
}
