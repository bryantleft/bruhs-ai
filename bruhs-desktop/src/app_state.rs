use bruhs_core::AppState;
use gpui::{App, AppContext as _, Global, Task};
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::{Builder, Runtime};

/// Desktop-specific wrapper around the shared bruhs-core AppState.
pub struct DesktopState {
    pub app: Arc<AppState>,
    pub runtime: Arc<Runtime>,
}

impl Global for DesktopState {}

/// Thread ids and titles the composer’s `@` menu reads. The workspace
/// republishes this while it draws, so a chat can list siblings without
/// holding the workspace.
#[derive(Clone, Debug, Default)]
pub struct ThreadCatalog {
    pub threads: Vec<(String, String)>,
}

impl Global for ThreadCatalog {}

impl DesktopState {
    pub fn new() -> std::io::Result<Self> {
        let runtime = Builder::new_multi_thread()
            .enable_all()
            .thread_name("bruhs-desktop-agent")
            .build()?;

        // `AppState::new` spawns nothing, so building it outside the
        // runtime is fine. `start_background` is deliberately not called:
        // the desktop reads Google Calendar through bruhs-server's SSE
        // stream, and a second sync loop here would race the server's on
        // the same keychain entry and cursor files with no subscriber.
        let builder = AppState::builder();
        #[cfg(test)]
        let builder = builder.secret_store(bruhs_core::secrets::SecretStore::in_memory());
        Ok(Self {
            app: Arc::new(builder.build()),
            runtime: Arc::new(runtime),
        })
    }
}

/// Run `future` to completion off the UI thread, inside the desktop's tokio
/// runtime (bruhs-core's git and file helpers spawn processes and use tokio
/// timers, so they need its reactor).
///
/// The future runs on a gpui background thread that enters the runtime, so
/// a gpui test drives it deterministically like any background task.
pub fn spawn_tokio<T, F>(cx: &App, future: F) -> Task<T>
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
{
    let runtime = Arc::clone(&cx.global::<DesktopState>().runtime);
    cx.background_spawn(async move { runtime.block_on(future) })
}
