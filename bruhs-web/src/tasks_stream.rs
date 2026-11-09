use crate::live_events::LiveEvents;
use crate::state::BackgroundTask;
use bruhs_models::api::live_events::TASKS;
use leptos::prelude::*;

/// Push each background task snapshot from the live stream into `set_tasks`.
/// The server sends one on every (re)connect and after every change.
pub fn subscribe_tasks_stream(live: &mut LiveEvents, set_tasks: WriteSignal<Vec<BackgroundTask>>) {
    live.on(TASKS, move |data| {
        match serde_json::from_str::<Vec<BackgroundTask>>(&data) {
            Ok(tasks) => set_tasks.set(tasks),
            Err(e) => log::warn!("tasks_stream: failed to parse snapshot: {:?}", e),
        }
    });
}
