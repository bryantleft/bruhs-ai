//! Debug-only, isolated OS window acceptance. Never builds DesktopState, reads
//! user workspace files, starts the hub, observes focus, or records anything.
use crate::views::web_button::{web_button, ButtonState, WebButtonSize, WebButtonVariant};
use crate::views::{meeting_dock, meetings_hub::MeetingsHub};
use crate::{assets, theme};
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
use gpui::prelude::*;
use gpui::*;
use std::sync::Arc;

pub fn run() {
    gpui_platform::application().with_assets(assets::DesktopAssets).run(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.text_system().add_fonts(assets::embedded_fonts()).expect("fixture fonts");
        let runtime = Arc::new(tokio::runtime::Runtime::new().expect("fixture runtime"));
        let hub = cx.new(|cx| MeetingsHub::synthetic_dock_fixture(runtime, cx));
        let window = cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(640.0), px(360.0)), cx))),
            ..Default::default()
        }, |window, cx| {
            window.set_window_title("bruhs, Synthetic dock acceptance");
            window.on_window_should_close(cx, |_, cx| { cx.quit(); true });
            cx.new(|_| Fixture { hub, dock: None })
        }).expect("fixture window");
        cx.activate(true);
        window.update(cx, |fixture, window, cx| fixture.open(window, cx)).expect("open fixture dock");
        eprintln!("SYNTHETIC_DOCK_READY: no recording, focus monitoring, workspace files or service connections");
    });
}

struct Fixture {
    hub: Entity<MeetingsHub>,
    dock: Option<WindowHandle<meeting_dock::MeetingDock>>,
}

impl Fixture {
    fn open(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.hub.read(cx).live_meeting_id.is_none() {
            return;
        }
        if let Some(dock) = self.dock.take() {
            let _ = dock.update(cx, |_, window, _| window.remove_window());
        }
        let dock = meeting_dock::open(self.hub.clone(), window, cx).expect("synthetic dock");
        dock.update(cx, |dock, window, cx| {
            dock.install_fixture_diagnostics(window, cx)
        })
        .expect("fixture position diagnostics");
        self.dock = Some(dock);
    }

    fn end(&mut self, cx: &mut Context<'_, Self>) {
        self.hub.update(cx, |hub, cx| {
            if let Some(id) = hub.live_meeting_id.clone() {
                hub.apply_meeting_event(
                    bruhs_models::MeetingEvent::Ended {
                        meeting_id: id,
                        ended_at_ms: bruhs_models::EpochMs::now().as_f64(),
                        status: bruhs_models::MeetingStatus::Ended,
                    },
                    cx,
                );
            }
        });
        cx.notify();
    }
}

impl Render for Fixture {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let ended = self.hub.read(cx).live_meeting_id.is_none();
        div().id("synthetic-fixture").size_full().p_6().bg(theme::longan_950()).text_color(theme::rambutan_200())
            .font_family("Monaspace Argon").text_size(px(14.0)).flex().flex_col().gap_4()
            .child("Synthetic meeting dock acceptance")
            .child("No audio, screen capture, focused-app monitoring, saved workspace or service connections.")
            .child("The dock’s Stop control is intentionally disabled. Test Move, Open and close/restore only.")
            .child(if ended { "Synthetic meeting ended." } else { "Synthetic meeting active." })
            .child(web_button("fixture-open", WebButtonVariant::Outline, WebButtonSize::Default, ButtonState::disabled_if(ended))
                .min_h(px(44.0)).aria_label("Open synthetic meeting dock")
                .map(|el| on_press_view(el, cx, |this, window, cx| this.open(window, cx)))
                .child("Open dock"))
            .child(web_button("fixture-end", WebButtonVariant::Outline, WebButtonSize::Default, ButtonState::disabled_if(ended))
                .min_h(px(44.0)).aria_label("End synthetic meeting")
                .map(|el| on_press_view(el, cx, |this, _, cx| this.end(cx)))
                .child("End synthetic meeting"))
    }
}
