//! Capture and playback stay in memory for the lifetime of the Voice pane.
use super::*;
use crate::voice_capture::sample::{self, Clip, Cursor, Meter};
use bruhs_models::num::ToIntSaturating;
use std::sync::{atomic::Ordering, mpsc, Arc};

#[derive(Default)]
pub(super) struct MicrophoneTest {
    clip: Option<Clip>,
    meter: Arc<Meter>,
    cursor: Arc<Cursor>,
    capture: Option<mpsc::Sender<bool>>,
    playback: Option<mpsc::Sender<()>>,
    generation: u64,
    error: Option<String>,
}
impl MicrophoneTest {
    pub(super) fn clear(&mut self) {
        *self = Self {
            generation: self.generation.wrapping_add(1),
            ..Default::default()
        };
    }
    pub(super) fn busy(&self) -> bool {
        self.capture.is_some() || self.playback.is_some()
    }
    pub(super) fn fraction(&self) -> f64 {
        let ms = self.cursor.ms.load(Ordering::Relaxed);
        self.clip
            .as_ref()
            .filter(|c| c.duration_ms() > 0)
            .map_or(0., |c| 100. * f64::from(ms) / f64::from(c.duration_ms()))
    }
    pub(super) fn seek(&self, percent: f64) {
        if let Some(clip) = &self.clip {
            self.cursor.seek(
                (percent.clamp(0., 100.) * f64::from(clip.duration_ms()) / 100.)
                    .round()
                    .to_u32_saturating(),
            );
        }
    }
}
impl Workspace {
    fn tick_microphone_test(&mut self, cx: &mut Context<'_, Self>) {
        let generation = self.microphone_test.generation;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(90))
                .await;
            if !this
                .update(cx, |this, cx| {
                    if this.microphone_test.generation != generation || !this.microphone_test.busy()
                    {
                        return false;
                    }
                    if !this.settings_visible() || this.settings_section != SettingsSection::Voice {
                        this.microphone_test.clear();
                        return false;
                    }
                    cx.notify();
                    true
                })
                .unwrap_or(false)
            {
                break;
            }
        })
        .detach();
    }
    fn record_mic_test(&mut self, cx: &mut Context<'_, Self>) {
        let state = &mut self.microphone_test;
        state.clear();
        let generation = state.generation;
        let meter = state.meter.clone();
        let (stop, rx) = mpsc::channel();
        state.capture = Some(stop);
        let order = self.dictation_prefs.ordered_microphones();
        let task = cx.background_spawn(async move { sample::record(&order, rx, &meter) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.microphone_test;
                if state.generation != generation {
                    return;
                }
                state.capture = None;
                match result {
                    Ok(clip) => state.clip = Some(clip),
                    Err(e) => state.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        self.tick_microphone_test(cx);
        cx.notify();
    }
    fn play_mic_test(&mut self, cx: &mut Context<'_, Self>) {
        let state = &mut self.microphone_test;
        if let Some(stop) = state.playback.take() {
            let _ = stop.send(());
            state.generation = state.generation.wrapping_add(1);
            cx.notify();
            return;
        }
        let Some(clip) = state.clip.clone() else {
            return;
        };
        state.error = None;
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        // Each playback owns its cursor. An old output callback cannot rewind a resumed clip.
        let position = state.cursor.ms.load(Ordering::Relaxed);
        state.cursor = Arc::new(Cursor::default());
        state.cursor.seek(if position >= clip.duration_ms() {
            0
        } else {
            position
        });
        let cursor = state.cursor.clone();
        let (stop, rx) = mpsc::channel();
        state.playback = Some(stop);
        let task = cx.background_spawn(async move { sample::play(clip, cursor, rx) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let state = &mut this.microphone_test;
                if state.generation == generation {
                    state.playback = None;
                    state.error = result.err();
                    cx.notify();
                }
            });
        })
        .detach();
        self.tick_microphone_test(cx);
        cx.notify();
    }

    pub(super) fn microphone_test_row(&self, cx: &Context<'_, Self>) -> AnyElement {
        let state = &self.microphone_test;
        let button = |id: &'static str, label: &'static str, disabled| {
            web_button(
                id,
                WebButtonVariant::Outline,
                WebButtonSize::Small,
                ButtonState::disabled_if(disabled),
            )
            .aria_label(label)
            .child(label)
        };
        let mut panel = div().flex().flex_col().gap_1();
        let mut actions = div().flex().flex_wrap().items_center().gap_1();
        if let Some(tx) = &state.capture {
            let stop = tx.clone();
            let elapsed = state.meter.elapsed_ms.load(Ordering::Relaxed);
            let peak = state.meter.peak_percent.load(Ordering::Relaxed);
            panel = panel.child(div().text_xs().child(format!(
                "Recording {:.1} s · Input level {peak}%",
                f64::from(elapsed) / 1000.
            )));
            actions = actions.child(on_press_view(
                button("mic-test-stop", "Stop recording", false),
                cx,
                move |_, _, _| {
                    let _ = stop.send(true);
                },
            ));
        } else {
            if let Some(clip) = &state.clip {
                let ms = state.cursor.ms.load(Ordering::Relaxed);
                panel = panel
                    .child(div().w_full().child(self.render_settings_slider(
                        settings_ui::SettingsSlider::MicrophonePlayback,
                        cx,
                    )))
                    .child(div().text_xs().child(format!(
                        "{:.1} / {:.1} s",
                        f64::from(ms) / 1000.,
                        f64::from(clip.duration_ms()) / 1000.
                    )));
                actions = actions.child(on_press_view(
                    button(
                        "mic-test-play",
                        if state.playback.is_some() {
                            "Pause recording"
                        } else {
                            "Play recording"
                        },
                        false,
                    ),
                    cx,
                    |this, _, cx| this.play_mic_test(cx),
                ));
            }
            actions = actions.child(on_press_view(
                button(
                    "voice-test-mic",
                    if state.clip.is_none() {
                        "Test microphone"
                    } else {
                        "Record again"
                    },
                    self.voice_testing,
                ),
                cx,
                |this, _, cx| this.record_mic_test(cx),
            ));
        }
        if state.capture.is_some() || state.clip.is_some() {
            actions = actions.child(on_press_view(
                button(
                    "voice-mic-discard",
                    if state.capture.is_some() {
                        "Cancel microphone test"
                    } else {
                        "Discard recording"
                    },
                    false,
                ),
                cx,
                |this, _, cx| {
                    this.microphone_test.clear();
                    cx.notify();
                },
            ));
        }
        panel = panel.child(actions);
        if let Some(e) = &state.error {
            panel = panel.child(div().text_xs().child(e.clone()));
        }
        self.settings_row(
            "desktop-settings-row-voice-test-mic",
            "Test microphone",
            "Record up to ten seconds and play it back on this device.",
            settings_ui::control_column(panel.into_any_element()),
        )
    }
}
