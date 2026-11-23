//! Web parity for Today's layout (PM-39): the header's quiet links, the
//! local suggestion's wording and count, the numbered focus queue, and the
//! four status cards in one row, each with its dot.

use super::super::performance_tests::fixture;
use super::super::{AppView, DesktopWorkItem, EpochMs};
use bruhs_models::{AutomationState, Priority, WorkItemPlanning, WorkItemStatus};
use bruhs_ui::zoom::px;
use gpui::{size, Bounds, Pixels, VisualTestContext};

fn task(id: &str, priority: Priority, status: WorkItemStatus) -> DesktopWorkItem {
    DesktopWorkItem {
        source: None,
        id: id.into(),
        project_id: None,
        title: format!("Task {id}"),
        description: None,
        status,
        priority,
        due_date: None,
        planning: WorkItemPlanning {
            estimate_minutes: Some(30),
            ..WorkItemPlanning::default()
        },
        updated_at: EpochMs::saturating_from_i64(1),
    }
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
}

#[gpui::test]
fn today_lays_out_its_cards_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(size(px(1280.0), px(800.0)));
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Today;
        workspace.automation_state = AutomationState::default();
        workspace.work_items = vec![
            task("ship", Priority::Urgent, WorkItemStatus::Todo),
            task("notes", Priority::Medium, WorkItemStatus::Todo),
            task("changelog", Priority::High, WorkItemStatus::InProgress),
            task("done", Priority::Low, WorkItemStatus::Done),
        ];
        // The first save offers the suggestion, as the web's Today does once
        // unfinished tasks load.
        workspace.persist_and_notify(cx);
    });
    cx.run_until_parked();

    workspace.read_with(cx, |workspace, _| {
        let suggestion = &workspace.automation_state.suggestions[0];
        assert_eq!(
            suggestion.explanation,
            "A task, calendar block, and local note can be reviewed as one reversible transaction."
        );
        assert!(
            suggestion.evidence.starts_with("3 unfinished tasks and "),
            "the done task is not counted: {}",
            suggestion.evidence
        );
    });
    assert!(cx
        .debug_bounds("desktop-today-suggestion-evidence")
        .is_some());
    assert!(cx.debug_bounds("desktop-today-agent-workflows").is_some());
    assert!(cx.debug_bounds("desktop-today-life").is_some());

    // The focus queue numbers its rows and says how many are ready.
    let ready = bounds(cx, "desktop-today-focus-ready");
    let queue = bounds(cx, "desktop-today-section-focus-queue");
    assert!(queue.contains(&ready.center()));
    let ranks = [
        bounds(cx, "desktop-today-focus-rank-0"),
        bounds(cx, "desktop-today-focus-rank-1"),
        bounds(cx, "desktop-today-focus-rank-2"),
    ];
    for pair in ranks.windows(2) {
        assert_eq!(pair[0].origin.x, pair[1].origin.x, "ranks share a column");
        assert!(
            pair[1].origin.y > pair[0].origin.y,
            "ranks go down in order"
        );
    }
    assert_eq!(ranks[0].size, size(px(24.0), px(24.0)));
    let priority = bounds(cx, "desktop-today-focus-priority-0");
    let row = bounds(cx, "desktop-today-focus-0");
    assert!(row.contains(&priority.center()));
    assert!(cx.debug_bounds("desktop-today-focus-3").is_none());

    // Now & next sits beside the wider Focus queue.
    let schedule = bounds(cx, "desktop-today-section-now-&-next");
    assert_eq!(schedule.origin.y, queue.origin.y);
    assert!(schedule.size.width < queue.size.width);

    // The four status cards share one row, each with its dot.
    let keys = ["overdue", "waiting-on", "follow-ups", "decisions"];
    let cards = [
        bounds(cx, "desktop-today-section-overdue"),
        bounds(cx, "desktop-today-section-waiting-on"),
        bounds(cx, "desktop-today-section-follow-ups"),
        bounds(cx, "desktop-today-section-decisions"),
    ];
    let dots = [
        bounds(cx, "desktop-today-dot-overdue"),
        bounds(cx, "desktop-today-dot-waiting-on"),
        bounds(cx, "desktop-today-dot-follow-ups"),
        bounds(cx, "desktop-today-dot-decisions"),
    ];
    for (index, card) in cards.iter().enumerate() {
        assert_eq!(
            card.origin.y, cards[0].origin.y,
            "{} in the row",
            keys[index]
        );
        assert_eq!(
            card.size.width, cards[0].size.width,
            "{} width",
            keys[index]
        );
        assert!(card.contains(&dots[index].center()), "{} dot", keys[index]);
        assert_eq!(dots[index].size, size(px(8.0), px(8.0)));
    }
    for pair in cards.windows(2) {
        assert!(pair[1].origin.x > pair[0].origin.x + pair[0].size.width);
    }

    // Below the web's `xl` breakpoint the cards sit two to a row.
    cx.simulate_resize(size(px(1000.0), px(800.0)));
    cx.run_until_parked();
    let overdue = bounds(cx, "desktop-today-section-overdue");
    let waiting = bounds(cx, "desktop-today-section-waiting-on");
    let follow_ups = bounds(cx, "desktop-today-section-follow-ups");
    assert_eq!(overdue.origin.y, waiting.origin.y);
    assert_eq!(follow_ups.origin.x, overdue.origin.x);
    assert!(follow_ups.origin.y > overdue.origin.y);
}
