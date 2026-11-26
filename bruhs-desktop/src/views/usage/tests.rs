use super::*;
use ::core::prelude::v1::test;
use bruhs_models::agent_extensions::{
    QuotaWindow, UsageEstimate, UsageHistoryBreakdown, UsageLimitAccount, UsageSpeed, UsageTotal,
};

fn account(name: &str, plan: &str, used: f64, reset: i64) -> UsageLimitAccount {
    UsageLimitAccount {
        provider: "Codex".into(),
        label: name.into(),
        email: None,
        workspace: None,
        on_device: true,
        via: vec![],
        zero_spend_cap: false,
        plan: Some(plan.into()),
        sources: vec!["This computer".into()],
        windows: vec![QuotaWindow {
            window_id: None,
            provider: "codex".into(),
            account: name.into(),
            name: "primary".into(),
            used_percent: used,
            duration_minutes: Some(300),
            resets_at: Some(reset),
            observed_at: reset - 300,
        }],
        credits: 0,
        reset: None,
    }
}

#[test]
fn pooled_limits_weight_known_plans_and_align_refills_by_account() {
    let a = account("First", "plus", 25., 1000);
    let b = account("Second", "pro", 75., 900);
    let (order, rows) = pools::collect("Codex", &[&a, &b], 800);
    assert_eq!(order, [1, 0]);
    assert_eq!(rows[0].1.left.round().to_string(), "27");
    assert_eq!(
        rows[0]
            .1
            .refill
            .map(|(time, gain)| (time, gain.to_string())),
        Some((900, "71".into()))
    );
}

#[gpui::test]
fn usage_shortcuts_only_change_periods_on_history_views(cx: &mut TestAppContext) {
    let usage = cx.new(UsageWindow::new);
    usage.update(cx, |view, cx| {
        let initial_period = view.period;
        let day = || UsageShortcut::from_command("usage.period.day").expect("Usage command");
        view.apply_shortcut(day(), cx);
        assert!(view.period == initial_period);

        view.apply_shortcut(UsageShortcut::View(View::Cost), cx);
        assert_eq!(view.view, View::Cost);
        view.apply_shortcut(day(), cx);
        assert!(view.period == Period::Last24h);

        view.open_filter = Some(Filter::Provider);
        assert!(!view.apply_shortcut(UsageShortcut::View(View::Limits), cx));
        assert_eq!(view.view, View::Cost);
        assert!(view.open_filter == Some(Filter::Provider));
    });
}

#[gpui::test]
fn usage_switches_between_limits_tokens_and_cost(cx: &mut TestAppContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
    });
    let (usage, visual) = cx.add_window_view(|_, cx| {
        let mut view = UsageWindow::new(cx);
        let now = chrono::Utc::now();
        let day = now.date_naive().to_string();
        let turn = UsageTotal {
            turns: 1,
            input: 100,
            output: 25,
            estimated_usd: Some(0.4),
            ..UsageTotal::default()
        };
        view.providers = vec!["Codex".into(), "Claude".into()];
        view.limits.finish(Ok(UsageLimitSnapshot {
            environments: vec!["This computer".into()],
            providers: view.providers.clone(),
            accounts: vec![account("Fixture", "plus", 25., now.timestamp() + 3600)],
            notices: vec![],
            external: vec![],
        }));
        view.history.finish(Ok(HistoryData {
            has_source: true,
            report: SharedUsageHistory {
                by_day: [(day.clone(), turn.clone())].into(),
                breakdown: UsageHistoryBreakdown {
                    by_day_provider: [(day, [("codex".into(), turn)].into())].into(),
                    cost_by_type: UsageEstimate {
                        input: 0.15,
                        output: 0.25,
                        ..Default::default()
                    },
                    cost_by_speed: [(UsageSpeed::Unknown, 0.4)].into(),
                    cli_costs: [(
                        "claude".into(),
                        bruhs_models::agent_extensions::CliCost { runs: 1, usd: 0.25 },
                    )]
                    .into(),
                    ..UsageHistoryBreakdown::default()
                },
                ..SharedUsageHistory::default()
            },
            ..HistoryData::default()
        }));
        view
    });
    visual.simulate_resize(size(px(1024.), px(760.)));
    visual.run_until_parked();
    let segment = visual.debug_bounds("usage-segment-0").expect("quota rail");
    visual.simulate_click(segment.center(), Modifiers::default());
    assert!(usage.read_with(visual, |view, _| view.selection.is_some()));
    usage.update_in(visual, |view, window, cx| view.escape(window, cx));
    assert!(usage.read_with(visual, |view, _| view.selection.is_none()));
    for selector in ["usage-view-Tokens", "usage-view-Cost"] {
        let button = visual.debug_bounds(selector).expect("metric switch");
        visual.simulate_click(button.center(), Modifiers::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("usage-chart").is_some());
        assert_eq!(
            visual.debug_bounds("usage-cli-reported-cost").is_some(),
            selector == "usage-view-Cost"
        );
        assert!(visual.debug_bounds("usage-segment-0").is_none());
    }
    let by_day = visual
        .debug_bounds("usage-breakdown-Day")
        .expect("day breakdown");
    visual.simulate_click(by_day.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(usage.read_with(visual, |view, _| view.breakdown == history::Breakdown::Day));
    assert!(visual.debug_bounds("usage-cost-breakdowns").is_some());
    assert!(visual.debug_bounds("usage-day-provider-codex").is_some());
    let by_model = visual
        .debug_bounds("usage-breakdown-Model")
        .expect("model breakdown");
    visual.simulate_click(by_model.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(usage.read_with(visual, |view, _| view.breakdown
        == history::Breakdown::Model));
    visual.simulate_resize(size(px(460.), px(530.)));
    visual.run_until_parked();
    let chart = visual.debug_bounds("usage-chart").expect("compact chart");
    assert!(chart.right() <= px(460.));
    let summaries = visual
        .debug_bounds("usage-provider-summaries")
        .expect("compact provider summaries");
    assert!(chart.bottom() <= summaries.top());
    usage.update(visual, |view, cx| {
        let end = chrono::Utc::now().timestamp();
        let hour = (end - 1).div_euclid(3600) * 3600;
        let entry = UsageTotal {
            turns: 1,
            unpriced_turns: 1,
            input: 100,
            ..UsageTotal::default()
        };
        view.period = Period::Last24h;
        view.history.finish(Ok(HistoryData {
            last_24h_ending_at: Some(end),
            has_source: true,
            report: SharedUsageHistory {
                by_hour: [(hour, entry.clone())].into(),
                breakdown: UsageHistoryBreakdown {
                    by_hour_provider: [(hour, [("codex".into(), entry)].into())].into(),
                    ..UsageHistoryBreakdown::default()
                },
                ..SharedUsageHistory::default()
            },
            ..HistoryData::default()
        }));
        cx.notify();
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("usage-chart").is_some());
    assert!(visual.debug_bounds("usage-breakdown-Hour").is_some());
}
