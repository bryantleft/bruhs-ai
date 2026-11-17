//! Web parity for the Life page: deleting a transaction, the holdings list
//! and the sleep and recovery card, in the web's cards.

use super::super::performance_tests::fixture;
use super::super::{AppView, DesktopLifeTab, Workspace};
use bruhs_models::{FinanceTransaction, Holding, LifeContext, RecoveryLog, SleepLog};
use gpui::{Bounds, Entity, Pixels, VisualTestContext};
use std::sync::Arc;

fn bounds(cx: &mut VisualTestContext, selector: &str) -> Option<Bounds<Pixels>> {
    let selector: &'static str = Box::leak(selector.to_string().into_boxed_str());
    cx.debug_bounds(selector)
}

fn click(cx: &mut VisualTestContext, selector: &str) {
    let target = bounds(cx, selector).unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(target.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn open_life(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
    tab: DesktopLifeTab,
    context: LifeContext,
) {
    workspace.update(cx, |workspace, cx| {
        workspace.life_context = context;
        workspace.active_view = AppView::Life;
        workspace.life_tab = tab;
        cx.notify();
    });
    cx.run_until_parked();
}

fn spend(merchant: &str, at: i64) -> Arc<FinanceTransaction> {
    Arc::new(FinanceTransaction::manual(merchant, "Other", -100, at))
}

fn merchants(workspace: &Workspace) -> Vec<String> {
    workspace
        .life_context
        .finance
        .transactions
        .iter()
        .map(|transaction| transaction.merchant.clone())
        .collect()
}

/// PM-15: like the web, each listed transaction has a ×, Delete
/// transaction, that removes it and saves; the card lists the newest eight.
#[gpui::test]
fn transaction_rows_delete_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let mut context = LifeContext::default();
    for index in 0..9 {
        context
            .finance
            .transactions
            .push(spend(&format!("Shop {index}"), 10 - index));
    }
    open_life(&workspace, cx, DesktopLifeTab::Finance, context);
    assert!(bounds(cx, "desktop-life-transaction-7").is_some());
    assert!(
        bounds(cx, "desktop-life-transaction-8").is_none(),
        "the newest eight, as on the web"
    );
    let saves = workspace.read_with(cx, |workspace, _| workspace.persist_requests);
    click(cx, "desktop-life-delete-transaction-0");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(merchants(workspace)[0], "Shop 1");
        assert_eq!(workspace.life_context.finance.transactions.len(), 8);
        assert!(workspace.persist_requests > saves, "the delete is saved");
    });
    assert!(
        bounds(cx, "desktop-life-transaction-7").is_some(),
        "the ninth moves up"
    );
    assert!(bounds(cx, "desktop-life-transaction-8").is_none());
}

/// PM-16: like the web's Holdings card, every holding shows as a chip of
/// its symbol and value; the record cards sit two to a row on a wide window
/// and stack on a compact one.
#[gpui::test]
fn holdings_card_lists_every_holding_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let mut context = LifeContext::default();
    context.finance.holdings = vec![
        Holding::manual("vti", 1_250, 1),
        Holding::manual("bnd", 900, 2),
    ];
    open_life(&workspace, cx, DesktopLifeTab::Finance, context);
    for (width, height) in [(1280.0, 800.0), (640.0, 480.0)] {
        cx.simulate_resize(gpui::size(gpui::px(width), gpui::px(height)));
        cx.run_until_parked();
        let card = bounds(cx, "desktop-life-holdings").expect("the Holdings card");
        for index in 0..2 {
            let chip = bounds(cx, &format!("desktop-life-holding-{index}"))
                .unwrap_or_else(|| panic!("holding {index} at {width}px"));
            assert!(chip.left() >= card.left() && chip.right() <= card.right());
            assert!(chip.top() >= card.top() && chip.bottom() <= card.bottom());
        }
        assert!(bounds(cx, "desktop-life-holding-2").is_none());
        let accounts = bounds(cx, "desktop-life-accounts").unwrap();
        let transactions = bounds(cx, "desktop-life-transactions").unwrap();
        if width >= 1024.0 {
            assert_eq!(accounts.top(), transactions.top(), "two to a row");
            assert!(accounts.right() <= transactions.left());
        } else {
            assert!(accounts.bottom() <= transactions.top(), "one to a row");
        }
    }
    workspace.read_with(cx, |workspace, _| {
        let labels = workspace
            .life_context
            .finance
            .holdings
            .iter()
            .map(Holding::label)
            .collect::<Vec<_>>();
        assert_eq!(labels, ["VTI, $12.50 USD", "BND, $9.00 USD"]);
    });
}

/// PM-17: like the web's Sleep & recovery card, the Health tab counts the
/// sleep and recovery logs (the web shows counts, not a list), beside the
/// Workout and Habits cards and the health sources; logging adds to them.
#[gpui::test]
fn sleep_and_recovery_card_counts_logs_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let mut context = LifeContext::default();
    context.health.sleep = vec![SleepLog::manual(480, 80, 2), SleepLog::manual(420, 70, 1)];
    context.health.recovery = vec![RecoveryLog::manual(64, 2)];
    open_life(&workspace, cx, DesktopLifeTab::Health, context);
    let card = bounds(cx, "desktop-life-sleep").expect("the Sleep & recovery card");
    let summary = bounds(cx, "desktop-life-sleep-summary").expect("its log counts");
    assert!(summary.top() >= card.top() && summary.bottom() <= card.bottom());
    for selector in [
        "desktop-life-workouts",
        "desktop-life-habits",
        "desktop-life-sources",
    ] {
        assert!(bounds(cx, selector).is_some(), "{selector}");
    }
    workspace.update_in(cx, |workspace, window, cx| {
        assert_eq!(
            workspace.life_context.health.sleep_and_recovery_summary(),
            "2 sleep logs, 1 recovery log"
        );
        workspace
            .life_value_input
            .update(cx, |input, cx| input.set_value("70", window, cx));
        workspace.add_health_entry(super::HealthEntryKind::Recovery, window, cx);
        assert_eq!(
            workspace.life_context.health.sleep_and_recovery_summary(),
            "2 sleep logs, 2 recovery logs"
        );
    });
}
