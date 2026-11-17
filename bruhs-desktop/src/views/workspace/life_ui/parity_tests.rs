//! Web parity for the Life page: CSV import, deleting a transaction, the
//! holdings list and the sleep and recovery card, in the web's cards, and
//! each finance card's own form (PM-46).

use super::super::load_desktop_state_from_path;
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

/// PM-14: like the web's CSV import card, pasted rows become transactions
/// ahead of the older ones, the notice says how many, and the paste clears;
/// a paste without a valid row stays for correction. The import is saved.
#[gpui::test]
async fn csv_import_adds_rows_first_and_says_how_many_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let dir = std::env::temp_dir().join(format!("bruhs-life-csv-{}", uuid::Uuid::new_v4()));
    let path = dir.join("state.json");
    workspace.update(cx, |workspace, _| {
        workspace.persistence_test_path = Some(path.clone());
    });
    let mut context = LifeContext::default();
    context.finance.transactions.push(spend("Older", 1));
    open_life(&workspace, cx, DesktopLifeTab::Finance, context);
    assert!(bounds(cx, "desktop-life-csv").is_some());
    assert!(bounds(cx, "desktop-life-notice").is_none());

    let invalid = "merchant,category,amount\nBad,row,nope";
    workspace.update_in(cx, |workspace, window, cx| {
        workspace
            .life_csv_input
            .update(cx, |input, cx| input.set_value(invalid, window, cx));
    });
    click(cx, "desktop-life-import-csv");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.life_notice.as_deref(),
            Some("No valid CSV rows found.")
        );
        assert_eq!(merchants(workspace), ["Older"]);
        assert_eq!(
            workspace.life_csv_input.read(cx).value().as_str(),
            invalid,
            "the paste stays for correction"
        );
    });
    assert!(bounds(cx, "desktop-life-notice").is_some());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.life_csv_input.update(cx, |input, cx| {
            input.set_value(
                "Coffee, Food, -5.25, client meeting\nRent, Home, -1200",
                window,
                cx,
            );
        });
    });
    click(cx, "desktop-life-import-csv");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.life_notice.as_deref(),
            Some("Imported 2 transactions locally.")
        );
        assert_eq!(merchants(workspace), ["Coffee", "Rent", "Older"]);
        let coffee = &workspace.life_context.finance.transactions[0];
        assert_eq!(coffee.amount_minor, -525);
        assert_eq!(coffee.category, "Food");
        assert_eq!(coffee.note.as_deref(), Some("client meeting"));
        assert!(workspace.life_csv_input.read(cx).value().is_empty());
    });
    assert!(
        bounds(cx, "desktop-life-transaction-2").is_some(),
        "the Transactions card lists the imported rows"
    );
    cx.condition(&workspace, |workspace, _| workspace.save_status.is_saved())
        .await;
    let saved = load_desktop_state_from_path(&path)
        .unwrap()
        .unwrap()
        .life_context
        .finance;
    assert_eq!(saved.transactions.len(), 3);
    assert_eq!(saved.transactions[1].amount_minor, -120_000);

    click(cx, "desktop-life-notice-dismiss");
    assert!(bounds(cx, "desktop-life-notice").is_none());
    std::fs::remove_dir_all(dir).unwrap();
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
    assert!(
        bounds(cx, "desktop-life-csv").is_none(),
        "CSV import is a finance card"
    );
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

fn set(
    cx: &mut VisualTestContext,
    workspace: &Entity<Workspace>,
    pick: fn(&super::FinanceForms) -> &gpui::Entity<gpui_component::input::InputState>,
    value: &str,
) {
    workspace.update_in(cx, |workspace, window, cx| {
        pick(&workspace.life_finance).update(cx, |input, cx| input.set_value(value, window, cx));
    });
}

/// PM-46: like the web's Finance tab, each card holds its own form (Accounts:
/// Name, Balance, Add; Transactions: Merchant, Spend amount, Category, Add
/// spend; Budgets & income: a Budget and an Income row; Holdings: Symbol,
/// Value, Add) instead of one Quick add form, the notes use the web's words,
/// and Amounts redacted stays disabled while agent access is off.
#[gpui::test]
fn finance_cards_hold_their_own_forms_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    open_life(
        &workspace,
        cx,
        DesktopLifeTab::Finance,
        LifeContext::default(),
    );
    assert!(
        bounds(cx, "desktop-life-quick-add").is_none(),
        "no Quick add form on Finance"
    );
    assert!(bounds(cx, "desktop-life-usd-note").is_some());
    for (card, fields, button) in [
        (
            "desktop-life-accounts",
            &["Account name", "Account balance"][..],
            "desktop-life-add-account",
        ),
        (
            "desktop-life-transactions",
            &[
                "Transaction merchant",
                "Transaction spend amount",
                "Transaction category",
            ][..],
            "desktop-life-add-spend",
        ),
        (
            "desktop-life-budgets",
            &[
                "Budget category",
                "Monthly budget limit",
                "Income stream name",
                "Expected monthly income",
            ][..],
            "desktop-life-add-budget",
        ),
        (
            "desktop-life-holdings",
            &["Holding symbol", "Holding current value"][..],
            "desktop-life-add-holding",
        ),
    ] {
        let card_bounds = bounds(cx, card).unwrap_or_else(|| panic!("{card}"));
        for field in fields {
            let field_bounds = bounds(cx, &format!("desktop-life-field-{field}"))
                .unwrap_or_else(|| panic!("{field}"));
            assert!(
                card_bounds.contains(&field_bounds.center()),
                "{field} in {card}"
            );
        }
        let button_bounds = bounds(cx, button).unwrap_or_else(|| panic!("{button}"));
        assert!(
            card_bounds.contains(&button_bounds.center()),
            "{button} in {card}"
        );
    }
    // Web `grid-cols-[minmax(0,1fr)_8rem_auto]`: Name, Balance, then Add.
    let name = bounds(cx, "desktop-life-field-Account name").unwrap();
    let balance = bounds(cx, "desktop-life-field-Account balance").unwrap();
    let add = bounds(cx, "desktop-life-add-account").unwrap();
    assert!(name.right() < balance.left() && balance.right() < add.left());
    assert_eq!(balance.size.width, gpui::px(128.0));
    assert_eq!(
        add.bottom(),
        balance.bottom(),
        "Add lines up with the fields"
    );
    // Transactions: Category under Merchant and Spend amount.
    let merchant = bounds(cx, "desktop-life-field-Transaction merchant").unwrap();
    let category = bounds(cx, "desktop-life-field-Transaction category").unwrap();
    assert_eq!(category.left(), merchant.left());
    assert!(category.top() > merchant.bottom());
    assert!(category.size.width > merchant.size.width * 1.5);

    set(cx, &workspace, |forms| &forms.account_name, "Checking");
    set(cx, &workspace, |forms| &forms.account_balance, "2,500.00");
    click(cx, "desktop-life-add-account");
    set(cx, &workspace, |forms| &forms.merchant, "Groceries");
    set(cx, &workspace, |forms| &forms.spend_amount, "42.00");
    click(cx, "desktop-life-add-spend");
    set(cx, &workspace, |forms| &forms.budget_category, "Food");
    set(cx, &workspace, |forms| &forms.budget_limit, "500");
    click(cx, "desktop-life-add-budget");
    set(cx, &workspace, |forms| &forms.income_name, "Salary");
    set(cx, &workspace, |forms| &forms.income_amount, "5000");
    click(cx, "desktop-life-add-income");
    set(cx, &workspace, |forms| &forms.holding_symbol, "vti");
    set(cx, &workspace, |forms| &forms.holding_value, "12.5");
    click(cx, "desktop-life-add-holding");
    workspace.read_with(cx, |workspace, cx| {
        let finance = &workspace.life_context.finance;
        assert_eq!(finance.accounts[0].name, "Checking");
        assert_eq!(finance.accounts[0].balance_minor, 250_000);
        assert_eq!(finance.transactions[0].merchant, "Groceries");
        assert_eq!(finance.transactions[0].amount_minor, -4_200);
        assert_eq!(finance.transactions[0].category, "Other");
        assert_eq!(finance.budgets[0].monthly_limit_minor, 50_000);
        assert_eq!(finance.income_streams[0].expected_monthly_minor, 500_000);
        assert_eq!(finance.holdings[0].label(), "VTI, $12.50 USD");
        assert!(workspace.life_notice.is_none());
        assert!(workspace
            .life_finance
            .account_name
            .read(cx)
            .value()
            .is_empty());
    });
    assert!(bounds(cx, "desktop-life-holding-0").is_some());
    // Each row's × is drawn, and clicked, inside its row: a wrapping row
    // once put it over the next card's Add.
    for (row, delete) in [
        ("desktop-life-account-0", "desktop-life-delete-account-0"),
        (
            "desktop-life-transaction-0",
            "desktop-life-delete-transaction-0",
        ),
    ] {
        let row_bounds = bounds(cx, row).unwrap_or_else(|| panic!("{row}"));
        let delete_bounds = bounds(cx, delete).unwrap_or_else(|| panic!("{delete}"));
        assert!(
            row_bounds.contains(&delete_bounds.center()),
            "{delete} in {row}"
        );
    }

    // A blank amount is refused in the page notice; the fields keep it.
    set(cx, &workspace, |forms| &forms.merchant, "Rent");
    click(cx, "desktop-life-add-spend");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.life_notice.as_deref(),
            Some(bruhs_models::MoneyError::InvalidAmount.to_string().as_str())
        );
        assert_eq!(workspace.life_context.finance.transactions.len(), 1);
        assert_eq!(
            workspace.life_finance.merchant.read(cx).value().as_str(),
            "Rent"
        );
    });
    assert!(bounds(cx, "desktop-life-notice").is_some());

    // The privacy toggles sit together; Amounts redacted waits for access.
    let access = bounds(cx, "desktop-life-agent-access").unwrap();
    let exact = bounds(cx, "desktop-life-exact-amounts").unwrap();
    assert_eq!(access.top(), exact.top());
    assert!(
        access.size.width > gpui::px(100.0) && exact.size.width > gpui::px(100.0),
        "each toggle shows its label"
    );
    assert!(exact.left() > access.right());
    assert!(
        exact.left() - access.right() <= gpui::px(9.0),
        "side by side"
    );
    click(cx, "desktop-life-exact-amounts");
    workspace.read_with(cx, |workspace, _| {
        let privacy = &workspace.life_context.finance.privacy;
        assert!(!privacy.reveal_exact_amounts_to_agent, "disabled while off");
    });
    click(cx, "desktop-life-agent-access");
    click(cx, "desktop-life-exact-amounts");
    workspace.read_with(cx, |workspace, _| {
        let privacy = &workspace.life_context.finance.privacy;
        assert!(privacy.allow_agent_context);
        assert!(privacy.reveal_exact_amounts_to_agent);
    });

    // Health keeps its one entry form.
    workspace.update(cx, |workspace, cx| {
        workspace.life_tab = DesktopLifeTab::Health;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(bounds(cx, "desktop-life-quick-add").is_some());
    assert!(bounds(cx, "desktop-life-field-Account name").is_none());
}

/// Web `min-h-[96px]`: the CSV import box is several rows tall, not one
/// line (a multi-line Input takes its height from itself).
#[gpui::test]
fn the_csv_box_is_web_height(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = super::super::performance_tests::fixture(cx);
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(1400.0)));
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = super::super::AppView::Life;
        cx.notify();
    });
    cx.run_until_parked();
    let csv = cx
        .debug_bounds("desktop-life-csv-input")
        .expect("the CSV box is drawn");
    assert!(csv.size.height >= gpui::px(96.0), "{csv:?}");
}
