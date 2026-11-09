//! The live pill's input level bars, as their own entity.
//!
//! Levels arrive several times a second while recording. Keeping them out of
//! [`super::MeetingsHub`] means a level update notifies only this meter, not
//! the hub's observers (the workspace and its transcript views).

use crate::theme;
use bruhs_models::num::{ToFloatLossy, ToIntSaturating};
use gpui::prelude::*;
use gpui::{div, px, Context, Window};

const METER_HEIGHT_PX: f32 = 20.0;

#[derive(Debug, Default)]
pub struct AudioLevelMeter {
    mic_rms: f32,
    /// `None` while system audio is not being captured.
    sys_rms: Option<f32>,
}

impl AudioLevelMeter {
    #[cfg(test)]
    pub fn levels(&self) -> (f32, Option<f32>) {
        (self.mic_rms, self.sys_rms)
    }

    /// Always redraws, even for an unchanged level: the tick is also what
    /// keeps the pill's elapsed-time label current.
    pub fn set(&mut self, mic_rms: f32, sys_rms: Option<f32>, cx: &mut Context<'_, Self>) {
        self.mic_rms = mic_rms;
        self.sys_rms = sys_rms;
        cx.notify();
    }
}

/// Bar height for an RMS level in [0, 1]: 2–20 px.
fn bar_height(rms: f32) -> f32 {
    (rms * METER_HEIGHT_PX)
        .to_u32_saturating()
        .clamp(2, METER_HEIGHT_PX.to_u32_saturating())
        .to_f32_lossy()
}

impl Render for AudioLevelMeter {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        let mut levels = div()
            .flex()
            .items_end()
            .gap(px(2.0))
            .h(px(METER_HEIGHT_PX))
            .child(
                div()
                    .w(px(3.0))
                    .h(px(bar_height(self.mic_rms)))
                    .rounded(px(4.0))
                    .bg(theme::dragonfruit_500()),
            );
        if let Some(sys) = self.sys_rms {
            levels = levels.child(
                div()
                    .w(px(3.0))
                    .h(px(bar_height(sys)))
                    .rounded(px(4.0))
                    .bg(theme::pandan_500()),
            );
        }
        levels
    }
}

#[cfg(test)]
mod tests {
    use super::bar_height;
    use crate::server_client::ServerClient;
    use crate::views::meetings_hub::{MeetingPrefs, MeetingsHub};
    use bruhs_models::MeetingEvent;
    use gpui::{AppContext as _, TestAppContext};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::Arc;

    #[gpui::test]
    fn a_level_tick_updates_the_meter_without_notifying_the_hub(cx: &mut TestAppContext) {
        let runtime = Arc::new(tokio::runtime::Runtime::new().unwrap());
        let hub = cx.new(|cx| {
            MeetingsHub::new(
                ServerClient::new("http://127.0.0.1:9", "http://127.0.0.1:9").unwrap(),
                runtime,
                MeetingPrefs::default(),
                cx,
            )
        });
        hub.update(cx, |hub, _| hub.server_online = true);
        let hub_notifies = Rc::new(Cell::new(0));
        let count = hub_notifies.clone();
        let _observer = cx.update(|cx| cx.observe(&hub, move |_, _| count.set(count.get() + 1)));

        for level in [0.1, 0.4, 0.8] {
            hub.update(cx, |hub, cx| {
                hub.apply_meeting_event(
                    MeetingEvent::AudioLevel {
                        meeting_id: "m-1".into(),
                        mic_rms: level,
                        sys_rms: Some(level / 2.0),
                    },
                    cx,
                );
            });
            cx.run_until_parked();
        }

        assert_eq!(hub_notifies.get(), 0);
        let meter = hub.read_with(cx, |hub, _| hub.audio_meter.clone());
        assert_eq!(
            meter.read_with(cx, |meter, _| meter.levels()),
            (0.8, Some(0.4))
        );
    }

    #[test]
    fn bars_stay_between_two_and_twenty_pixels() {
        assert_eq!(bar_height(0.0), 2.0);
        assert_eq!(bar_height(0.5), 10.0);
        assert_eq!(bar_height(1.0), 20.0);
        assert_eq!(bar_height(3.0), 20.0);
    }
}
