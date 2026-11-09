//! Serialize capture transitions without blocking the async executor. Once a
//! hardware operation starts, its reservation survives an HTTP disconnect;
//! a following stop waits for it rather than racing an unfinished start.

use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard};

#[derive(Clone, Default)]
pub struct CaptureLifecycle(Arc<Mutex<()>>);

impl CaptureLifecycle {
    pub async fn enter(&self) -> OwnedMutexGuard<()> {
        self.0.clone().lock_owned().await
    }

    pub fn try_enter(&self) -> Option<OwnedMutexGuard<()>> {
        self.0.clone().try_lock_owned().ok()
    }

    pub async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, tokio::task::JoinError> {
        let reservation = self.enter().await;
        tokio::task::spawn_blocking(move || {
            let _reservation = reservation;
            operation()
        })
        .await
    }
}

#[cfg(test)]
mod tests;
