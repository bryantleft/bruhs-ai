//! Quota for the provider of the visible thread, under Appearance → Thread details.
use super::*;
use bruhs_models::agent_extensions::QuotaWindow;
use std::time::Duration;

#[derive(IntoElement)]
pub(super) struct ThreadUsage(pub(super) AgentProvider);

impl RenderOnce for ThreadUsage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let provider = self.0;
        window.use_keyed_state(
            SharedString::from(format!("thread-usage-{}", provider.as_str())),
            cx,
            |_, cx| {
                let mut state = Usage {
                    provider,
                    windows: Vec::new(),
                    loaded: false,
                    failed: false,
                    polling: None,
                };
                if cx.has_global::<crate::app_state::DesktopState>() {
                    state.start(cx);
                }
                state
            },
        )
    }
}

struct Usage {
    provider: AgentProvider,
    windows: Vec<QuotaWindow>,
    loaded: bool,
    failed: bool,
    polling: Option<Task<()>>,
}

impl Usage {
    fn start(&mut self, cx: &mut Context<'_, Self>) {
        let provider = self.provider;
        self.failed = false;
        let runtime = cx
            .global::<crate::app_state::DesktopState>()
            .runtime
            .clone();
        self.polling = Some(cx.spawn(async move |this, cx| loop {
            let result = runtime
                .spawn(async move {
                    crate::server_client::ServerClient::workbench_client()
                        .quota_windows(provider)
                        .await
                })
                .await;
            if this
                .update(cx, |state, cx| {
                    state.loaded = true;
                    match result {
                        Ok(Ok(windows)) => {
                            state.windows = windows;
                            state.failed = false;
                        }
                        _ => state.failed = true,
                    }
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
        }));
    }
}

impl Render for Usage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let now = chrono::Utc::now().timestamp();
        let active: Vec<_> = self
            .windows
            .iter()
            .filter(|w| w.resets_at.is_none_or(|end| end > now))
            .collect();
        let mut section = div()
            .id("detail-usage")
            .px_2()
            .py_1()
            .flex()
            .flex_col()
            .gap_1()
            .min_w_0()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::rambutan_300())
                    .child(format!("{} usage limits", provider_label(self.provider))),
            );
        if self.failed {
            let retry = web_button(
                "detail-usage-retry",
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::Enabled,
            )
            .aria_label("Retry usage limits")
            .child("Retry")
            .on_click(cx.listener(|this, _, _, cx| {
                this.start(cx);
                cx.notify();
            }));
            section = section.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(if active.is_empty() {
                        "Usage limits unavailable"
                    } else {
                        "Last reported limits"
                    })
                    .child(retry)
                    .text_xs(),
            );
        } else if active.is_empty() {
            section = section.child(div().text_xs().text_color(theme::rambutan_400()).child(
                if self.loaded {
                    "No current limits reported"
                } else {
                    "Checking limits…"
                },
            ));
        }
        let mut account = None;
        let several = active
            .first()
            .is_some_and(|first| active.iter().any(|w| w.account != first.account));
        for w in active {
            if several && account != Some(&w.account) {
                section = section.child(div().text_xs().child(w.account.clone()));
                account = Some(&w.account);
            }
            section = section.child(window_row(w, now));
        }
        section
    }
}

fn label_and_minutes(w: &QuotaWindow) -> (String, Option<u32>) {
    if w.provider == "codex" {
        let label = match (w.name.as_str(), w.duration_minutes) {
            ("individual", _) => "Monthly credit limit",
            (_, Some(43200..)) => "Monthly",
            (_, Some(10080..)) => "Weekly",
            (_, Some(_)) => "Session",
            ("secondary", None) => "Weekly",
            _ => "Session",
        };
        return (label.into(), w.duration_minutes);
    }
    match w.name.as_str() {
        "five_hour" => ("Session".into(), Some(300)),
        "seven_day" => ("Weekly".into(), Some(10080)),
        name if name.starts_with("seven_day_") => {
            (format!("Weekly · {}", human_name(&name[10..])), Some(10080))
        }
        name => (human_name(name), w.duration_minutes),
    }
}
fn human_name(name: &str) -> String {
    let mut s = name.replace('_', " ");
    if let Some(first) = s.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    s
}
pub(in crate::views::workspace) fn countdown(seconds: i64) -> String {
    let mins = seconds / 60;
    if mins >= 1440 {
        format!("{}d {}h", mins / 1440, mins % 1440 / 60)
    } else if mins >= 60 {
        format!("{}h {}m", mins / 60, mins % 60)
    } else if mins > 0 {
        format!("{mins}m")
    } else {
        "<1m".into()
    }
}

pub(in crate::views::workspace) fn window_row(w: &QuotaWindow, now: i64) -> AnyElement {
    let (label, minutes) = label_and_minutes(w);
    let left = (100. - w.used_percent).clamp(0., 100.).round();
    let remaining = w.resets_at.map(|end| end.saturating_sub(now).max(0));
    let time_fraction = remaining
        .zip(minutes.filter(|m| *m > 0))
        .map(|(s, m)| {
            (bruhs_models::num::ToFloatLossy::to_f64_lossy(s) / (f64::from(m) * 60.)).clamp(0., 1.)
        });
    let pace = time_fraction.map(|f| {
        let delta = 100. - w.used_percent - f * 100.;
        if delta < -5. {
            "Ahead of pace"
        } else if delta > 5. {
            "Under pace"
        } else {
            "On pace"
        }
    });
    let mut hint = format!("{label} · {left:.0}% left");
    if let Some(f) = time_fraction {
        hint.push_str(&format!(" · {:.0}% of the window left", f * 100.));
    }
    if let Some(p) = pace {
        hint.push_str(&format!(" · {p}"));
    }
    if let Some(t) = w
        .resets_at
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
    {
        hint.push_str(&format!(
            " · Resets {}",
            t.with_timezone(&chrono::Local)
                .format(&format!("%b %-d {}", crate::clock_format::pattern()))
        ));
    }
    let pace_icon = pace.map(|p| match p {
        "Ahead of pace" => "↗",
        "Under pace" => "↘",
        _ => "→",
    });
    let accent = if w.provider == "claude" {
        theme::lychee_400()
    } else {
        theme::blueberry_400()
    };
    let bar = div()
        .relative()
        .flex_1()
        .min_w(px(20.))
        .h(px(8.))
        .rounded_full()
        .bg(theme::longan_700())
        .when(left > 0., |el| {
            el.child(
                div()
                    .h_full()
                    .rounded_full()
                    .w(relative(bruhs_models::num::ToFloatLossy::to_f32_lossy(
                        left / 100.,
                    )))
                    .bg(accent),
            )
        })
        .when_some(time_fraction, |el, f| {
            el.child(
                div()
                    .absolute()
                    .left(relative(bruhs_models::num::ToFloatLossy::to_f32_lossy(
                        f.clamp(0., 0.99),
                    )))
                    .top(px(-2.))
                    .w(px(2.))
                    .h(px(12.))
                    .bg(theme::rambutan_100()),
            )
        });
    let stacked = label.chars().count() > 8;
    let meters = div()
        .flex()
        .items_center()
        .min_w_0()
        .gap_1()
        .when(!stacked, |row| {
            row.child(
                div()
                    .w(px(60.))
                    .flex_shrink_0()
                    .truncate()
                    .child(label.clone()),
            )
        })
        .child(
            div()
                .w(px(66.))
                .whitespace_nowrap()
                .flex_shrink_0()
                .when(!stacked, |value| value.text_right())
                .child(format!("{left:.0}% left")),
        )
        .child(bar)
        .child(
            div()
                .w(px(82.))
                .whitespace_nowrap()
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .text_color(theme::rambutan_400())
                .when_some(pace_icon, |el, icon| el.child(icon))
                .when_some(remaining, |el, s| el.child(format!("in {}", countdown(s)))),
        );
    div()
        .id(SharedString::from(format!(
            "detail-usage-{}-{}",
            w.account,
            w.window_id.as_deref().unwrap_or(&w.name)
        )))
        .tab_stop(true)
        .aria_label(hint.clone())
        .tooltip(move |win, cx| Tooltip::new(hint.clone()).build(win, cx))
        .flex()
        .flex_col()
        .min_w_0()
        .text_xs()
        .py_1()
        .when(stacked, |row| {
            row.child(div().min_w_0().truncate().child(label))
        })
        .child(meters)
        .into_any_element()
}
