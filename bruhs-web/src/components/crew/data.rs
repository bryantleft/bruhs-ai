//! The crew's server data as one bruh page sees it: the reactive snapshot,
//! reloaded whenever the service reports a change to the crew, and a way to
//! send commands that keeps them in order.

use std::collections::VecDeque;

use bruhs_models::crew::room::rooms_reason;
use bruhs_models::crew::{crew_commands_ready, rooms_ready, service_notice};
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
    /// The last Computer state read said this system runs no bruh browser.
    pub computer_unsupported: RwSignal<bool>,
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
        self.protocol().is_some_and(crew_commands_ready)
    }

    /// Whether the service has rooms. Unknown counts as not yet.
    pub fn rooms_ready(&self) -> bool {
        self.protocol().is_some_and(rooms_ready)
    }

    /// Why crew commands are off, once the version is known to be older.
    /// Empty while they work, so a working control carries no excuse.
    pub fn service_reason(&self) -> Option<&'static str> {
        if self.crew_ready() {
            return None;
        }
        self.protocol()
            .and_then(|protocol| service_notice(protocol, None))
            .map(|notice| notice.message())
    }

    /// Why rooms are off, once the version is known to be older.
    pub fn rooms_reason(&self) -> Option<&'static str> {
        rooms_reason(self.protocol())
    }
}

/// Context data a caller may share. When a view is rebuilt in place, such
/// as one bruh's page replacing another's, its owner is cleaned up and run
/// again: the old page's data stays in context, but its signals are gone.
fn reusable(data: Option<CrewData>) -> Option<CrewData> {
    data.filter(|data| !data.snapshot.is_disposed())
}

/// The page's crew data: made by the first caller under a page and shared
/// with every tab below it.
pub fn use_crew_snapshot() -> CrewData {
    if let Some(data) = reusable(use_context::<CrewData>()) {
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

    // Load now, and again whenever the service reports a change to the crew,
    // its rooms or a run.
    let updates = use_context::<AgentUpdates>();
    if let Some(updates) = updates {
        Effect::new(move |_| {
            updates.bruhs.track();
            updates.rooms.track();
            updates.runs_at.track();
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
        computer_unsupported: RwSignal::new(false),
        live_protocol: updates.map(|updates| updates.protocol),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> CrewData {
        CrewData {
            snapshot: RwSignal::new(None),
            send: Callback::new(|_| {}),
            error: RwSignal::new(None),
            busy: RwSignal::new(false),
            reload: Callback::new(|()| {}),
            computer_unsupported: RwSignal::new(false),
            live_protocol: None,
        }
    }

    fn data_at(protocol: u32) -> CrewData {
        let data = data();
        data.snapshot.set(Some(ReactiveSnapshot {
            protocol,
            ..ReactiveSnapshot::default()
        }));
        data
    }

    #[test]
    fn a_protocol_one_snapshot_offers_crew_commands_but_not_rooms() {
        let owner = Owner::new();
        owner.with(|| {
            let old = data_at(1);
            assert!(old.crew_ready() && !old.rooms_ready());
            assert_eq!(old.service_reason(), None);
            assert_eq!(
                old.rooms_reason(),
                Some(bruhs_models::crew::ServiceNotice::ServingOld.message())
            );
            let current = data_at(bruhs_models::crew::CREW_PROTOCOL);
            assert!(current.crew_ready() && current.rooms_ready());
            assert_eq!(
                (current.service_reason(), current.rooms_reason()),
                (None, None)
            );
        });
    }

    #[test]
    fn a_protocol_two_snapshot_offers_rooms_but_not_computers() {
        let owner = Owner::new();
        owner.with(|| {
            // The Computer tab reads the service through the shared gate.
            let gate =
                |data: CrewData| bruhs_models::crew::computer_gate(None, "a", data.protocol());
            let old = data_at(2);
            assert!(old.rooms_ready() && !gate(old).ready);
            assert_eq!(
                gate(old).reason,
                Some(bruhs_models::crew::ServiceNotice::ServingOld.message())
            );
            let current = data_at(bruhs_models::crew::COMPUTER_PROTOCOL);
            assert!(gate(current).ready);
            assert_eq!(gate(current).reason, None);
            // Unknown counts as not yet, with no reason to show.
            let unknown = data();
            assert!(!gate(unknown).ready);
            assert_eq!(gate(unknown).reason, None);
        });
    }

    #[test]
    fn data_from_a_page_rebuilt_in_place_is_made_again() {
        let owner = Owner::new();
        let shared = owner.with(data);
        assert!(reusable(Some(shared)).is_some());
        assert!(reusable(None).is_none());
        // Opening another bruh cleans up the owner and runs the page again.
        owner.cleanup();
        assert!(reusable(Some(shared)).is_none());
    }
}
