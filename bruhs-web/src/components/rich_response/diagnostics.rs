use std::sync::{Arc, Mutex, PoisonError};

use leptos::prelude::*;

use super::client;

#[derive(Default)]
struct Pending {
    next: Option<Vec<String>>,
    running: bool,
}

#[derive(Clone)]
pub(super) struct DiagnosticsWriter {
    id: String,
    pending: Arc<Mutex<Pending>>,
    status: RwSignal<Option<String>>,
}

impl DiagnosticsWriter {
    pub fn new(id: String, status: RwSignal<Option<String>>) -> Self {
        Self {
            id,
            pending: Arc::new(Mutex::new(Pending::default())),
            status,
        }
    }

    pub fn queue(&self, errors: Vec<String>) {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .next = Some(errors);
        self.drain();
    }

    pub fn drain(&self) {
        {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            if pending.running || pending.next.is_none() {
                return;
            }
            pending.running = true;
        }
        let writer = self.clone();
        leptos::task::spawn_local(async move {
            loop {
                let next = {
                    let mut pending = writer
                        .pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    match pending.next.take() {
                        Some(errors) => errors,
                        None => {
                            pending.running = false;
                            drop(pending);
                            break;
                        }
                    }
                };
                match client::diagnostics(&writer.id, &next).await {
                    Ok(()) => {
                        let _ = writer.status.try_set(None);
                    }
                    Err(error) => {
                        let mut pending = writer
                            .pending
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner);
                        if pending.next.is_none() {
                            pending.next = Some(next);
                        }
                        pending.running = false;
                        drop(pending);
                        let _ = writer
                            .status
                            .try_set(Some(format!("Could not report artifact preview: {error}")));
                        break;
                    }
                }
            }
        });
    }
}
