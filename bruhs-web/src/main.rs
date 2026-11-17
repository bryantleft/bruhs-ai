mod activity_monitor;
mod agent_updates;
mod api;
mod app;
mod auth;
mod auto_record;
mod calendar_sync_stream;
mod clock;
mod components;
mod durable_metadata;
mod format;
mod highlight;
mod live_events;
mod markdown;
mod meeting_actions;
mod meeting_suggestions;
mod meetings_stream;
mod navigation_guard;
mod queue;
mod relationship_store;
mod stash;
mod state;
mod storage;
mod stream;
mod task_saves;
mod task_style;
mod tasks_stream;
mod thread_catalog;
mod thread_history;
mod thread_link;
mod transcript_sync;
mod ui;
mod updates;
mod whiteboard;
mod workflow_executor;
mod workspace_bridge;

fn main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).ok();
    #[cfg(feature = "whiteboard-benchmark")]
    if web_sys::window().is_some_and(|window| {
        window
            .location()
            .pathname()
            .is_ok_and(|path| path == "/whiteboard-benchmark")
    }) {
        if let Err(error) = whiteboard::benchmark::mount() {
            log::error!("Whiteboard benchmark failed: {error:?}");
        }
        return;
    }
    leptos::mount::mount_to_body(app::App);
}
