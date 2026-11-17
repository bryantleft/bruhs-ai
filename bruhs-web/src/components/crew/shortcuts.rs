//! What the crew's two shortcuts do on the web: ⌘. stops every bruh and
//! ⌘⇧B moves focus into the rail.

use bruhs_models::crew::{send_stop_commands, service_is_current, stop_all_result, stop_commands};
use bruhs_models::reactive::{ReactiveCommand, ReactiveSnapshot};
use leptos::prelude::*;

/// Asks the rail to open and take focus; each press bumps the count.
#[derive(Clone, Copy)]
pub struct CrewFocusRequest(pub RwSignal<u32>);

/// The commands that stop every bruh. A service known to be current needs
/// no snapshot; otherwise `None` until the runs are `fetched`.
pub fn stop_plan(
    protocol: Option<u32>,
    fetched: Option<&ReactiveSnapshot>,
) -> Option<Vec<ReactiveCommand>> {
    if let Some(protocol) = protocol.filter(|protocol| service_is_current(*protocol)) {
        return Some(stop_commands(protocol, &[]));
    }
    fetched.map(|snapshot| stop_commands(snapshot.protocol, &snapshot.runs))
}

/// Stop every bruh. A current service gets `StopAll`; an older or unknown one
/// gets the snapshot first, then a cancel for each live run, going on past a
/// cancel that fails. `on_done` hears the outcome once every send finished.
pub fn stop_all(protocol: Option<u32>, on_done: impl Fn(Result<&'static str, String>) + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        let commands = match stop_plan(protocol, None) {
            Some(commands) => commands,
            None => match crate::api::reactive_snapshot().await {
                Ok(snapshot) => stop_plan(protocol, Some(&snapshot)).unwrap_or_default(),
                Err(error) => return on_done(stop_all_result(&[error.to_string()])),
            },
        };
        let failed = send_stop_commands(commands, |command| async move {
            crate::api::reactive_command(&command)
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        })
        .await;
        on_done(stop_all_result(&failed));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::CREW_PROTOCOL;

    fn run(id: &str, status: &str) -> bruhs_models::reactive::ReactiveRun {
        serde_json::from_value(serde_json::json!({
            "id": id, "agent_id": "a", "source_id": null, "subscription_id": null,
            "event_id": null, "status": status, "prompt": id, "thread_id": "t",
            "session_id": null, "created_at": 1, "updated_at": 1,
            "not_before": 1, "result": null, "error": null,
        }))
        .unwrap()
    }

    fn fetched(protocol: u32) -> ReactiveSnapshot {
        ReactiveSnapshot {
            protocol,
            runs: vec![
                run("q", "queued"),
                run("r", "running"),
                run("d", "completed"),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn a_current_service_stops_with_one_command_and_no_fetch() {
        assert_eq!(
            stop_plan(Some(CREW_PROTOCOL), None),
            Some(vec![ReactiveCommand::StopAll])
        );
    }

    #[test]
    fn an_old_or_unknown_service_reads_first_then_cancels_each_live_run() {
        let cancels = vec![
            ReactiveCommand::CancelRun { id: "q".into() },
            ReactiveCommand::CancelRun { id: "r".into() },
        ];
        for protocol in [Some(0), None] {
            assert_eq!(
                stop_plan(protocol, None),
                None,
                "{protocol:?} needs a fetch"
            );
            assert_eq!(
                stop_plan(protocol, Some(&fetched(0))),
                Some(cancels.clone())
            );
        }
        // An unknown protocol that the snapshot shows to be current.
        assert_eq!(
            stop_plan(None, Some(&fetched(CREW_PROTOCOL))),
            Some(vec![ReactiveCommand::StopAll])
        );
    }
}
