use super::*;
use bruhs_ui::line_chart::{
    line_chart, LineChartInterval, LineChartPoint, LineChartSeries, LineChartStyle,
};

pub(super) fn provider_color(provider: AgentProvider) -> Hsla {
    match provider {
        AgentProvider::Claude => theme::lychee_400(),
        AgentProvider::Codex => theme::blueberry_400(),
        AgentProvider::Cursor => theme::rambutan_400(),
        AgentProvider::Antigravity => theme::mangosteen_400(),
        AgentProvider::Grok => theme::pandan_400(),
        AgentProvider::OpenCode => theme::durian_400(),
        AgentProvider::Acp => theme::dragonfruit_400(),
    }
}

pub(super) fn render(
    timeline: &[Bucket<'_>],
    providers: &[AgentProvider],
    cost: bool,
    hourly: bool,
    narrow: bool,
) -> Div {
    let heading = match (hourly, cost) {
        (false, true) => "Daily estimated cost",
        (false, false) => "Daily processed tokens",
        (true, true) => "Hourly estimated cost",
        (true, false) => "Hourly processed tokens",
    };
    let chart = div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_2()
        .when(!narrow, |el| el.flex_1())
        .child(title(heading));
    if providers.is_empty() {
        return chart.child(muted("No measured turns in this period."));
    }

    let intervals = timeline
        .iter()
        .map(|bucket| LineChartInterval {
            axis_label: bucket.axis_label.clone().into(),
            tooltip_title: format!("{} · UTC", bucket.label).into(),
        })
        .collect();
    let series: Vec<_> = providers
        .iter()
        .map(|&provider| LineChartSeries {
            label: provider.label().into(),
            color: provider_color(provider),
            points: timeline
                .iter()
                .map(|bucket| {
                    let row = bucket.provider(provider);
                    let (value, label) = match row {
                        Some(row) if cost => (row.estimated_usd, row_cost(row)),
                        Some(row) => {
                            let tokens = processed(row);
                            (Some(tokens.to_f64_lossy()), tokens.to_string())
                        }
                        None if cost => (Some(0.), dollars(0.)),
                        None => (Some(0.), "0".into()),
                    };
                    LineChartPoint {
                        value,
                        tooltip_value: label.into(),
                    }
                })
                .collect(),
        })
        .collect();
    let missing = cost
        && series
            .iter()
            .any(|series| series.points.iter().any(|point| point.value.is_none()));
    chart
        .child(line_chart(
            "usage-chart",
            format!("{heading} by provider. The breakdown lists the same intervals."),
            px(if narrow { 144. } else { 186. }),
            intervals,
            series,
            LineChartStyle {
                rounded: true,
                fill_opacity: 0.11,
            },
            |value| {
                if cost {
                    dollars(value)
                } else {
                    format_tokens(value.to_u64_saturating())
                }
            },
        ))
        .when(missing, |el| {
            el.child(muted("Gaps indicate missing model prices."))
        })
}
