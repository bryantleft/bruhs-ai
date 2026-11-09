use async_channel::{unbounded, Receiver};
use bruhs_core::{BackgroundTaskRegistry, TaskEvent};
use std::sync::Arc;

pub struct TaskRegistrySubscription {
    registry: Arc<BackgroundTaskRegistry>,
    listener_index: usize,
}

impl TaskRegistrySubscription {
    pub fn new(registry: Arc<BackgroundTaskRegistry>) -> (Self, Receiver<TaskEvent>) {
        let (sender, receiver) = unbounded();
        let listener_index = registry.subscribe(move |event| {
            let _ = sender.try_send(event.clone());
        });
        (
            Self {
                registry,
                listener_index,
            },
            receiver,
        )
    }
}

impl Drop for TaskRegistrySubscription {
    fn drop(&mut self) {
        self.registry.unsubscribe(self.listener_index);
    }
}
