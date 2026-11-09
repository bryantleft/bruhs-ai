mod activity_monitor;
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
mod markdown;
mod meeting_actions;
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
mod transcript_sync;
mod ui;
mod updates;
mod workflow_executor;

fn main() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).ok();
    leptos::mount::mount_to_body(app::App);
}
