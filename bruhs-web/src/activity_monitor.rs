//! Permission is session-only, never inferred from persisted preferences.
use bruhs_models::{ActivityMonitorSession, AutoRecordSettings, ACTIVITY_MONITOR_RENEW_MS};

use crate::api::ApiError;
use leptos::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct MonitorFeedback(pub RwSignal<String>);

pub fn start(
    settings: ReadSignal<AutoRecordSettings>,
    set_settings: WriteSignal<AutoRecordSettings>,
) {
    let MonitorFeedback(feedback) = expect_context::<MonitorFeedback>();
    let desired = Memo::new(move |_| settings.with(|s| s.enabled && s.audio_driven_enabled));
    let generation = Arc::new(AtomicU64::new(0));
    Effect::new(move |_| {
        if !desired.get() {
            return;
        }
        let session = Arc::new(ActivityMonitorSession::default());
        generation.fetch_add(1, Ordering::Relaxed);
        let cleanup = session.clone();
        let cleanup_generation = generation.clone();
        on_cleanup(move || {
            let expected = cleanup_generation.fetch_add(1, Ordering::Relaxed) + 1;
            let was_cancelled = cleanup.cancelled();
            if let Some(command) = cleanup.cancel() {
                feedback.try_set("Stopping microphone activity checks…".into());
                wasm_bindgen_futures::spawn_local(async move {
                    let result = crate::api::activity_monitor(Some(&command)).await;
                    if desired.try_get_untracked() == Some(false)
                        && cleanup_generation.load(Ordering::Relaxed) == expected
                    {
                        feedback.try_set(match result {
                            Ok(_) => "Microphone activity checks are off.".into(),
                            Err(_) => "Permission renewal stopped. Could not confirm microphone shutdown; the daemon stops checks when its current 30-second permission expires.".into(),
                        });
                    }
                });
            } else if !was_cancelled {
                feedback.try_set("Microphone activity checks are off.".into());
            }
        });
        feedback.set("Requesting microphone activity permission…".into());
        wasm_bindgen_futures::spawn_local(async move {
            let result = run(&session, feedback).await;
            if !session.cancelled() {
                if let Err(error) = result {
                    let revoke = session.cancel();
                    set_settings.try_update(|s| s.audio_driven_enabled = false);
                    let message =
                        format!("Activity checks stopped: {error} Turn the switch on to retry.");
                    feedback.try_set(message.clone());
                    if let Some(command) = revoke {
                        if crate::api::activity_monitor(Some(&command)).await.is_err()
                            && feedback.try_get_untracked().as_ref() == Some(&message)
                        {
                            feedback.try_set(format!("{message} Shutdown could not be confirmed; the current permission expires after 30 seconds without renewal."));
                        }
                    }
                }
            }
        });
    });
}

/// What the switch's feedback line says about a failed permission call.
fn describe(error: &ApiError) -> String {
    const UNREACHABLE: &str =
        "Cannot reach the capture daemon. Start or update it, then try again.";
    match error {
        ApiError::Network(_) => UNREACHABLE.into(),
        // The server's capture proxy answers 502/503 when the daemon is down.
        ApiError::Http {
            status: 502 | 503, ..
        } => UNREACHABLE.into(),
        ApiError::Http { status, detail } => format!(
            "Microphone activity request failed ({status}). {}",
            detail.as_deref().unwrap_or_default()
        ),
        ApiError::Decode(_) => {
            "Update the capture daemon to support microphone activity permission.".into()
        }
        ApiError::Timeout { after_ms } => format!(
            "The capture daemon did not respond within {} seconds.",
            after_ms / 1000
        ),
        ApiError::Aborted => "The permission request was cancelled.".into(),
    }
}

async fn run(session: &ActivityMonitorSession, feedback: RwSignal<String>) -> Result<(), String> {
    let status = crate::api::activity_monitor(None)
        .await
        .map_err(|error| describe(&error))?;
    let Some(command) = session.prepare(status).map_err(|error| error.to_string())? else {
        return Ok(());
    };
    let status = crate::api::activity_monitor(Some(&command))
        .await
        .map_err(|error| describe(&error))?;
    session
        .validate(&status)
        .map_err(|error| error.to_string())?;
    if session.cancelled() {
        return Ok(());
    }
    feedback.try_set("Microphone activity checks are on for this app session.".into());
    loop {
        gloo_timers::future::TimeoutFuture::new(
            u32::try_from(ACTIVITY_MONITOR_RENEW_MS).unwrap_or(u32::MAX),
        )
        .await;
        let Some(command) = session.renew() else {
            return Ok(());
        };
        let status = crate::api::activity_monitor(Some(&command))
            .await
            .map_err(|error| describe(&error))?;
        session
            .validate(&status)
            .map_err(|error| error.to_string())?;
        if session.cancelled() {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_failures_read_as_capture_daemon_states() {
        let unreachable = "Cannot reach the capture daemon. Start or update it, then try again.";
        assert_eq!(describe(&ApiError::Network("x".into())), unreachable);
        assert_eq!(describe(&ApiError::http(502, "")), unreachable);
        assert_eq!(describe(&ApiError::http(503, "")), unreachable);
        assert_eq!(
            describe(&ApiError::http(409, r#"{"error":"busy"}"#)),
            "Microphone activity request failed (409). busy"
        );
        assert_eq!(
            describe(&ApiError::Timeout { after_ms: 5_000 }),
            "The capture daemon did not respond within 5 seconds."
        );
        assert!(describe(&ApiError::Decode("x".into())).starts_with("Update the capture daemon"));
    }
}
