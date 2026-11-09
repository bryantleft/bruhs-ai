use super::tests::{call, connect, fixture};
use super::WorkspaceBridge;
use bruhs_core::workspace_bridge::{
    BridgeRequest, BridgeResponse, FinanceReadSection, HealthReadSection,
};
use bruhs_models::*;
use gpui::TestAppContext;
use serde_json::Value;

fn finance(section: FinanceReadSection, query: &str, offset: usize) -> BridgeRequest {
    BridgeRequest::ReadFinance {
        section,
        query: query.into(),
        offset,
    }
}

fn health(section: HealthReadSection, query: &str, offset: usize) -> BridgeRequest {
    BridgeRequest::ReadHealth {
        section,
        query: query.into(),
        offset,
    }
}

fn ok(response: BridgeResponse) -> Value {
    match response {
        BridgeResponse::Ok(value) => value,
        other => panic!("Expected page, got {other:?}"),
    }
}

// Scoped native handler timing: excludes socket/stdio, hydration, file I/O and paint.
#[cfg(not(debug_assertions))]
#[gpui::test]
fn life_read_handler_large_ledger_measurement(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, cx| {
        w.life_context = LifeContext::default();
        w.automation_state
            .permissions
            .set(AgentDomain::Finance, AgentAccess::Read, true);
        w.life_context.finance.privacy.allow_agent_context = true;
        w.life_context.finance.transactions = (0..100_001)
            .map(|i| {
                let mut record =
                    FinanceTransaction::manual(format!("Merchant {i}"), "Category", i, i);
                record.id = format!("transaction-{i}");
                record
            })
            .collect();
        for (label, query, offset, count) in [
            ("first-page", "", 0, 50),
            ("deep-page", "", 99_950, 50),
            ("last-match", "Merchant 100000", 0, 1),
            ("no-match", "missing", 0, 0),
        ] {
            let mut samples = Vec::new();
            for iteration in 0..55 {
                let request = finance(FinanceReadSection::Transactions, query, offset);
                let start = std::time::Instant::now();
                let response = w.handle_bridge_request(request, cx);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(ok(response)["records"].as_array().unwrap().len(), count);
                if iteration >= 5 {
                    samples.push(elapsed);
                }
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "LIFE_READ_HANDLER 100001 records {label} p50={:.3}ms p95={:.3}ms p99={:.3}ms",
                samples[24], samples[47], samples[49]
            );
        }
    });
}

#[gpui::test]
async fn life_reads_recheck_both_gates_and_never_infer_read_from_write(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| w.life_context = LifeContext::default());
    connect(&workspace, cx).await;
    for domain in [AgentDomain::Finance, AgentDomain::Health] {
        for read in [false, true] {
            for context in [false, true] {
                workspace.update_in(cx, |w, _, _| {
                    w.automation_state
                        .permissions
                        .set(domain, AgentAccess::Write, true);
                    w.automation_state
                        .permissions
                        .set(domain, AgentAccess::Read, read);
                    w.life_context.finance.privacy.allow_agent_context = context;
                    w.life_context.health.privacy.allow_agent_context = context;
                });
                let selected = match domain {
                    AgentDomain::Finance => finance(Default::default(), "", 0),
                    _ => health(Default::default(), "", 0),
                };
                for request in [
                    selected,
                    BridgeRequest::Read {
                        domain,
                        query: String::new(),
                        offset: 0,
                    },
                ] {
                    assert_eq!(
                        matches!(call(&workspace, cx, request).await, BridgeResponse::Ok(_)),
                        read && context
                    );
                }
            }
        }
        // Revoke context after a successful request on the same connection.
        workspace.update_in(cx, |w, _, _| {
            w.life_context.finance.privacy.allow_agent_context = false;
            w.life_context.health.privacy.allow_agent_context = false;
        });
        assert!(matches!(
            call(
                &workspace,
                cx,
                BridgeRequest::Read {
                    domain,
                    query: String::new(),
                    offset: 0
                }
            )
            .await,
            BridgeResponse::Error(_)
        ));
        workspace.update_in(cx, |w, _, _| {
            w.life_context.finance.privacy.allow_agent_context = true;
            w.life_context.health.privacy.allow_agent_context = true;
        });
        // Independently revoke Read with context still enabled.
        workspace.update_in(cx, |w, _, _| {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Read, false)
        });
        assert!(matches!(
            call(
                &workspace,
                cx,
                BridgeRequest::Read {
                    domain,
                    query: String::new(),
                    offset: 0
                }
            )
            .await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        assert!(w.automation_state.workflows.is_empty());
        assert!(w.automation_state.jobs.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}

#[gpui::test]
async fn finance_sections_redact_by_default_preserve_exact_money_and_exclude_note_search(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.life_context = LifeContext::default();
        w.automation_state
            .permissions
            .set(AgentDomain::Finance, AgentAccess::Read, true);
        let f = &mut w.life_context.finance;
        f.privacy.allow_agent_context = true;
        f.updated_at_ms = 123;
        f.accounts
            .push(FinanceAccount::manual("Account", i64::MIN, 1));
        let mut transaction =
            FinanceTransaction::manual("Merchant", "Category", -9_007_199_254_740_993, 1);
        transaction.note = Some("never-export-this-note".into());
        transaction.currency = "EUR".into();
        f.transactions.push(transaction);
        let mut budget = FinanceBudget::monthly("Category", 1);
        budget.monthly_limit_minor = u64::MAX;
        f.budgets.push(budget);
        let mut income = IncomeStream::monthly("Income", 1);
        income.expected_monthly_minor = u64::MAX;
        f.income_streams.push(income);
        let mut holding = Holding::manual("BRUHS", i64::MAX, 1);
        holding.quantity = 1.25;
        holding.cost_basis_minor = Some(i64::MIN);
        f.holdings.push(holding);
    });
    connect(&workspace, cx).await;
    let cases = [
        (
            FinanceReadSection::Accounts,
            "balance_minor",
            i64::MIN.to_string(),
        ),
        (
            FinanceReadSection::Transactions,
            "amount_minor",
            "-9007199254740993".into(),
        ),
        (
            FinanceReadSection::Budgets,
            "monthly_limit_minor",
            u64::MAX.to_string(),
        ),
        (
            FinanceReadSection::IncomeStreams,
            "expected_monthly_minor",
            u64::MAX.to_string(),
        ),
        (
            FinanceReadSection::Holdings,
            "current_value_minor",
            i64::MAX.to_string(),
        ),
    ];
    for exact in [false, true, false] {
        workspace.update_in(cx, |w, _, _| {
            w.life_context.finance.privacy.reveal_exact_amounts_to_agent = exact
        });
        for (section, field, expected) in &cases {
            let page = ok(call(&workspace, cx, finance(*section, "", 0)).await);
            assert_eq!(page["amounts_redacted"], !exact);
            assert_eq!(page["durable_receipt"], false);
            assert_eq!(page["updated_at_ms"], 123);
            assert_eq!(page["source"], "live_workspace");
            assert_eq!(page["section"], serde_json::to_value(section).unwrap());
            assert_eq!(page["records"].as_array().unwrap().len(), 1);
            let record = &page["records"][0];
            if exact {
                assert_eq!(record[*field], expected.as_str());
            } else {
                assert!(record.get(*field).is_none());
            }
            assert!(record.get("note").is_none());
            if *section == FinanceReadSection::Holdings {
                assert_eq!(record.get("quantity").is_some(), exact);
                assert_eq!(record.get("cost_basis_minor").is_some(), exact);
                if exact {
                    assert_eq!(record["cost_basis_minor"], i64::MIN.to_string());
                }
            }
            if *section == FinanceReadSection::Transactions {
                assert_eq!(record["currency"], "EUR");
            }
        }
        for query in ["never-export-this-note", "9007199254740993"] {
            assert_eq!(
                ok(call(
                    &workspace,
                    cx,
                    finance(FinanceReadSection::Transactions, query, 0)
                )
                .await)["records"],
                serde_json::json!([])
            );
        }
    }
    let page = ok(call(
        &workspace,
        cx,
        finance(FinanceReadSection::Transactions, "mERCHant", 0),
    )
    .await);
    assert_eq!(page["records"].as_array().unwrap().len(), 1);
    workspace.update_in(cx, |w, _, _| w.mcp_bridge = WorkspaceBridge::default());
}

#[gpui::test]
async fn life_reads_bound_pages_and_report_invalid_selected_records_without_mutation(
    cx: &mut TestAppContext,
) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |w, _, _| {
        w.life_context = LifeContext::default();
        for domain in [AgentDomain::Finance, AgentDomain::Health] {
            w.automation_state
                .permissions
                .set(domain, AgentAccess::Read, true);
        }
        w.life_context.finance.privacy.allow_agent_context = true;
        w.life_context.health.privacy.allow_agent_context = true;
        let h = &mut w.life_context.health;
        h.workouts = (0..123)
            .map(|i| WorkoutLog::manual(format!("Walk {i}"), 30, i))
            .collect();
        h.sleep.push(SleepLog::manual(480, 85, 1));
        h.recovery.push(RecoveryLog::manual(75, 1));
        h.habits.push(Habit::weekly("Read", 5));
        let mut holding = Holding::manual("Invalid", 100, 1);
        holding.quantity = f64::NAN;
        w.life_context.finance.holdings.push(holding);
    });
    connect(&workspace, cx).await;
    for (offset, count, next) in [
        (0, 50, Some(50)),
        (50, 50, Some(100)),
        (100, 23, None),
        (123, 0, None),
    ] {
        let page = ok(call(
            &workspace,
            cx,
            health(HealthReadSection::Workouts, "", offset),
        )
        .await);
        assert_eq!(page["records"].as_array().unwrap().len(), count);
        assert_eq!(page["next_offset"], serde_json::to_value(next).unwrap());
        if count > 0 {
            assert_eq!(page["records"][0]["activity"], format!("Walk {offset}"));
        }
    }
    for section in [
        HealthReadSection::Sleep,
        HealthReadSection::Recovery,
        HealthReadSection::Habits,
    ] {
        assert_eq!(
            ok(call(&workspace, cx, health(section, "", 0)).await)["records"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    ok(call(&workspace, cx, finance(FinanceReadSection::Holdings, "", 0)).await);
    workspace.update_in(cx, |w, _, _| {
        w.life_context.finance.privacy.reveal_exact_amounts_to_agent = true
    });
    assert!(matches!(
        call(&workspace, cx, finance(FinanceReadSection::Holdings, "", 0)).await,
        BridgeResponse::Error(_)
    ));
    // Unselected invalid data does not poison another page.
    ok(call(&workspace, cx, finance(FinanceReadSection::Holdings, "", 1)).await);
    workspace.update_in(cx, |w, _, _| {
        w.life_context.health.recovery[0].note = Some("\n".repeat(140_000));
        w.life_context
            .health
            .recovery
            .push(w.life_context.health.recovery[0].clone());
    });
    let page = ok(call(&workspace, cx, health(HealthReadSection::Recovery, "", 0)).await);
    assert_eq!(page["next_offset"], 1);
    assert!(page.to_string().len() < 512 * 1024);
    ok(call(&workspace, cx, health(HealthReadSection::Recovery, "", 1)).await);
    workspace.update_in(cx, |w, _, _| {
        w.life_context.health.recovery[0].note = Some("\n".repeat(300_000))
    });
    assert!(matches!(
        call(&workspace, cx, health(HealthReadSection::Recovery, "", 0)).await,
        BridgeResponse::Error(_)
    ));
    for request in [
        finance(Default::default(), "", 100_001),
        health(Default::default(), &"x".repeat(513), 0),
    ] {
        assert!(matches!(
            call(&workspace, cx, request).await,
            BridgeResponse::Error(_)
        ));
    }
    workspace.update_in(cx, |w, _, _| {
        assert_eq!(w.life_context.health.workouts.len(), 123);
        assert!(w.automation_state.jobs.is_empty());
        w.mcp_bridge = WorkspaceBridge::default();
    });
}
