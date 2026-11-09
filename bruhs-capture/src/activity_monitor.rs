//! Background activity monitor — polls the default microphone RMS every
//! `POLL_INTERVAL_SECS` seconds. When sustained activity exceeds
//! `START_ACTIVITY_SECS` continuously, emits `MeetingEvent::AudioActivityDetected`
//! and resets the counter.
//!
//! The worker is idle until a short-lived explicit consent lease is granted.
//! While a recording is
//! active (`CaptureState::current` is `Some`), the consecutive-active
//! counter is reset so that an ongoing meeting's audio does not self-trigger
//! a second prompt.
//!
//! If cpal fails to open the input device (permission denied, no device),
//! the error is logged and the task retries after `RETRY_SECS`.

use bruhs_models::num::ToFloatLossy;
use std::sync::Arc;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::Semaphore;

use bruhs_models::MeetingEvent;

use crate::state::CaptureState;

pub mod consent;
use consent::SamplePermit;

/// How often (seconds) we sample the microphone RMS.
const POLL_INTERVAL_SECS: u64 = 5;

/// How long mic activity must be sustained before we fire the event.
const START_ACTIVITY_SECS: u64 = 60;

/// RMS threshold above which a window is considered "active speech".
/// Matches the live-transcribe VAD threshold.
const RMS_THRESHOLD: f32 = 0.01;

/// How long (seconds) to sample when computing the RMS window.
const SAMPLE_WINDOW_SECS: u64 = 1;

/// How long to wait before retrying after a cpal error.
const RETRY_SECS: u64 = 60;

/// Spawn a process-lifetime worker, idle until explicitly authorized.
pub fn spawn(state: Arc<CaptureState>) {
    tokio::spawn(run(state, Arc::new(probe_rms)));
}

async fn run<F>(state: Arc<CaptureState>, sample: Arc<F>)
where
    F: Fn(SamplePermit) -> Result<f32, String> + Send + Sync + 'static,
{
    let mut consecutive_active_secs: u64 = 0;
    let mut activity_window_start_ms: f64 = 0.0;
    let mut epoch = String::new();

    loop {
        // A probe and a capture transition cannot open the microphone at once.
        let Some(reservation) = state.lifecycle.try_enter() else {
            consecutive_active_secs = 0;
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        let recording_active = state.with_current(|cur| cur.is_some());
        if recording_active {
            drop(reservation);
            consecutive_active_secs = 0;
            tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
            continue;
        }

        let Some(guard) = state.monitor.acquire() else {
            drop(reservation);
            consecutive_active_secs = 0;
            activity_window_start_ms = 0.0;
            tokio::select! {
                () = state.monitor.changed.notified() => {},
                () = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
            continue;
        };
        if epoch != guard.0.epoch {
            epoch.clone_from(&guard.0.epoch);
            consecutive_active_secs = 0;
            activity_window_start_ms = 0.0;
        }
        let sampler = sample.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _reservation = reservation;
            let result = if guard.0.authorized() {
                sampler(guard.0.clone())
            } else {
                Err("Activity monitoring permission revoked.".into())
            };
            (guard, result)
        })
        .await;
        let Ok((guard, rms_result)) = result else {
            tracing::warn!("activity monitor probe panicked");
            tokio::time::sleep(Duration::from_secs(RETRY_SECS)).await;
            continue;
        };
        if !guard.0.authorized() {
            continue;
        }
        guard
            .0
            .report_probe_error(rms_result.as_ref().err().map(String::as_str));
        let mut pause = POLL_INTERVAL_SECS;
        match rms_result {
            Ok(rms) => {
                if rms > RMS_THRESHOLD {
                    if consecutive_active_secs == 0 {
                        // Record the wall-clock start of this active window
                        // (approximate — within POLL_INTERVAL_SECS precision).
                        activity_window_start_ms = wall_clock_ms();
                    }
                    consecutive_active_secs += POLL_INTERVAL_SECS;
                    tracing::debug!(
                        "activity_monitor: rms={rms:.4} active_secs={consecutive_active_secs}"
                    );
                    if consecutive_active_secs >= START_ACTIVITY_SECS {
                        tracing::info!(
                            "activity_monitor: sustained activity detected ({}s), emitting AudioActivityDetected",
                            consecutive_active_secs
                        );
                        guard.0.while_authorized(|| {
                            state.emit(MeetingEvent::AudioActivityDetected {
                                started_at_ms: activity_window_start_ms,
                            });
                        });
                        consecutive_active_secs = 0;
                        activity_window_start_ms = 0.0;
                    }
                } else {
                    consecutive_active_secs = 0;
                    activity_window_start_ms = 0.0;
                }
            }
            Err(e) => {
                tracing::warn!("activity_monitor: cpal error, retrying in {RETRY_SECS}s: {e}");
                consecutive_active_secs = 0;
                activity_window_start_ms = 0.0;
                pause = RETRY_SECS;
            }
        }
        drop(guard);
        tokio::select! {
            () = state.monitor.changed.notified() => {},
            () = tokio::time::sleep(Duration::from_secs(pause)) => {},
        }
    }
}

/// Collect `SAMPLE_WINDOW_SECS` of samples from the default input device.
///
/// CPAL's non-Send stream stays on this blocking thread. No samples are saved.
fn probe_rms(permit: SamplePermit) -> Result<f32, String> {
    use std::sync::Mutex;

    if !permit.authorized() {
        return Err("Activity monitoring permission revoked.".into());
    }
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default input device".to_string())?;

    let config = device
        .default_input_config()
        .map_err(|e| format!("default_input_config: {e}"))?;

    let sample_rate = config.sample_rate().0 as usize;
    let channels = config.channels() as usize;
    // Number of frames (per-channel sample groups) we want to accumulate.
    let frames_wanted = sample_rate * usize::try_from(SAMPLE_WINDOW_SECS).unwrap_or(usize::MAX);

    // Accumulated squared samples for RMS; store as f32 sum + count.
    let sum_sq = Arc::new(Mutex::new(0f64));
    let sample_count = Arc::new(Mutex::new(0usize));
    let done = Arc::new(Semaphore::new(0));

    let sum_sq_clone = sum_sq.clone();
    let count_clone = sample_count.clone();
    let done_clone = done.clone();
    let callback_permit = permit.clone();

    let err_fn = |e| tracing::warn!("activity_monitor cpal stream error: {e}");

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream::<f32, _, _>(
                &config.into(),
                move |data: &[f32], _| {
                    if !callback_permit.authorized() {
                        return;
                    }
                    // Mix down to mono by averaging channels, accumulate sum-of-squares.
                    let mut s = sum_sq_clone
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut c = count_clone
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    for frame in data.chunks(channels) {
                        if *c >= frames_wanted {
                            break;
                        }
                        let mono: f32 = frame.iter().sum::<f32>() / channels.to_f32_lossy();
                        *s += f64::from(mono).powi(2);
                        *c += 1;
                    }
                    drop(s);
                    if *c >= frames_wanted {
                        done_clone.add_permits(1);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_input_stream(f32): {e}"))?,
        cpal::SampleFormat::I16 => device
            .build_input_stream::<i16, _, _>(
                &config.into(),
                move |data: &[i16], _| {
                    if !callback_permit.authorized() {
                        return;
                    }
                    let mut s = sum_sq_clone
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut c = count_clone
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    for frame in data.chunks(channels) {
                        if *c >= frames_wanted {
                            break;
                        }
                        let mono: f32 = frame
                            .iter()
                            .map(|&s| f32::from(s) / f32::from(i16::MAX))
                            .sum::<f32>()
                            / channels.to_f32_lossy();
                        *s += f64::from(mono).powi(2);
                        *c += 1;
                    }
                    drop(s);
                    if *c >= frames_wanted {
                        done_clone.add_permits(1);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("build_input_stream(i16): {e}"))?,
        fmt => return Err(format!("unsupported sample format: {fmt:?}")),
    };

    if !permit.authorized() {
        return Err("Activity monitoring permission revoked.".into());
    }
    stream.play().map_err(|e| format!("play stream: {e}"))?;

    // Block until we have enough samples (or 2s timeout).
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while done.available_permits() == 0 {
        if !permit.authorized() {
            break;
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(stream);

    let s = *sum_sq
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let c = *sample_count
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if c == 0 {
        return Ok(0.0);
    }
    Ok((s / c.to_f64_lossy()).sqrt().to_f32_lossy())
}

fn wall_clock_ms() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Just verify constants are sensible.
    #[test]
    fn activity_thresholds_are_sensible() {
        const { assert!(POLL_INTERVAL_SECS <= START_ACTIVITY_SECS) };
        const { assert!(RMS_THRESHOLD > 0.0) };
        const { assert!(RMS_THRESHOLD < 1.0) };
    }

    #[test]
    fn wall_clock_ms_is_plausible() {
        let ms = wall_clock_ms();
        // Any timestamp after 2024-01-01 is fine.
        assert!(ms > 1_704_067_200_000.0);
    }
}
