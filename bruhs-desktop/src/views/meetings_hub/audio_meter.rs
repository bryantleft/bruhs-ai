//! The live pill's input level bars, as their own entity.
//!
//! Levels arrive several times a second while recording. Keeping them out of
//! [`super::MeetingsHub`] means a level update notifies only this meter, not
//! the hub's observers (the workspace and its transcript views).

use crate::theme;
use bruhs_models::logic::dither::meter_column;
use gpui::prelude::*;
use gpui::{div, px, Context, Div, Hsla, Window};

// One braille cell (Bruhs AGENTS.md section 19), the same size as web's
// `BrailleMeter`: 4px dots, 2px apart in a column, columns 3px apart.
const DOT_PX: f32 = 4.0;
const ROW_GAP_PX: f32 = 2.0;
const COLUMN_GAP_PX: f32 = 3.0;

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

/// One column of dots, lit from the bottom for `level`.
fn column(level: f32, lit: Hsla) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(ROW_GAP_PX))
        .children(meter_column(level).map(|on| {
            div()
                .size(px(DOT_PX))
                .rounded_full()
                .bg(if on { lit } else { theme::longan_700() })
        }))
}

impl Render for AudioLevelMeter {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "desktop-braille-meter".to_string())
            .flex()
            .flex_none()
            .gap(px(COLUMN_GAP_PX))
            .child(column(self.mic_rms, theme::dragonfruit_500()))
            .when_some(self.sys_rms, |el, sys| {
                el.child(column(sys, theme::pandan_500()))
            })
    }
}

#[cfg(test)]
mod tests {
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
}
