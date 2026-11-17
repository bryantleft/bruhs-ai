//! What the crew's two shortcuts do on the web: ⌘. stops every bruh and
//! ⌘⇧B moves focus into the rail.

use bruhs_models::crew::{service_is_current, stop_commands};
use bruhs_models::reactive::ReactiveCommand;
use leptos::prelude::*;

/// Asks the rail to open and take focus; each press bumps the count.
#[derive(Clone, Copy)]
pub struct CrewFocusRequest(pub RwSignal<u32>);

/// The commands that stop every bruh, when the service's version is known to
/// understand one command for it. `None` means read the runs first.
pub fn immediate_stop(protocol: Option<u32>) -> Option<Vec<ReactiveCommand>> {
    protocol
        .filter(|protocol| service_is_current(*protocol))
        .map(|protocol| stop_commands(protocol, &[]))
}

/// Stop every bruh. A current service gets `StopAll`; an older or unknown one
/// gets the snapshot first, then a cancel for each live run.
pub fn stop_all(protocol: Option<u32>, on_error: impl Fn(String) + 'static) {
    wasm_bindgen_futures::spawn_local(async move {
        let commands = match immediate_stop(protocol) {
            Some(commands) => commands,
            None => match crate::api::reactive_snapshot().await {
                Ok(snapshot) => stop_commands(snapshot.protocol, &snapshot.runs),
                Err(error) => return on_error(error.to_string()),
            },
        };
        for command in commands {
            if let Err(error) = crate::api::reactive_command(&command).await {
                return on_error(error.to_string());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bruhs_models::crew::CREW_PROTOCOL;

    #[test]
    fn a_current_service_stops_with_one_command_and_an_old_one_reads_first() {
        assert_eq!(
            immediate_stop(Some(CREW_PROTOCOL)),
            Some(vec![ReactiveCommand::StopAll])
        );
        assert_eq!(immediate_stop(Some(0)), None);
        assert_eq!(immediate_stop(None), None);
    }
}
