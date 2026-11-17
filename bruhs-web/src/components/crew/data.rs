//! The crew's server data as one bruh page sees it: the reactive snapshot,
//! reloaded whenever the service reports a change to the crew, and a way to
//! send commands that keeps them in order.

use std::collections::VecDeque;

use bruhs_models::crew::{service_is_current, service_notice};
use bruhs_models::reactive::{ReactiveCommand, ReactiveSnapshot};
use leptos::prelude::*;

use crate::agent_updates::AgentUpdates;

/// The snapshot a bruh page and its tabs share.
#[derive(Clone, Copy)]
pub struct CrewData {
    pub snapshot: RwSignal<Option<ReactiveSnapshot>>,
    /// Sends a command, after any sent before it, and replaces the snapshot
    /// with the service's answer. A failure drops the commands queued after
    /// it, since they assumed it would apply.
    pub send: Callback<ReactiveCommand>,
    pub error: RwSignal<Option<String>>,
    /// A command is on its way.
    pub busy: RwSignal<bool>,
    /// Loads the snapshot again, such as after a failed load.
    pub reload: Callback<()>,
    /// The protocol the live stream reports, when the app has the stream.
    live_protocol: Option<RwSignal<Option<u32>>>,
}

impl CrewData {
    /// The service's crew protocol, once a frame or a snapshot has said it.
    pub fn protocol(&self) -> Option<u32> {
        self.live_protocol
            .and_then(|protocol| protocol.get())
            .or_else(|| self.snapshot.with(|s| s.as_ref().map(|s| s.protocol)))
    }

    /// Whether the service understands the crew commands. Unknown counts as
    /// not yet: nothing that needs them is offered until the version is known.
    pub fn crew_ready(&self) -> bool {
        self.protocol().is_some_and(service_is_current)
    }

    /// Why crew commands are off, once the version is known to be older.
    pub fn service_reason(&self) -> Option<&'static str> {
        self.protocol()
            .and_then(|protocol| service_notice(protocol, None))
            .map(|notice| notice.message())
    }
}

/// The page's crew data: made by the first caller under a page and shared
/// with every tab below it.
pub fn use_crew_snapshot() -> CrewData {
    if let Some(data) = use_context::<CrewData>() {
        return data;
    }
    let data = new_crew_data();
    provide_context(data);
    data
}

fn new_crew_data() -> CrewData {
    let snapshot = RwSignal::new(None::<ReactiveSnapshot>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let alive = RwSignal::new(true);
    let revision = RwSignal::new(0u64);
    let queue = StoredValue::new(VecDeque::<ReactiveCommand>::new());

    let load_failed = StoredValue::new(false);
    let reload = Callback::new(move |()| {
        if busy.get_untracked() {
            return;
        }
        revision.update(|value| *value = value.wrapping_add(1));
        let requested = revision.get_untracked();
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::reactive_snapshot().await;
            if alive.try_get_untracked() != Some(true)
                || revision.try_get_untracked() != Some(requested)
            {
                return;
            }
            match result {
                Ok(next) => {
                    // A load that works again clears the load's own error,
                    // never a command's.
                    if load_failed.get_value() {
                        load_failed.set_value(false);
                        error.set(None);
                    }
                    if snapshot.with_untracked(|current| current.as_ref() != Some(&next)) {
                        snapshot.set(Some(next));
                    }
                }
                Err(err) => {
                    load_failed.set_value(true);
                    error.set(Some(err.to_string()));
                }
            }
        });
    });

    // Sends the queue's head, then the next, until the queue is empty.
    let drain = StoredValue::new(None::<Callback<()>>);
    let next = Callback::new(move |()| {
        let Some(command) = queue.try_update_value(VecDeque::pop_front).flatten() else {
            busy.set(false);
            return;
        };
        busy.set(true);
        revision.update(|value| *value = value.wrapping_add(1));
        wasm_bindgen_futures::spawn_local(async move {
            let result = crate::api::reactive_command(&command).await;
            if alive.try_get_untracked() != Some(true) {
                return;
            }
            match result {
                Ok(next) => {
                    snapshot.set(Some(next));
                    if let Some(drain) = drain.get_value() {
                        drain.run(());
                    }
                }
                Err(err) => {
                    queue.update_value(VecDeque::clear);
                    busy.set(false);
                    error.set(Some(err.to_string()));
                }
            }
        });
    });
    drain.set_value(Some(next));

    let send = Callback::new(move |command: ReactiveCommand| {
        error.set(None);
        queue.update_value(|queue| queue.push_back(command));
        if !busy.get_untracked() {
            next.run(());
        }
    });

    // Load now, and again whenever the service reports a change to the crew.
    let updates = use_context::<AgentUpdates>();
    if let Some(updates) = updates {
        Effect::new(move |_| {
            updates.bruhs.track();
            reload.run(());
        });
    } else {
        Effect::new(move |_| reload.run(()));
    }
    on_cleanup(move || alive.set(false));

    CrewData {
        snapshot,
        send,
        error,
        busy,
        reload,
        live_protocol: updates.map(|updates| updates.protocol),
    }
}
