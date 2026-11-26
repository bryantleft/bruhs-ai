use super::*;
use bruhs_models::{
    agent_extensions::{UsageHistoryBreakdown, UsageSpeed, UsageTotal},
    logic::context_meter::format_tokens,
    num::{ToFloatLossy as _, ToIntSaturating as _},
};
use bruhs_ui::line_chart::line_swatch;
use chrono::{DateTime, Days, Utc};
use std::collections::BTreeMap;

mod chart;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Breakdown {
    #[default]
    Model,
    Day,
}

impl Breakdown {
    fn label(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Day => "Day",
        }
    }
}

#[derive(Default)]
struct PeriodTotal {
    turns: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    unpriced_buckets: usize,
    priced_buckets: usize,
    priced_usd: f64,
}

impl PeriodTotal {
    fn add(&mut self, day: &UsageTotal) {
        self.turns = self.turns.saturating_add(day.turns);
        self.input = self.input.saturating_add(day.input);
        self.output = self.output.saturating_add(day.output);
        self.cache_read = self.cache_read.saturating_add(day.cache_read);
        self.cache_write = self.cache_write.saturating_add(day.cache_write);
        if day.unpriced_turns > 0 {
            self.unpriced_buckets += 1;
        }
        if let Some(cost) = day.priced_subtotal() {
            self.priced_usd += cost;
            self.priced_buckets += 1;
        }
    }

    fn tokens(&self) -> u64 {
        tokens(self.input, self.output, self.cache_read, self.cache_write)
    }
}

fn tokens(input: u64, output: u64, read: u64, write: u64) -> u64 {
    input
        .saturating_add(output)
        .saturating_add(read)
        .saturating_add(write)
}

fn processed(row: &UsageTotal) -> u64 {
    tokens(row.input, row.output, row.cache_read, row.cache_write)
}

struct Bucket<'a> {
    label: String,
    axis_label: String,
    total: Option<&'a UsageTotal>,
    providers: Option<&'a BTreeMap<String, UsageTotal>>,
}

impl Bucket<'_> {
    fn provider(&self, provider: AgentProvider) -> Option<&UsageTotal> {
        self.providers?.get(provider.as_str())
    }
}

fn timeline(
    snapshot: &SharedUsageHistory,
    period: Period,
    hourly_end: Option<i64>,
) -> (Vec<Bucket<'_>>, String) {
    if let Some(end) = hourly_end {
        let start = end - 86_400;
        let first_hour = start.div_euclid(3600) * 3600;
        let buckets = (first_hour..end)
            .step_by(3600)
            .filter_map(|hour| {
                let date = DateTime::from_timestamp(hour, 0)?;
                Some(Bucket {
                    label: date.format("%b %-d %H:%M").to_string(),
                    axis_label: date.format("%H:%M").to_string(),
                    total: snapshot.by_hour.get(&hour),
                    providers: snapshot.breakdown.by_hour_provider.get(&hour),
                })
            })
            .collect();
        return (buckets, "Rolling 24 hours · UTC".into());
    }
    let Period::Days(days) = period else {
        return (vec![], String::new());
    };
    let today = Utc::now().date_naive();
    let buckets: Vec<_> = (0..u64::from(days))
        .rev()
        .filter_map(|offset| today.checked_sub_days(Days::new(offset)))
        .map(|day| {
            let key = day.to_string();
            let label = day.format("%b %-d").to_string();
            Bucket {
                axis_label: label.clone(),
                label,
                total: snapshot.by_day.get(&key),
                providers: snapshot.breakdown.by_day_provider.get(&key),
            }
        })
        .collect();
    let label = buckets.first().map_or_else(String::new, |first| {
        format!("{} – {} · UTC", first.label, today.format("%b %-d"))
    });
    (buckets, label)
}

fn activity_label(turns: u64, sessions: u64, detailed: bool) -> String {
    match (sessions, detailed) {
        (0, true) => format!(
            "{turns} measured {}",
            if turns == 1 { "turn" } else { "turns" }
        ),
        (0, false) => format!("{turns} {}", if turns == 1 { "turn" } else { "turns" }),
        (_, true) => format!(
            "{sessions} recorded {}",
            if sessions == 1 { "session" } else { "sessions" }
        ),
        (_, false) => format!(
            "{sessions} {}",
            if sessions == 1 { "session" } else { "sessions" }
        ),
    }
}

fn provider_summaries(
    timeline: &[Bucket<'_>],
    providers: &[AgentProvider],
    sessions: &BTreeMap<String, u64>,
    estimated_cost: Option<f64>,
) -> impl IntoElement {
    let cost = estimated_cost.is_some();
    let mut totals: Vec<_> = providers
        .iter()
        .map(|&provider| {
            let mut total = PeriodTotal::default();
            for row in timeline
                .iter()
                .filter_map(|bucket| bucket.provider(provider))
            {
                total.add(row);
            }
            (provider, total)
        })
        .collect();
    if cost {
        totals.sort_by(|(_, a), (_, b)| b.priced_usd.total_cmp(&a.priced_usd));
    } else {
        totals.sort_by_key(|(_, total)| std::cmp::Reverse(total.tokens()));
    }
    div()
        .id("usage-provider-summaries")
        .debug_selector(|| "usage-provider-summaries".into())
        .flex()
        .flex_col()
        .gap_1()
        .children(totals.into_iter().map(|(provider, total)| {
            let amount = if !cost {
                format_tokens(total.tokens())
            } else if total.priced_buckets == 0 {
                "Unpriced".into()
            } else {
                dollars(total.priced_usd)
            };
            let partial = cost && total.unpriced_buckets > 0 && total.priced_buckets > 0;
            let activity = activity_label(
                total.turns,
                sessions.get(provider.as_str()).copied().unwrap_or_default(),
                false,
            );
            let share = estimated_cost
                .filter(|estimate| *estimate > 0. && total.priced_buckets > 0)
                .map(|estimate| format!("{:.1}% cost", 100. * (total.priced_usd / estimate)));
            let tokens = cost.then(|| format!("{} tokens", format_tokens(total.tokens())));
            let metadata = |value| muted(value).text_size(px(11.)).line_height(px(15.));
            let amount_label = if partial {
                format!("{amount}, priced turns only")
            } else {
                amount.clone()
            };
            div()
                .flex()
                .flex_col()
                .pt_1()
                .border_t_1()
                .border_color(theme::longan_800())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(line_swatch(chart::provider_color(provider)))
                        .child(provider_mark(provider))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .child(muted(provider.label()).truncate()),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!(
                                    "usage-provider-cost-{}",
                                    provider.as_str()
                                )))
                                .role(Role::Label)
                                .aria_label(amount_label.clone())
                                .text_size(px(12.))
                                .font_features(theme::tabular_nums())
                                .flex_none()
                                .when(partial, |el| {
                                    el.tooltip(move |window, cx| {
                                        bruhs_ui::tooltip::Tooltip::new(amount_label.clone())
                                            .build(window, cx)
                                    })
                                })
                                .child(if partial {
                                    format!("≥{amount}")
                                } else {
                                    amount
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x_2()
                        .child(metadata(activity))
                        .children(share.map(metadata))
                        .children(tokens.map(metadata)),
                )
        }))
}

impl UsageWindow {
    pub(super) fn render_history(
        &mut self,
        narrow: bool,
        window: &Window,
        cx: &Context<'_, Self>,
    ) -> Div {
        let cost = self.view == View::Cost;
        let content = div()
            .flex()
            .flex_col()
            .gap_5()
            .children(self.history.notice().map(|notice| muted(notice.to_owned())));
        let Some(data) = &self.history.data else {
            return content;
        };
        let content = content.children(data.notices.iter().cloned().map(muted));
        if !data.has_source {
            return content;
        }
        let snapshot = &data.report;

        let hourly = data.last_24h_ending_at.is_some();
        let (timeline, period_label) = timeline(snapshot, self.period, data.last_24h_ending_at);
        let providers: Vec<_> = AgentProvider::ALL
            .into_iter()
            .filter(|provider| {
                timeline
                    .iter()
                    .any(|bucket| bucket.provider(*provider).is_some())
            })
            .collect();
        let mut total = PeriodTotal::default();
        for bucket in &timeline {
            if let Some(row) = bucket.total {
                total.add(row);
            }
        }
        let (value, amount) = if total.turns == 0 {
            (None, "—".into())
        } else if !cost {
            (
                Some(total.tokens().to_f64_lossy()),
                format_tokens(total.tokens()),
            )
        } else if snapshot.breakdown.cost_by_speed.is_empty() {
            (None, "Unpriced".into())
        } else {
            let estimate = snapshot.breakdown.cost_by_type.total();
            (Some(estimate), dollars(estimate))
        };
        let amount = self.history_total.render(
            "usage-history-total",
            value,
            amount,
            |value| {
                if cost {
                    dollars(value)
                } else {
                    format_tokens(value.round().to_u64_saturating())
                }
            },
            window,
            cx,
        );
        let mut summary = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(title(if cost {
                "Estimated API cost"
            } else {
                "Processed tokens"
            }))
            .child(figure(amount))
            .child(muted(activity_label(
                total.turns,
                snapshot
                    .breakdown
                    .sessions_by_provider
                    .values()
                    .copied()
                    .fold(0, u64::saturating_add),
                true,
            )))
            .child(muted(period_label));
        if cost && total.unpriced_buckets > 0 {
            summary = summary.child(muted("Some turns lack prices"));
        }
        let providers_summary = provider_summaries(
            &timeline,
            &providers,
            &snapshot.breakdown.sessions_by_provider,
            cost.then(|| snapshot.breakdown.cost_by_type.total()),
        );
        let chart = chart::render(&timeline, &providers, cost, hourly, narrow);
        let overview = if narrow {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(summary)
                .child(chart)
                .child(providers_summary)
        } else {
            div()
                .flex()
                .gap_4()
                .child(
                    div()
                        .w(px(244.))
                        .flex_none()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(summary)
                        .child(providers_summary),
                )
                .child(chart)
        };
        content
            .child(overview)
            .when(cost && total.turns > 0, |el| {
                el.child(cost_totals(&total, &snapshot.breakdown))
                    .when(!snapshot.breakdown.cli_costs.is_empty(), |el| {
                        el.child(cli_reported_costs(&snapshot.breakdown))
                    })
                    .child(cost_breakdowns(&snapshot.breakdown, narrow))
            })
            .when(!cost && total.turns > 0, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_4()
                        .py_3()
                        .border_t_1()
                        .border_b_1()
                        .border_color(theme::longan_700())
                        .children(
                            [
                                ("Uncached input", total.input),
                                ("Output", total.output),
                                ("Cache reads", total.cache_read),
                                ("Cache writes", total.cache_write),
                            ]
                            .map(|(label, value)| {
                                div()
                                    .flex_1()
                                    .min_w(px(116.))
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(muted(label))
                                    .child(title(format_tokens(value)))
                            }),
                        ),
                )
            })
            .child(self.render_breakdown(snapshot, &timeline, &providers, cost, narrow, cx))
            .when(data.imported_records != 0, |el| {
                el.child(muted(format!(
                    "{} CLI history records are tracked separately.",
                    data.imported_records
                )))
            })
    }

    fn render_breakdown(
        &self,
        snapshot: &SharedUsageHistory,
        timeline: &[Bucket<'_>],
        providers: &[AgentProvider],
        cost: bool,
        narrow: bool,
        cx: &Context<'_, Self>,
    ) -> Div {
        let columns = self.breakdown;
        let section = div().flex().flex_col().gap_2().child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(title("Breakdown"))
                .child(
                    Self::control_group().children([Breakdown::Model, Breakdown::Day].map(
                        |choice| {
                            let selected = columns == choice;
                            let label =
                                if choice == Breakdown::Day && self.period == Period::Last24h {
                                    "Hour"
                                } else {
                                    choice.label()
                                };
                            Self::button(
                                SharedString::from(format!("usage-breakdown-{label}")),
                                label,
                                WebButtonVariant::Ghost,
                                false,
                                cx,
                                move |view, _, cx| {
                                    view.breakdown = choice;
                                    cx.notify();
                                },
                            )
                            .aria_selected(selected)
                            .when(selected, |button| button.bg(theme::longan_700()))
                            .debug_selector(move || format!("usage-breakdown-{label}"))
                        },
                    )),
                ),
        );
        let models = columns == Breakdown::Model;
        if !models {
            return section.child(time_breakdown(
                timeline,
                providers,
                self.period == Period::Last24h,
                cost,
                narrow,
            ));
        }
        let mut rows: Vec<_> = snapshot.by_model.iter().collect();
        if cost {
            rows.sort_by(|(_, a), (_, b)| {
                b.estimated_usd
                    .unwrap_or(-1.)
                    .total_cmp(&a.estimated_usd.unwrap_or(-1.))
            });
        } else {
            rows.sort_by_key(|(_, row)| std::cmp::Reverse(processed(row)));
        }
        if rows.is_empty() {
            return section.child(muted("No measured turns yet."));
        }
        section
            .child(div().text_color(theme::rambutan_400()).child(breakdown_row(
                columns.label(),
                None,
                "Turns",
                "Tokens",
                muted("Estimated cost").truncate(),
            )))
            .children(rows.into_iter().map(|(key, row)| {
                let locally_unpriced = self
                    .history
                    .data
                    .as_ref()
                    .is_some_and(|data| data.local_unpriced_models.contains(key));
                let model = key.clone();
                let cost = if locally_unpriced {
                    Self::button(
                        SharedString::from(format!("usage-set-price-{model}")),
                        "Set price",
                        WebButtonVariant::Ghost,
                        false,
                        cx,
                        move |this, window, cx| {
                            this.open_model_prices(Some(model.clone()), window, cx)
                        },
                    )
                    .aria_label(format!("Set price for {key} on this computer"))
                    .into_any_element()
                } else {
                    muted(row_cost(row)).truncate().into_any_element()
                };
                breakdown_row(
                    bruhs_models::agent::model_label(key),
                    Some(AgentProvider::for_model(key)),
                    row.turns.to_string(),
                    format_tokens(processed(row)),
                    cost,
                )
            }))
    }
}

fn cli_reported_costs(breakdown: &UsageHistoryBreakdown) -> impl IntoElement {
    div()
        .id("usage-cli-reported-cost")
        .debug_selector(|| "usage-cli-reported-cost".into())
        .flex()
        .flex_col()
        .gap_2()
        .border_t_1()
        .border_color(theme::longan_800())
        .pt_4()
        .child(title("CLI-reported cost"))
        .child(muted(
            "API-equivalent amounts. Subscription billing is separate.",
        ))
        .children(AgentProvider::ALL.into_iter().filter_map(|provider| {
            let cost = breakdown.cli_costs.get(provider.as_str())?;
            Some(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(provider_mark(provider))
                    .child(muted(format!(
                        "{} · {} {}",
                        provider.label(),
                        cost.runs,
                        if cost.runs == 1 { "run" } else { "runs" }
                    )))
                    .child(title(dollars(cost.usd)).font_features(theme::tabular_nums())),
            )
        }))
}

fn cost_totals(total: &PeriodTotal, breakdown: &UsageHistoryBreakdown) -> Div {
    let cached = total.cache_read.saturating_add(total.cache_write);
    let savings = &breakdown.cache_savings;
    let cache_estimate = if cached == 0 {
        dollars(0.)
    } else if savings.priced_tokens == 0 {
        "Unavailable".into()
    } else {
        dollars(savings.estimated_usd)
    };
    let partial = savings.priced_tokens > 0 && savings.priced_tokens < cached;
    div()
        .flex()
        .flex_col()
        .gap_2()
        .border_t_1()
        .border_color(theme::longan_800())
        .pt_4()
        .child(title("Totals"))
        .child(
            div().flex().flex_wrap().gap_4().children(
                [
                    ("Processed tokens", format_tokens(total.tokens())),
                    ("Cached input", format_tokens(cached)),
                    ("Uncached input", format_tokens(total.input)),
                    ("Output", format_tokens(total.output)),
                    ("Est. cache savings", cache_estimate),
                ]
                .map(|(label, value)| {
                    div()
                        .flex_1()
                        .min_w(px(116.))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(muted(label))
                        .child(title(value).font_features(theme::tabular_nums()))
                        .when(partial && label == "Est. cache savings", |el| {
                            el.child(muted("Priced cache tokens only"))
                        })
                }),
            ),
        )
}

fn cost_breakdowns(breakdown: &UsageHistoryBreakdown, narrow: bool) -> impl IntoElement {
    let fast = &breakdown.fast_pricing;
    let premium = (fast.tier_priced_turns > 0).then(|| {
        div()
            .id("usage-fast-premium")
            .debug_selector(|| "usage-fast-premium".into())
            .flex()
            .items_center()
            .gap_2()
            .child(muted("Est. fast premium"))
            .child(title(dollars(fast.premium_usd)).font_features(theme::tabular_nums()))
            .into_any_element()
    });
    let costs = &breakdown.cost_by_type;
    let speed = &breakdown.cost_by_speed;
    let by_type = [
        ("Input", costs.input, theme::blueberry_400()),
        ("Output", costs.output, theme::pandan_400()),
        ("Cache reads", costs.cache_read, theme::mangosteen_400()),
        ("Cache writes", costs.cache_write, theme::durian_400()),
    ];
    let by_speed = [
        (UsageSpeed::Standard, "Standard", theme::blueberry_400()),
        (UsageSpeed::Fast, "Fast", theme::pandan_400()),
        (UsageSpeed::Unknown, "Unknown", theme::rambutan_400()),
    ]
    .into_iter()
    .filter_map(|(key, label, color)| speed.get(&key).map(|&cost| (label, cost, color)));
    div()
        .id("usage-cost-breakdowns")
        .debug_selector(|| "usage-cost-breakdowns".into())
        .flex()
        .gap_5()
        .when(narrow, |el| el.flex_col())
        .border_t_1()
        .border_color(theme::longan_800())
        .pt_4()
        .child(cost_bar(
            "Cost by type",
            (!speed.is_empty()).then_some(by_type).into_iter().flatten(),
            None,
        ))
        .child(
            cost_bar("Cost by requested speed", by_speed, premium)
                .children(fast_pricing_details(breakdown)),
        )
}

fn fast_pricing_details(breakdown: &UsageHistoryBreakdown) -> Vec<Div> {
    let fast = &breakdown.fast_pricing;
    let mut details = Vec::new();
    if fast.standard_fallback_turns > 0 {
        details.push(muted(format!(
            "Standard rates used for {} fast {}",
            fast.standard_fallback_turns,
            if fast.standard_fallback_turns == 1 {
                "turn"
            } else {
                "turns"
            }
        )));
    }
    if fast.unpriced_turns > 0 {
        details.push(muted(format!(
            "{} fast {} unpriced",
            fast.unpriced_turns,
            if fast.unpriced_turns == 1 {
                "turn"
            } else {
                "turns"
            }
        )));
    }
    if fast.includes_legacy_report {
        details.push(muted("Some linked reports lack fast pricing details."));
    }
    details
}

fn cost_bar(
    label: &'static str,
    parts: impl IntoIterator<Item = (&'static str, f64, Hsla)>,
    header: Option<AnyElement>,
) -> Div {
    let parts: Vec<_> = parts.into_iter().collect();
    let total = parts.iter().map(|(_, value, _)| value).sum::<f64>();
    let legend = div()
        .flex()
        .flex_wrap()
        .gap_x_3()
        .gap_y_1()
        .children(parts.iter().map(|&(name, cost, color)| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(line_swatch(color))
                .child(muted(name))
                .child(div().text_size(px(12.)).child(dollars(cost)))
        }));
    div()
        .min_w_0()
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap_2()
                .child(title(label))
                .children(header),
        )
        .when(parts.is_empty(), |el| el.child(muted("No priced turns.")))
        .when(!parts.is_empty(), |el| {
            el.child(
                div()
                    .h(px(6.))
                    .flex()
                    .overflow_hidden()
                    .rounded_full()
                    .bg(theme::longan_800())
                    .children(
                        parts
                            .into_iter()
                            .filter(|(_, cost, _)| *cost > 0. && total.is_finite())
                            .map(|(_, cost, color)| {
                                div()
                                    .h_full()
                                    .flex_none()
                                    .w(relative((cost / total).to_f32_lossy()))
                                    .bg(color)
                            }),
                    ),
            )
            .child(legend)
        })
}

fn row_cost(row: &UsageTotal) -> String {
    row.estimated_usd.map_or_else(|| "Unpriced".into(), dollars)
}

fn provider_mark(provider: AgentProvider) -> Svg {
    svg()
        .path(crate::assets::provider_logo_path(provider))
        .size(px(12.))
        .flex_none()
        .text_color(theme::rambutan_200())
}

fn time_breakdown(
    timeline: &[Bucket<'_>],
    providers: &[AgentProvider],
    hourly: bool,
    cost: bool,
    narrow: bool,
) -> Div {
    if !timeline.iter().any(|bucket| bucket.total.is_some()) {
        return muted("No measured turns yet.");
    }
    let row = || {
        div()
            .flex()
            .items_center()
            .gap_2()
            .py_2()
            .border_b_1()
            .border_color(theme::longan_800())
    };
    let cell = |label: String| {
        div()
            .min_w_0()
            .flex_1()
            .text_right()
            .child(muted(label).truncate())
    };
    let date_cell = |label: String| {
        div()
            .w(px(if hourly { 118. } else { 82. }))
            .flex_none()
            .text_size(px(12.))
            .child(label)
    };
    let value = |total: &UsageTotal| {
        if cost {
            row_cost(total)
        } else {
            format_tokens(processed(total))
        }
    };
    let totals =
        |row: Div, total: String, secondary: String| row.child(cell(total)).child(cell(secondary));
    let table = div().flex().flex_col();
    let header = row()
        .text_color(theme::rambutan_400())
        .child(date_cell(if hourly { "Hour · UTC" } else { "Day" }.into()))
        .when(!narrow, |el| {
            el.children(providers.iter().map(|&provider| {
                div()
                    .id(SharedString::from(format!(
                        "usage-day-provider-{}",
                        provider.as_str()
                    )))
                    .debug_selector(move || format!("usage-day-provider-{}", provider.as_str()))
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .justify_end()
                    .items_center()
                    .gap_1()
                    .aria_label(provider.label())
                    .tooltip(move |window, cx| {
                        bruhs_ui::tooltip::Tooltip::new(provider.label()).build(window, cx)
                    })
                    .child(provider_mark(provider))
                    .child(muted(provider.label()).truncate())
            }))
        });
    table
        .child(totals(
            header,
            "Total".into(),
            if cost { "Tokens" } else { "Cost" }.into(),
        ))
        .children(timeline.iter().rev().filter_map(|bucket| {
            let total = bucket.total?;
            let date = bucket.label.clone();
            let primary = row().child(date_cell(date)).when(!narrow, |el| {
                el.children(providers.iter().map(|provider| {
                    cell(bucket.provider(*provider).map_or_else(|| "—".into(), value))
                }))
            });
            let primary = totals(
                primary,
                value(total),
                if cost {
                    format_tokens(processed(total))
                } else {
                    row_cost(total)
                },
            );
            Some(div().flex().flex_col().child(primary).when(narrow, |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_x_4()
                        .gap_y_1()
                        .py_2()
                        .children(providers.iter().filter_map(|&provider| {
                            let entry = bucket.provider(provider)?;
                            Some(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(provider_mark(provider))
                                    .child(muted(provider.label()))
                                    .child(muted(value(entry))),
                            )
                        })),
                )
            }))
        }))
}

fn breakdown_row(
    name: impl Into<SharedString>,
    provider: Option<AgentProvider>,
    turns: impl Into<SharedString>,
    tokens: impl Into<SharedString>,
    cost: impl IntoElement,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .py_2()
        .border_b_1()
        .border_color(theme::longan_800())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap_2()
                .when_some(provider, |el, provider| {
                    el.child(
                        svg()
                            .path(crate::assets::provider_logo_path(provider))
                            .size(px(14.))
                            .flex_none()
                            .text_color(theme::rambutan_200()),
                    )
                })
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .child(name.into()),
                ),
        )
        .children(
            [(0.15, turns.into()), (0.20, tokens.into())].map(|(width, value)| {
                div()
                    .w(relative(width))
                    .min_w_0()
                    .flex_shrink_0()
                    .text_right()
                    .child(muted(value).truncate())
            }),
        )
        .child(
            div()
                .w(relative(0.25))
                .min_w_0()
                .flex_none()
                .flex()
                .justify_end()
                .child(cost),
        )
}

fn dollars(amount: f64) -> String {
    let sign = if amount < 0. { "-" } else { "" };
    let amount = amount.abs();
    if amount > 0. && amount < 0.01 {
        format!("{sign}${amount:.4}")
    } else {
        format!("{sign}${amount:.2}")
    }
}
