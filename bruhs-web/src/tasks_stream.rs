use crate::state::BackgroundTask;
use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{EventSource, MessageEvent};

use crate::api::BASE_URL;

/// Subscribe to `/api/tasks/stream` via EventSource and push each received
/// snapshot into `set_tasks`. The underlying `EventSource` is leaked (never
/// closed) so the subscription lives for the lifetime of the page — matching
/// the TS `use-thread-tasks` hook behavior.
pub fn subscribe_tasks_stream(set_tasks: WriteSignal<Vec<BackgroundTask>>) {
    let url = format!("{}/api/tasks/stream", BASE_URL);
    let es = match EventSource::new(&url) {
        Ok(es) => es,
        Err(e) => {
            log::warn!("tasks_stream: failed to open EventSource: {:?}", e);
            return;
        }
    };

    let on_message = Closure::<dyn Fn(MessageEvent)>::new(move |ev: MessageEvent| {
        if let Some(data) = ev.data().as_string() {
            match serde_json::from_str::<Vec<BackgroundTask>>(&data) {
                Ok(tasks) => set_tasks.set(tasks),
                Err(e) => log::warn!("tasks_stream: failed to parse snapshot: {:?}", e),
            }
        }
    });
    es.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    on_message.forget();

    let on_error = Closure::<dyn Fn(web_sys::Event)>::new(move |_| {
        log::debug!("tasks_stream: EventSource reconnecting");
    });
    es.set_onerror(Some(on_error.as_ref().unchecked_ref()));
    on_error.forget();
}
