//! PM-53: each layout switches at the web's own breakpoint. Web docks the
//! sidebar and the task detail from `sm` (640px), the right panel and the
//! Tasks project list from `md` (768px), the document's metadata column
//! from `lg` (1024px), and puts booking times in two columns from `sm`.

use super::{
    docked_right_panel_range, docked_right_panel_width, AppView, DesktopCalendarMode, Workspace,
    RIGHT_PANEL_MIN_WIDTH_PX, WORKSPACE_RIGHT_PANEL_WIDTH_PX,
};
use crate::{app_state::DesktopState, theme};
use bruhs_models::KnowledgeDocument;
use bruhs_ui::zoom::px;
use gpui::{size, AppContext, Bounds, Entity, Pixels, TestAppContext, VisualTestContext};
use std::{cell::RefCell, rc::Rc};

fn workspace(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state"));
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().expect("workspace");
    cx.run_until_parked();
    (workspace, cx)
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn open_tasks_panel(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Chat;
        workspace.sidebar_visible = true;
        workspace.tasks_panel_visible = true;
        cx.notify();
    });
    cx.run_until_parked();
}

/// The shared `clamp_panel_width`, with web's own cases: at least 360px, at
/// most 70% of the window, never so wide the chat column collapses.
#[test]
fn the_right_panel_width_matches_web() {
    use bruhs_models::logic::layout;
    assert_eq!(
        f64::from(WORKSPACE_RIGHT_PANEL_WIDTH_PX),
        layout::RIGHT_PANEL_DEFAULT_WIDTH
    );
    assert_eq!(
        f64::from(RIGHT_PANEL_MIN_WIDTH_PX),
        layout::RIGHT_PANEL_MIN_WIDTH
    );
    assert_eq!(docked_right_panel_width(540.0, 1440.0, 0.0), 540.0);
    assert_eq!(docked_right_panel_width(100.0, 1440.0, 0.0), 360.0);
    assert_eq!(docked_right_panel_width(2000.0, 1440.0, 0.0), 1008.0);
    assert_eq!(docked_right_panel_width(600.0, 800.0, 0.0), 440.0);
    // Beside the docked sidebar the chat keeps its 360px.
    assert_eq!(docked_right_panel_width(540.0, 800.0, 248.0), 192.0);
    assert_eq!(docked_right_panel_range(800.0, 248.0), (192.0, 192.0));
}

/// At the window's 640px minimum the sidebar docks, as web's does from
/// `sm`; the right panel stays a 384px drawer over a scrim that closes it
/// until `md`, where it docks and leaves the chat its 360px.
#[gpui::test]
fn the_sidebar_docks_from_sm_and_the_right_panel_from_md_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    open_tasks_panel(&workspace, cx);
    assert!(shown(cx, "desktop-workspace-sidebar"), "docked from sm");
    assert!(!shown(cx, "desktop-compact-sidebar-drawer"));
    let sidebar = bounds(cx, "desktop-workspace-sidebar");
    assert_eq!(sidebar.left(), px(0.0));

    let drawer = bounds(cx, "desktop-compact-tasks-panel");
    assert_eq!(drawer.size.width, px(384.0), "web w-[min(88vw,24rem)]");
    assert_eq!(drawer.right(), px(640.0));
    assert!(shown(cx, "desktop-compact-right-panel-backdrop"));
    // The scrim covers the docked sidebar too; a click on it closes the
    // panel, as web's `md:hidden` scrim does.
    cx.simulate_click(gpui::point(px(40.0), px(240.0)), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(!shown(cx, "desktop-compact-tasks-panel"));
    assert!(!shown(cx, "desktop-compact-right-panel-backdrop"));
    workspace.read_with(cx, |workspace, _| {
        assert!(!workspace.right_panel_open(), "the scrim closed the panel");
        assert!(workspace.sidebar_visible, "and left the sidebar docked");
    });

    // From md the panel docks. Beside the docked sidebar the chat keeps
    // web's 360px minimum and the panel gives way.
    cx.simulate_resize(size(px(800.0), px(600.0)));
    open_tasks_panel(&workspace, cx);
    assert!(!shown(cx, "desktop-compact-tasks-panel"));
    assert!(!shown(cx, "desktop-compact-right-panel-backdrop"));
    let sidebar = bounds(cx, "desktop-workspace-sidebar");
    let panel = bounds(cx, "desktop-right-panel");
    assert_eq!(panel.right(), px(800.0));
    let chat = f32::from(panel.left() - sidebar.right());
    assert!(
        (359.0..=361.0).contains(&chat),
        "the chat keeps 360px beside the panel: {chat}"
    );

    // The window's room, not a rescaled split, sets the width: hiding the
    // sidebar gives the panel its remembered 540px back as far as web's
    // clamp allows (440px at 800px), and a wide window all of it.
    workspace.update(cx, |workspace, cx| {
        workspace.sidebar_visible = false;
        cx.notify();
    });
    cx.run_until_parked();
    assert_eq!(bounds(cx, "desktop-right-panel").size.width, px(440.0));
    cx.simulate_resize(size(px(1440.0), px(900.0)));
    cx.run_until_parked();
    assert_eq!(bounds(cx, "desktop-right-panel").size.width, px(540.0));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.right_panel_width, 540.0, "never rewritten");
    });
}

/// Web's Tasks page: the project list docks from `md` ("Projects" opens it
/// below) and the task detail docks from `sm` (a drawer below).
#[gpui::test]
fn tasks_dock_projects_from_md_and_the_detail_from_sm_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.sidebar_visible = true;
        workspace.task_create_title_input.update(cx, |input, cx| {
            input.set_value("Narrow detail", window, cx);
        });
        workspace.create_task_from_title_input(window, cx);
        workspace.selected_work_item_id = workspace.work_items.last().map(|item| item.id.clone());
        cx.notify();
    });
    for (width, projects_docked, detail_drawer) in [
        (600.0, false, true),
        (700.0, false, false),
        (800.0, true, false),
    ] {
        cx.simulate_resize(size(px(width), px(700.0)));
        cx.run_until_parked();
        assert_eq!(
            shown(cx, "desktop-project-sidebar"),
            projects_docked,
            "project list at {width}px"
        );
        assert_eq!(
            shown(cx, "desktop-task-open-projects"),
            !projects_docked,
            "Projects button at {width}px"
        );
        assert!(shown(cx, "desktop-task-detail"), "detail at {width}px");
        assert_eq!(
            shown(cx, "desktop-compact-task-detail"),
            detail_drawer,
            "detail drawer at {width}px"
        );
    }
}

/// Web `lg:grid-cols-[minmax(0,1fr)_18rem]`: a document's metadata column
/// sits under the editor below 1024px and beside it from there.
#[gpui::test]
fn the_document_metadata_column_docks_from_lg_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let mut plan = KnowledgeDocument::new("Plan", "Body", 1);
        plan.id = "plan".into();
        workspace.knowledge_documents = vec![plan];
        workspace.select_knowledge_document("plan", window, cx);
    });
    cx.simulate_resize(size(px(1000.0), px(1400.0)));
    cx.run_until_parked();
    let body = bounds(cx, "desktop-knowledge-body");
    let meta = bounds(cx, "desktop-knowledge-document-meta");
    assert!(meta.top() >= body.bottom(), "under the editor below lg");

    cx.simulate_resize(size(px(1100.0), px(1400.0)));
    cx.run_until_parked();
    let body = bounds(cx, "desktop-knowledge-body");
    let meta = bounds(cx, "desktop-knowledge-document-meta");
    assert!(meta.left() >= body.right(), "beside the editor from lg");
    assert_eq!(meta.size.width, px(288.0), "web 18rem");
}

/// Web `sm:grid-cols-2`: booking times in two columns from 640px.
#[gpui::test]
fn booking_times_use_two_columns_from_sm_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let link = bruhs_models::SchedulingLink::new("Intro call", 30);
        let slug = link.slug.clone();
        workspace.scheduling_preferences.availability[0].weekdays = (0..=6).collect();
        workspace.scheduling_preferences.links = vec![link];
        workspace.open_booking_page(slug, window, cx);
    });
    cx.simulate_resize(size(px(600.0), px(900.0)));
    cx.run_until_parked();
    let first = bounds(cx, "desktop-booking-slot-0");
    let second = bounds(cx, "desktop-booking-slot-1");
    assert!(second.top() >= first.bottom(), "one column below sm");

    cx.simulate_resize(size(px(640.0), px(900.0)));
    cx.run_until_parked();
    let first = bounds(cx, "desktop-booking-slot-0");
    let second = bounds(cx, "desktop-booking-slot-1");
    assert_eq!(second.top(), first.top(), "two columns from sm");
    assert!(second.left() >= first.right());
}

/// With the sidebar docked at 640px a page is about 390px wide. Like the
/// web's `flex-wrap` headers, the desktop's wrap their actions under the
/// title instead of squeezing it or running past the window's edge.
#[gpui::test]
fn narrow_page_headers_wrap_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    let inside = |cx: &mut VisualTestContext, selector: &'static str| {
        let element = bounds(cx, selector);
        assert!(
            element.right() <= px(640.0),
            "{selector} runs past the window: {element:?}"
        );
        element
    };
    let show = |workspace: &Entity<Workspace>, cx: &mut VisualTestContext, view: AppView| {
        workspace.update(cx, |workspace, cx| {
            workspace.active_view = view;
            workspace.sidebar_visible = true;
            cx.notify();
        });
        cx.run_until_parked();
    };

    show(&workspace, cx, AppView::Today);
    assert!(shown(cx, "desktop-workspace-sidebar"));
    let title = inside(cx, "desktop-page-title");
    assert!(
        title.size.width >= px(80.0) && title.size.height <= px(60.0),
        "Today's title keeps its width: {title:?}"
    );
    let plan = inside(cx, "desktop-today-plan");
    assert!(plan.top() >= title.bottom(), "the actions wrap under it");

    show(&workspace, cx, AppView::Knowledge);
    inside(cx, "desktop-knowledge-new-document");

    workspace.update_in(cx, |workspace, window, cx| {
        let mut plan = KnowledgeDocument::new("Plan", "Body", 1);
        plan.id = "plan".into();
        workspace.knowledge_documents = vec![plan];
        workspace.select_knowledge_document("plan", window, cx);
    });
    cx.run_until_parked();
    inside(cx, "desktop-knowledge-save");

    show(&workspace, cx, AppView::Life);
    inside(cx, "desktop-life-title");
    inside(cx, "desktop-life-privacy-text");
    inside(cx, "desktop-life-agent-access");

    show(&workspace, cx, AppView::People);
    inside(cx, "desktop-page-title");
}

/// Web `grid-rows-6`: the month's six weeks share the window's height, so a
/// short window shows every week and clips each day's records instead of
/// scrolling the month.
#[gpui::test]
fn the_month_fits_its_six_weeks_in_a_short_window_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    cx.simulate_resize(size(px(640.0), px(480.0)));
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Calendar;
        workspace.calendar_mode = DesktopCalendarMode::Month;
        workspace.calendar_anchor =
            chrono::NaiveDate::from_ymd_opt(2026, 11, 16).expect("a valid date");
        cx.notify();
    });
    cx.run_until_parked();
    let month = bounds(cx, "desktop-calendar-month");
    assert!(month.bottom() <= px(480.0), "{month:?}");
    // November 2026 starts on a Sunday; its sixth week ends on December 12.
    let first = bounds(cx, "desktop-calendar-day-2026-11-01");
    let last = bounds(cx, "desktop-calendar-day-2026-12-12");
    assert!(
        last.bottom() <= month.bottom(),
        "the sixth week shows: {last:?}"
    );
    assert!(
        (f32::from(last.size.height) - f32::from(first.size.height)).abs() <= 1.0,
        "the weeks share the height: {first:?} {last:?}"
    );
}

/// Web Agent workflows: `lg:grid-cols-[minmax(0,1fr)_22rem]`, so the grants
/// sit beside the workflows from 1024px and under them below.
#[gpui::test]
fn agent_workflows_put_the_grants_beside_from_lg_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Automations;
        cx.notify();
    });
    cx.simulate_resize(size(px(1000.0), px(1600.0)));
    cx.run_until_parked();
    let workflows = bounds(cx, "desktop-automation-workflows");
    let grants = bounds(cx, "desktop-automation-grants");
    assert!(
        grants.top() >= workflows.bottom(),
        "under the workflows below lg"
    );

    cx.simulate_resize(size(px(1100.0), px(1600.0)));
    cx.run_until_parked();
    let workflows = bounds(cx, "desktop-automation-workflows");
    let grants = bounds(cx, "desktop-automation-grants");
    assert!(grants.left() >= workflows.right(), "beside them from lg");
    assert_eq!(grants.size.width, px(352.0), "web 22rem");
}

/// Web Life: the summary cards are one column below `md` and three from it.
#[gpui::test]
fn life_summary_cards_take_three_columns_from_md_like_web(cx: &mut TestAppContext) {
    let (workspace, cx) = workspace(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Life;
        workspace.sidebar_visible = true;
        cx.notify();
    });
    for (width, columns) in [(700.0, 1.0), (800.0, 3.0)] {
        cx.simulate_resize(size(px(width), px(900.0)));
        cx.run_until_parked();
        let grid = bounds(cx, "desktop-life-summary");
        let card = bounds(cx, "desktop-life-summary-Manual net worth");
        let share = f32::from(grid.size.width) / f32::from(card.size.width);
        assert!(
            (share - columns).abs() < 0.3,
            "{columns} column(s) at {width}px: grid {grid:?}, card {card:?}"
        );
    }
}
