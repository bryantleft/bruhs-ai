//! Large-record UI regressions and diagnostic release timings.
use super::{AppView, InboxConversation, InboxMessage, InboxParticipant, InboxSource, Workspace};
use super::{DesktopPriority, DesktopTaskViewMode, DesktopWorkItem, DesktopWorkItemStatus};
use crate::app_state::DesktopState;
use crate::theme;
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use std::{cell::RefCell, hint::black_box, rc::Rc, time::Instant};

mod action_costs;
mod calendar_costs;
mod load_costs;
mod save_costs;

pub(super) fn fixture(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().unwrap());
    });
    let captured = Rc::new(RefCell::new(None));
    let capture = captured.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = captured.borrow().clone().unwrap();
    cx.run_until_parked();
    (workspace, cx)
}

fn inbox_records(count: usize) -> Vec<InboxConversation> {
    (0..count)
        .map(|index| {
            let at = i64::try_from(index).unwrap();
            let mut conversation = InboxConversation::new(
                InboxSource::Email,
                format!("Conversation {index}"),
                vec![InboxParticipant {
                    id: format!("participant-{index}"),
                    display_name: format!("Person {index}"),
                    address: format!("person-{index}@example.invalid"),
                    person_id: None,
                }],
                at,
            );
            conversation.id = format!("conversation-{index}");
            conversation.receive(InboxMessage::incoming(
                "Person",
                "A long message from the inbox. ".repeat(100),
                at,
            ));
            conversation
        })
        .collect()
}

fn print_samples(label: &str, mut samples: Vec<std::time::Duration>) {
    samples.sort_unstable();
    println!(
        "{label}: p50 {:.3} ms, p95 {:.3} ms",
        samples[samples.len() / 2].as_secs_f64() * 1000.0,
        samples[samples.len() * 95 / 100].as_secs_f64() * 1000.0
    );
}

fn tasks(count: usize) -> Vec<DesktopWorkItem> {
    (0..count)
        .map(|index| DesktopWorkItem {
            source: None,
            id: format!("task-{index}"),
            project_id: None,
            title: format!("Task {index}"),
            description: Some("Task context and notes. ".repeat(100)),
            status: DesktopWorkItemStatus::Todo,
            priority: DesktopPriority::Medium,
            due_date: None,
            planning: Default::default(),
            updated_at: bruhs_models::EpochMs::now(),
        })
        .collect()
}

fn selector(prefix: &str, id: &str) -> &'static str {
    Box::leak(format!("{prefix}{}", super::element_key(id)).into_boxed_str())
}

#[gpui::test]
fn task_pages_preserve_selection_edits_and_status_moves(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    cx.simulate_resize(gpui::size(gpui::px(1800.0), gpui::px(900.0)));
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.work_items = tasks(201);
        workspace.selected_project_id = None;
        workspace.selected_work_item_id = None;
        cx.notify();
    });
    cx.run_until_parked();
    let first = selector("desktop-task-row-", "task-0");
    let last = selector("desktop-task-row-", "task-200");
    assert!(cx.debug_bounds(first).is_some());
    assert!(cx.debug_bounds(last).is_none());
    let next = cx.debug_bounds("desktop-task-list-next").unwrap();
    cx.simulate_click(next.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let last_bounds = cx
        .debug_bounds(last)
        .expect("keyboard paging reaches the final task");
    cx.simulate_click(last_bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.selected_work_item_id.as_deref(), Some("task-200"));
        assert_eq!(
            workspace
                .task_detail_description_input
                .read(cx)
                .value()
                .as_str(),
            workspace.work_items[200].description.as_deref().unwrap()
        );
    });
    let board = cx.debug_bounds("desktop-task-view-board").unwrap();
    cx.simulate_click(board.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let selected_card = selector("desktop-task-card-", "task-200");
    assert!(
        cx.debug_bounds(selected_card).is_some(),
        "mode changes reveal the selection"
    );
    let previous = cx.debug_bounds("desktop-task-column-1-previous").unwrap();
    cx.simulate_click(previous.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        workspace.update_selected_work_item_title("Edited from another page".into(), cx);
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(selected_card).is_none(),
        "typing must not undo manual paging"
    );
    workspace.update(cx, |workspace, cx| {
        workspace.update_work_item_status("task-200", DesktopWorkItemStatus::Done, cx);
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(selected_card).is_some(),
        "status changes reveal the destination column"
    );
    workspace.update(cx, |workspace, cx| {
        workspace.work_items[0].project_id = Some("filtered".into());
        workspace.select_project(Some("filtered".into()), cx);
        workspace.set_task_view_mode(DesktopTaskViewMode::List, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds(first).is_some());
    assert!(cx.debug_bounds(last).is_none());
    assert!(cx.debug_bounds("desktop-task-list-next").is_none());
}

fn show_knowledge_footer(
    workspace: &Entity<Workspace>,
    section: usize,
    cx: &mut VisualTestContext,
) {
    workspace.update(cx, |workspace, cx| {
        let scroll = &workspace.library_pages.knowledge_scroll;
        let bounds = scroll.bounds_for_item(section).unwrap();
        scroll.set_offset(gpui::point(
            gpui::px(0.0),
            (scroll.bounds().bottom() - bounds.bottom()).min(gpui::px(0.0)),
        ));
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn knowledge_pages_reach_documents_and_sources_with_complete_text(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    // Like the web, the library lists the latest change first, so record 24
    // (the oldest) is the 25th card and opens the second page.
    let changed_at = |index: usize| 25 - i64::try_from(index).unwrap();
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Knowledge;
        workspace.knowledge_documents = (0..25)
            .map(|index| {
                let mut document = bruhs_models::KnowledgeDocument::new(
                    format!("Document {index}"),
                    "Full document content. ".repeat(100),
                    changed_at(index),
                );
                document.id = format!("document-{index}");
                document
            })
            .collect();
        workspace.source_clips = (0..25)
            .map(|index| {
                let mut clip = bruhs_models::SourceClip::new(
                    format!("Source {index}"),
                    format!("https://example.invalid/{index}"),
                    "Full source excerpt. ".repeat(100),
                    changed_at(index),
                );
                clip.id = format!("source-{index}");
                clip
            })
            .collect();
        cx.notify();
    });
    cx.run_until_parked();
    let last_document = selector("desktop-knowledge-document-", "document-24");
    let last_source = selector("desktop-knowledge-source-", "source-24");
    assert!(cx.debug_bounds(last_document).is_none());
    // The scroll's items: the search box, the documents, the sources.
    show_knowledge_footer(&workspace, 1, cx);
    let next = cx.debug_bounds("desktop-knowledge-documents-next").unwrap();
    cx.simulate_click(next.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.library_pages.documents, 1, "next page activated");
    });
    let last = cx.debug_bounds(last_document).unwrap();
    cx.simulate_click(last.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.active_view,
            AppView::DocumentDetail {
                document_id: "document-24".into()
            }
        );
        assert_eq!(
            workspace.knowledge_body_input.read(cx).value().as_str(),
            workspace.knowledge_documents[24].body_markdown
        );
        assert_eq!(
            workspace.library_pages.sources, 0,
            "sections page independently"
        );
    });
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Knowledge;
        cx.notify();
    });
    cx.run_until_parked();
    show_knowledge_footer(&workspace, 2, cx);
    let next = cx.debug_bounds("desktop-knowledge-sources-next").unwrap();
    cx.simulate_click(next.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let last = cx.debug_bounds(last_source).unwrap();
    cx.simulate_click(last.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.active_view,
            AppView::ClipDetail {
                clip_id: "source-24".into()
            }
        );
        assert_eq!(
            workspace.clip_excerpt_input.read(cx).value().as_str(),
            workspace.source_clips[24].excerpt
        );
    });
}

#[gpui::test]
#[ignore = "manual optimized action timing; cargo test -p bruhs-desktop --bin bruhs-desktop --release native_library_action_cost -- --ignored --nocapture"]
fn native_library_action_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    for count in [100, 1000, 10_000] {
        workspace.update(cx, |workspace, _| {
            workspace.active_view = AppView::Tasks;
            workspace.work_items = tasks(count);
            workspace.selected_project_id = None;
            workspace.selected_work_item_id = None;
        });
        for mode in [DesktopTaskViewMode::List, DesktopTaskViewMode::Board] {
            workspace.update(cx, |workspace, cx| {
                workspace.task_view_mode = mode;
                cx.notify();
            });
            cx.run_until_parked();
            let samples = (0..11)
                .map(|_| {
                    let start = Instant::now();
                    workspace.update(cx, |_, cx| cx.notify());
                    cx.run_until_parked();
                    start.elapsed()
                })
                .collect();
            print_samples(&format!("native tasks {mode:?} / {count}"), samples);
        }
        workspace.update(cx, |workspace, cx| {
            workspace.active_view = AppView::Knowledge;
            workspace.work_items.clear();
            workspace.knowledge_documents = (0..count)
                .map(|index| {
                    let mut document = bruhs_models::KnowledgeDocument::new(
                        format!("Document {index}"),
                        "Saved knowledge and notes. ".repeat(100),
                        i64::try_from(index).unwrap(),
                    );
                    document.id = format!("document-{index}");
                    document
                })
                .collect();
            cx.notify();
        });
        cx.run_until_parked();
        let samples = (0..11)
            .map(|_| {
                let start = Instant::now();
                workspace.update(cx, |_, cx| cx.notify());
                cx.run_until_parked();
                start.elapsed()
            })
            .collect();
        print_samples(&format!("native knowledge / {count}"), samples);
        workspace.update(cx, |workspace, _| workspace.knowledge_documents.clear());
    }
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Tasks;
        workspace.task_view_mode = DesktopTaskViewMode::Board;
        workspace.work_items = tasks(10_000);
        for (index, item) in workspace.work_items.iter_mut().enumerate() {
            item.status = DesktopWorkItemStatus::board_columns()[index % 4];
            item.title = format!("Task {index} with a longer title wrapping onto a second line");
            item.due_date = Some("2026-11-10".into());
        }
        cx.notify();
    });
    cx.run_until_parked();
    let samples = (0..11)
        .map(|_| {
            let start = Instant::now();
            workspace.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            start.elapsed()
        })
        .collect();
    print_samples("native dense board / 10,000 across four columns", samples);
    workspace.update(cx, |workspace, cx| {
        workspace.work_items.clear();
        workspace.active_view = AppView::Chat;
        cx.notify();
    });
    let chat = workspace.read_with(cx, |workspace, _| {
        workspace.active_thread().unwrap().chat.clone()
    });
    for count in [100, 1000, 5000] {
        chat.update(cx, |chat, cx| {
            chat.messages = (0..count)
                .map(|index| {
                    crate::views::chat::Message::user(format!(
                        "Message {index}: {}",
                        "Saved chat text and context. ".repeat(5)
                    ))
                })
                .collect();
            cx.notify();
        });
        cx.run_until_parked();
        let samples = (0..11)
            .map(|_| {
                let start = Instant::now();
                chat.update(cx, |_, cx| cx.notify());
                cx.run_until_parked();
                start.elapsed()
            })
            .collect();
        print_samples(&format!("native chat / {count} messages"), samples);
    }
}

#[gpui::test]
fn inbox_pages_reach_records_by_pointer_and_preserve_full_history(cx: &mut TestAppContext) {
    use bruhs_models::INBOX_PAGE_SIZE;
    let (workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Inbox;
        workspace.inbox_conversations = inbox_records(INBOX_PAGE_SIZE + 1);
        cx.notify();
    });
    cx.run_until_parked();
    let newest: &'static str = Box::leak(
        format!(
            "desktop-inbox-card-{}",
            super::element_key(&format!("conversation-{INBOX_PAGE_SIZE}"))
        )
        .into_boxed_str(),
    );
    let last: &'static str = Box::leak(
        format!(
            "desktop-inbox-card-{}",
            super::element_key("conversation-0")
        )
        .into_boxed_str(),
    );
    assert!(cx.debug_bounds(newest).is_some());
    assert!(cx.debug_bounds(last).is_none());
    let next = cx.debug_bounds("desktop-inbox-next").unwrap();
    cx.simulate_click(next.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds(newest).is_none());
    let last_bounds = cx.debug_bounds(last).unwrap();
    cx.simulate_click(last_bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.active_view,
            AppView::ConversationDetail {
                conversation_id: "conversation-0".into()
            }
        );
        assert!(workspace.inbox_conversations[0].messages[0].body.len() > 160);
    });
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Inbox;
        workspace
            .inbox_conversations
            .retain(|record| record.id == "conversation-0");
        cx.notify();
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds(last).is_some(),
        "shrinking data clamps a stale page"
    );
    assert!(cx.debug_bounds("desktop-inbox-next").is_none());
}

#[gpui::test]
#[ignore = "manual optimized action timing; cargo test -p bruhs-desktop --bin bruhs-desktop --release large_action_cost -- --ignored --nocapture"]
fn large_action_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    for count in [100, 1000, 10_000] {
        workspace.update(cx, |workspace, cx| {
            workspace.active_view = AppView::Inbox;
            workspace.inbox_conversations = inbox_records(count);
            cx.notify();
        });
        cx.run_until_parked();
        let mut samples = Vec::new();
        for _ in 0..11 {
            let start = Instant::now();
            workspace.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            samples.push(start.elapsed());
        }
        print_samples(
            &format!("native inbox frame / {count} conversations"),
            samples,
        );
    }
    workspace.update(cx, |workspace, cx| {
        workspace.active_view = AppView::Search;
        workspace.inbox_conversations.clear();
        let chat = workspace.active_thread().unwrap().chat.clone();
        chat.update(cx, |chat, _| {
            chat.messages = (0..5000)
                .map(|_| crate::views::chat::Message::user("Saved chat content. ".repeat(200)))
                .collect();
        });
    });
    let samples = (0..31)
        .map(|_| {
            let start = Instant::now();
            workspace.read_with(cx, |workspace, cx| black_box(workspace.snapshot(cx)));
            start.elapsed()
        })
        .collect();
    print_samples("workspace snapshot / 5,000 messages", samples);
}
