use super::*;
use bruhs_models::{
    agent_extensions::{UsageCreditChoice, UsageLimitAccount},
    num::ToFloatLossy as _,
};
use std::collections::BTreeMap;

use pools::{Kind, Pool, Segment};

impl UsageWindow {
    pub(super) fn render_limits(&self, narrow: bool, cx: &Context<'_, Self>) -> Div {
        let mut content = div().flex().flex_col().gap(px(32.));
        if let Some(notice) = self.limits.notice() {
            content = content.child(muted(notice.to_owned()));
        }
        if let Some(message) = &self.credit.message {
            content = content.child(muted(message.clone()));
        }
        let Some(report) = &self.limits.data else {
            return content;
        };
        let now = chrono::Utc::now().timestamp();
        let mut providers = BTreeMap::<(usize, &str), Vec<_>>::new();
        for account in &report.accounts {
            let order = [
                "Codex",
                "Claude",
                "Grok",
                "OpenCode",
                "Antigravity",
                "Cursor",
            ];
            let rank = order
                .iter()
                .position(|name| *name == account.provider)
                .unwrap_or(order.len());
            providers
                .entry((rank, &account.provider))
                .or_default()
                .push(account);
        }
        if providers.is_empty() && report.external.is_empty() && report.notices.is_empty() {
            content = content.child(muted("No providers reported subscription limits."));
        }
        content = content.children(providers.into_iter().map(|((_, provider), accounts)| {
            let (order, rows) = pools::collect(provider, &accounts, now);
            let logo = provider.to_ascii_lowercase().parse::<AgentProvider>().ok();
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .children(logo.map(usage_provider_mark))
                        .child(title(provider.to_owned())),
                )
                .children(rows.iter().enumerate().map(|(index, (kind, row))| {
                    let card = PoolCard {
                        provider,
                        row,
                        index,
                        kind: *kind,
                        order: &order,
                        accounts: &accounts,
                        now,
                    };
                    self.render_pool(&card, narrow, cx)
                }))
        }));
        content = content.children(report.external.iter().filter_map(|page| {
            let url = reqwest::Url::parse(&page.url).ok()?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.has_host()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return None;
            }
            Some(
                card_surface()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(title(page.label.clone()))
                            .child(muted(page.sources.join(" · "))),
                    )
                    .child(Self::button(
                        SharedString::from(format!("usage-management-{}", page.url)),
                        "Manage usage ↗",
                        WebButtonVariant::Outline,
                        false,
                        cx,
                        move |_, _, cx| cx.open_url(url.as_str()),
                    )),
            )
        }));
        if !report.notices.is_empty() {
            content = content.child(
                card_surface()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(report.notices.iter().map(|message| muted(message.clone()))),
            );
        }
        let expand = Self::button(
            "usage-recorded",
            if self.show_recorded {
                "Hide recorded windows"
            } else {
                "Recorded windows"
            },
            WebButtonVariant::Ghost,
            false,
            cx,
            |this, _, cx| {
                this.show_recorded = !this.show_recorded;
                if this.show_recorded {
                    this.refresh_recorded(cx);
                }
                cx.notify();
            },
        );
        content.child(
            div()
                .flex()
                .flex_col()
                .items_start()
                .gap_3()
                .child(expand)
                .when(self.show_recorded, |el| el.child(self.recorded_windows())),
        )
    }

    fn refresh_recorded(&mut self, cx: &mut Context<'_, Self>) {
        if self.recorded.loading {
            return;
        }
        self.recorded.loading = true;
        let request = cx.background_spawn(async {
            use bruhs_core::agent_extensions::{directory, snapshot_at};
            directory()
                .and_then(|path| snapshot_at(&path))
                .map(|report| report.limits)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                this.recorded.finish(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn recorded_windows(&self) -> Div {
        let notice = self.recorded.notice().map(str::to_owned);
        let empty = self.recorded.data.as_ref().is_some_and(Vec::is_empty);
        card_surface()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .children(notice.map(muted))
            .when(empty, |el| {
                el.child(muted("No recorded subscription windows."))
            })
            .children(self.recorded.data.iter().flatten().map(|window| {
                let observed = chrono::DateTime::from_timestamp(window.observed_at, 0)
                    .map(|date| {
                        date.with_timezone(&chrono::Local)
                            .format("%b %-d, %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(title(format!(
                        "{} · {} · {:.0}% left",
                        window.provider,
                        window.name,
                        (100. - window.used_percent).clamp(0., 100.)
                    )))
                    .child(muted(format!("{} · Observed {observed}", window.account)))
            }))
    }

    fn render_pool(&self, card: &PoolCard<'_>, narrow: bool, cx: &Context<'_, Self>) -> AnyElement {
        let row = card.row;
        let summary = div()
            .flex()
            .flex_col()
            .gap_1()
            .when(!narrow, |el| el.w(px(176.)).flex_none())
            .when(!row.label.is_empty(), |el| {
                el.child(title(row.label.clone()))
            })
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap_2()
                    .child(figure(format!("{:.0}%", row.left)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(px(14.))
                            .text_color(theme::rambutan_400())
                            .child("left")
                            .children(row.pace.map(|pace| {
                                usage_icon(pace_icon(pace), 14., theme::rambutan_400())
                            })),
                    ),
            )
            .when_some(
                row.refill.filter(|_| card.accounts.len() > 1),
                |el, (reset, refill)| {
                    el.child(muted(format!(
                        "↻ +{refill:.0}% {}",
                        reset_label(reset, card.now)
                    )))
                },
            );
        let track = div()
            .flex()
            .gap_1()
            .w_full()
            .children(card.order.iter().enumerate().map(|(position, &account)| {
                let selected = Selection {
                    provider: card.provider.to_owned(),
                    row: card.index,
                    account,
                };
                let data = card.accounts[account];
                let cell = div().relative().flex_1().min_w_0();
                match row.segments.get(&account) {
                    Some(segment) => {
                        let active = self.selection.as_ref() == Some(&selected);
                        cell.child(self.segment_button(position, data, segment, selected, card, cx))
                            .when(active, |el| {
                                el.child(bruhs_ui::menu_popover::menu_popover(
                                    size(px(180.), px(34.)),
                                    self.render_account(data, segment, card.now, cx)
                                        .id("usage-account-popover")
                                        .debug_selector(|| "usage-account-popover".into())
                                        .w(px(308.))
                                        .max_w_full()
                                        .p_3()
                                        .rounded(px(9.))
                                        .border_1()
                                        .border_color(theme::ring_modal())
                                        .bg(theme::longan_900())
                                        .on_click(|_, _, cx| cx.stop_propagation())
                                        .into_any_element(),
                                ))
                            })
                    }
                    None if card.kind == Kind::Monthly && data.zero_spend_cap => cell.child(
                        div()
                            .h(px(32.))
                            .border_1()
                            .border_color(theme::longan_700())
                            .rounded(px(5.))
                            .px_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(muted("$0 cap")),
                    ),
                    None => cell,
                }
            }));
        let right = div()
            .min_w_0()
            .when(narrow, |el| el.w_full())
            .when(!narrow, |el| el.flex_1())
            .flex()
            .flex_col()
            .gap_2()
            .child(track)
            .when(narrow || card.order.len() > 3, |el| {
                el.children(
                    card.order
                        .iter()
                        .enumerate()
                        .filter_map(|(position, &account)| {
                            let segment = row.segments.get(&account)?;
                            let data = card.accounts[account];
                            let selection = Selection {
                                provider: card.provider.to_owned(),
                                row: card.index,
                                account,
                            };
                            let label = format!(
                                "{} · {} · {:.0}% left",
                                position + 1,
                                short_name(data),
                                segment.left
                            );
                            Some(Self::button(
                                ("usage-legend", position),
                                label,
                                WebButtonVariant::Ghost,
                                self.credit.pending,
                                cx,
                                move |this, _, cx| this.select_segment(selection.clone(), cx),
                            ))
                        }),
                )
            });
        card_surface()
            .id((SharedString::from(card.provider.to_owned()), card.index))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap(px(if narrow { 12. } else { 24. }))
                    .when(narrow, |el| el.flex_col())
                    .when(!narrow, |el| el.items_center())
                    .child(summary)
                    .child(right),
            )
            .children(row.description.map(muted))
            .into_any_element()
    }

    fn segment_button(
        &self,
        position: usize,
        account: &UsageLimitAccount,
        segment: &Segment,
        selection: Selection,
        card: &PoolCard<'_>,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let name = short_name(account);
        let reset = segment.reset.map(|reset| reset_label(reset, card.now));
        let label = format!(
            "{name}, {:.0}% left{}",
            segment.left,
            reset
                .as_ref()
                .map_or_else(String::new, |reset| format!(", resets {reset}"))
        );
        let is_selected = self.selection.as_ref() == Some(&selection);
        let accent = provider_color(card.provider);
        let fraction = |percent: f64| relative((percent * 0.01).to_f32_lossy());
        let compact = card.order.len() > 3;
        let multiple = card.order.len() > 1;
        let badge = if compact {
            (position + 1).to_string()
        } else {
            format!("{:.0}%", segment.left)
        };
        let bar = div()
            .absolute()
            .inset_0()
            .rounded(px(4.))
            .overflow_hidden()
            .bg(theme::longan_800())
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(fraction(segment.left))
                    .rounded(px(5.))
                    .bg(accent.opacity(0.42)),
            )
            .when(segment.refill() > 0., |el| {
                el.child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .h_full()
                        .w(fraction(segment.refill()))
                        .bg(accent.opacity(0.08))
                        .child(refill_hatch(accent)),
                )
            })
            .when_some(segment.time_left, |el, remaining| {
                el.child(
                    div()
                        .absolute()
                        .top(px(2.))
                        .bottom(px(2.))
                        .left(relative(remaining.min(0.99).to_f32_lossy()))
                        .w(px(2.))
                        .rounded_full()
                        .bg(theme::rambutan_100()),
                )
            });
        on_press_view(
            web_button(
                ("usage-segment", position),
                WebButtonVariant::Ghost,
                WebButtonSize::Small,
                ButtonState::disabled_if(self.credit.pending),
            )
            .aria_label(label)
            .debug_selector(move || format!("usage-segment-{position}")),
            cx,
            move |this, _, cx| this.select_segment(selection.clone(), cx),
        )
        .relative()
        .overflow_hidden()
        .w_full()
        .h(px(34.))
        .min_w_0()
        .px_2()
        .aria_selected(is_selected)
        .when(is_selected, |el| el.border_color(theme::persimmon_400()))
        .child(bar)
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .px_2()
                .text_xs()
                .text_color(theme::rambutan_100())
                .when(multiple, |el| {
                    el.child(
                        div()
                            .flex_none()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(badge),
                    )
                    .when(!compact, |el| {
                        el.child(div().min_w_0().truncate().child(name.to_owned()))
                    })
                })
                .when_some(reset.filter(|_| !compact), |el, reset| {
                    el.child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px(px(6.))
                            .py(px(2.))
                            .text_size(px(11.))
                            .line_height(px(16.))
                            .font_weight(FontWeight::NORMAL)
                            .rounded(px(4.))
                            .bg(theme::longan_900().opacity(0.84))
                            .child(usage_icon(
                                crate::assets::ROTATE_CCW_ICON_PATH,
                                11.,
                                theme::rambutan_100(),
                            ))
                            .child(reset.trim_start_matches("in ").to_owned())
                            .when(account.credits > 0, |el| {
                                el.child("·")
                                    .child(usage_icon(
                                        bruhs_ui::icons::TICKET,
                                        12.,
                                        theme::rambutan_100(),
                                    ))
                                    .child(account.credits.to_string())
                            }),
                    )
                }),
        )
    }

    fn select_segment(&mut self, selection: Selection, cx: &mut Context<'_, Self>) {
        if self.credit.pending {
            return;
        }
        self.selection = if self.selection.as_ref() == Some(&selection) {
            None
        } else {
            Some(selection)
        };
        self.reveal_email = false;
        self.credit = Credit::default();
        cx.notify();
    }

    fn render_account(
        &self,
        account: &UsageLimitAccount,
        segment: &Segment,
        now: i64,
        cx: &Context<'_, Self>,
    ) -> Div {
        let identity = account.email.as_ref().map(|email| {
            Self::button(
                "usage-account-email",
                if self.reveal_email {
                    email.clone()
                } else {
                    "Reveal account email".into()
                },
                WebButtonVariant::Ghost,
                false,
                cx,
                |this, _, cx| {
                    this.reveal_email = !this.reveal_email;
                    cx.notify();
                },
            )
        });
        let mut details = div()
            .flex()
            .flex_col()
            .items_start()
            .gap_2()
            .child(title(short_name(account).to_owned()))
            .children(identity)
            .when_some(account.plan.as_ref(), |el, plan| {
                el.child(muted(format!("Plan · {plan}")))
            })
            .when_some(account.workspace.as_ref(), |el, workspace| {
                el.child(muted(format!("Workspace · {workspace}")))
            })
            .child(muted(format!("Sources · {}", account.sources.join(" · "))))
            .when_some(segment.reset, |el, reset| {
                el.child(muted(format!("Resets {}", reset_label(reset, now))))
            })
            .when_some(segment.time_left, |el, time| {
                el.child(muted(format!("{:.0}% of the window left", time * 100.)))
            });
        if let Some(choice) = &account.reset {
            details = details.child(self.render_credit(choice, account.credits, cx));
        }
        details
    }

    fn render_credit(
        &self,
        choice: &UsageCreditChoice,
        credits: usize,
        cx: &Context<'_, Self>,
    ) -> Div {
        let mut details = div().flex().flex_col().items_start().gap_2();
        if self.credit.confirming {
            let handle = choice.handle;
            details = details
                .child(muted(format!(
                    "Use one reset credit from {}? This clears the account’s current rate limits.",
                    choice.hub
                )))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(Self::button(
                            "usage-credit-cancel",
                            "Cancel",
                            WebButtonVariant::Ghost,
                            self.credit.pending,
                            cx,
                            |this, _, cx| {
                                this.credit = Credit::default();
                                cx.notify();
                            },
                        ))
                        .child(Self::button(
                            "usage-credit-confirm",
                            if self.credit.pending {
                                "Using credit…"
                            } else {
                                "Use credit"
                            },
                            WebButtonVariant::Primary,
                            self.credit.pending,
                            cx,
                            move |this, _, cx| this.redeem_credit(handle, cx),
                        )),
                );
        } else {
            details = details.child(Self::button(
                "usage-credit-open",
                format!("Reset credits available · {credits}"),
                WebButtonVariant::Outline,
                self.limits.loading,
                cx,
                |this, _, cx| {
                    this.credit.confirming = true;
                    cx.notify();
                },
            ));
        }
        details
    }

    fn redeem_credit(&mut self, handle: uuid::Uuid, cx: &mut Context<'_, Self>) {
        if self.credit.pending || self.limits.loading || !self.credit.confirming {
            return;
        }
        let Some(runtime) = cx
            .try_global::<DesktopState>()
            .map(|state| state.runtime.clone())
        else {
            return;
        };
        self.credit.pending = true;
        self.credit.message = None;
        let request = runtime.spawn(async move {
            ServerClient::workbench_client()
                .redeem_usage_credit(handle)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = match request.await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.credit.pending = false;
                match result {
                    Ok(message) => {
                        this.credit.confirming = false;
                        this.credit.message = Some(message);
                        this.refresh_limits(cx);
                    }
                    Err(error) => this.credit.message = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

struct PoolCard<'a> {
    provider: &'a str,
    row: &'a Pool,
    index: usize,
    kind: Kind,
    order: &'a [usize],
    accounts: &'a [&'a UsageLimitAccount],
    now: i64,
}

fn card_surface() -> Div {
    div()
        .p_4()
        .border_1()
        .border_color(theme::longan_800())
        .rounded(px(8.))
        .bg(theme::longan_900())
}

fn refill_hatch(accent: Hsla) -> impl IntoElement {
    canvas(
        |bounds, _, _| bounds,
        move |_, bounds, window, _| {
            let width = bounds.size.width;
            let height = bounds.size.height;
            let step = px(6.);
            let mut path = PathBuilder::stroke(px(1.));
            let mut offset = -height;
            while offset < width {
                let start = offset.max(px(0.));
                let end = (offset + height).min(width);
                path.move_to(point(
                    bounds.left() + start,
                    bounds.bottom() + offset - start,
                ));
                path.line_to(point(bounds.left() + end, bounds.bottom() + offset - end));
                offset += step;
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, accent.opacity(0.32));
            }
        },
    )
    .size_full()
}

fn provider_color(provider: &str) -> Hsla {
    match provider {
        "Codex" => theme::blueberry_400(),
        "Grok" => theme::pandan_400(),
        "Claude" => theme::lychee_400(),
        "Antigravity" => theme::mangosteen_400(),
        "Cursor" => theme::rambutan_400(),
        _ => theme::blueberry_400(),
    }
}

fn short_name(account: &UsageLimitAccount) -> &str {
    if let Some(email) = &account.email {
        email.split_once('@').map_or(email, |(name, _)| name)
    } else {
        &account.label
    }
}

fn pace_icon(pace: &str) -> &'static str {
    match pace {
        "Ahead of pace" => bruhs_ui::icons::TRENDING_UP,
        "Under pace" => bruhs_ui::icons::TRENDING_DOWN,
        _ => bruhs_ui::icons::GAUGE,
    }
}

fn usage_icon(path: &'static str, size: f32, color: Hsla) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(size))
        .flex_none()
        .text_color(color)
}

fn usage_provider_mark(provider: AgentProvider) -> Div {
    let mark = if provider == AgentProvider::Antigravity {
        img("logos/antigravity-color.svg")
            .size(px(20.))
            .flex_none()
            .into_any_element()
    } else {
        let color = if provider == AgentProvider::Claude {
            theme::lychee_400()
        } else {
            theme::rambutan_100()
        };
        usage_icon(crate::assets::provider_logo_path(provider), 16., color).into_any_element()
    };
    div()
        .size(px(20.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .child(mark)
}

fn reset_label(reset: i64, now: i64) -> String {
    let minutes = reset.saturating_sub(now) / 60;
    match minutes {
        ..1 => "now".into(),
        1..60 => format!("in {minutes}m"),
        60..1440 => format!("in {}h {}m", minutes / 60, minutes % 60),
        _ => format!("in {}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}
