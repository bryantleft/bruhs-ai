//! Move in end to end: the agent tabs, By prompt, the palette, the hand
//! over to New bruh, the Export file tab's memory, and the keyboard. The
//! fixtures are the web's (Task 11), so both apps agree.

use super::super::import_ui::export_tab::ExportCalls;
use super::super::import_ui::ImportTab;
use super::super::palette_ui::COMMAND_ACTIONS;
use super::super::{DesktopState, Overlay, TopLayer, Workspace};
use super::{MoveInStart, MoveInTab, ScanCall};
use crate::theme;
use bruhs_models::crew::{BruhAccess, BruhLook, BruhState, BruhStatus, CREW_PROTOCOL};
use bruhs_models::imports::{
    export_memory_from, looked_in_line, memory_found_line, memory_proposed_line, moved_in_memory,
    persona_lines, AgentScan, FileUse, ImportBatch, ImportJob, ImportJobState, MemorySuggestion,
    PersonaCandidate, PersonaFile, SkipReason, AGENTS_PRIVACY, CHOOSE_FOLDER, COPIED, COPY_PROMPT,
    LOOK_HERE, MAKE_A_BRUH, MAKE_BRUH, MEMORY_WAITS, MOVE_IN_COMMAND, MOVE_IN_COMMAND_HINT,
    MOVE_IN_HINT, MOVE_IN_PROMPT, MOVE_IN_TOO_LONG, NONE_FOUND_HERMES, NONE_FOUND_OPENCLAW,
    NO_BRUHS, PROPOSE_MEMORY, USE_AS_MEMORY,
};
use bruhs_models::logic::portable::ImportSource;
use bruhs_models::reactive::{
    ReactiveAgent, ReactiveChange, ReactiveCommand, ReactiveSnapshot, MAX_TEXT_CHARS,
};
use gpui::{AppContext as _, Entity, FocusHandle, Focusable as _, Task, VisualTestContext};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

fn persona(name: &str, memory: &str) -> PersonaCandidate {
    PersonaCandidate {
        id: format!("id-{name}"),
        source: ImportSource::OpenClaw,
        folder: format!("/home/example/{name}"),
        name: name.into(),
        instructions: format!("## SOUL.md\n\nBe {name}."),
        memory: memory.into(),
        files: vec![
            PersonaFile {
                name: "SOUL.md".into(),
                use_: FileUse::Instructions,
            },
            PersonaFile {
                name: "USER.md".into(),
                use_: FileUse::Memory,
            },
            PersonaFile {
                name: "HEARTBEAT.md".into(),
                use_: FileUse::Skipped {
                    reason: SkipReason::NotImported,
                },
            },
        ],
        clipped: false,
        dropped_identity: vec!["Emoji".into()],
        left_out_logs: 0,
    }
}

fn web_scan() -> AgentScan {
    AgentScan {
        personas: vec![persona("Pinch", "Likes tea."), persona("Scout", "")],
        looked_in: vec!["~/clawd".into(), "~/.openclaw/workspace".into()],
    }
}

/// The server, as Move in's scan seam sees it: the scan for OpenClaw, an
/// empty one for Hermes, and every call it was asked for.
#[derive(Default)]
struct FakeScans {
    openclaw: RefCell<Option<AgentScan>>,
    calls: RefCell<Vec<(ImportSource, Option<String>)>>,
}

fn fake_scan(fake: &Rc<FakeScans>) -> ScanCall {
    let fake = Rc::clone(fake);
    Rc::new(
        move |(source, path): (ImportSource, Option<String>), _cx: &gpui::App| {
            fake.calls.borrow_mut().push((source, path));
            let scan = match source {
                ImportSource::OpenClaw => fake.openclaw.borrow().clone().unwrap_or_default(),
                _ => AgentScan {
                    personas: Vec::new(),
                    looked_in: vec!["~/.hermes".into()],
                },
            };
            Task::ready(Ok(scan))
        },
    )
}

fn bruh(id: &str, name: &str) -> BruhState {
    BruhState {
        agent_id: id.into(),
        name: name.into(),
        look: BruhLook::default(),
        access: BruhAccess::default(),
        effort: None,
        status: BruhStatus::Asleep,
        next_wake_ms: None,
        unread: 0,
        computer: false,
        vault: Vec::new(),
        apps: Vec::new(),
        mail: Vec::new(),
    }
}

fn agent(id: &str, memory: &str) -> ReactiveAgent {
    ReactiveAgent {
        id: id.into(),
        name: id.into(),
        instructions: String::new(),
        memory: memory.into(),
        model: String::new(),
        project_path: String::new(),
        enabled: true,
    }
}

/// A crew of `bruhs`, each with the memory given.
fn crew(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, bruhs: &[(&str, &str, &str)]) {
    let snapshot = ReactiveSnapshot {
        protocol: CREW_PROTOCOL,
        bruhs: bruhs.iter().map(|(id, name, _)| bruh(id, name)).collect(),
        agents: bruhs
            .iter()
            .map(|(id, _, memory)| agent(id, memory))
            .collect(),
        ..ReactiveSnapshot::default()
    };
    let panel = workspace.read_with(cx, |workspace, _| workspace.reactive_automations.clone());
    panel.update(cx, |panel, cx| {
        panel.show_crew_for_test(Vec::new(), CREW_PROTOCOL, cx);
        panel.show_snapshot_for_test(snapshot, cx);
    });
    cx.run_until_parked();
}

fn workspace_window(
    cx: &mut gpui::TestAppContext,
) -> (Entity<Workspace>, &mut gpui::VisualTestContext) {
    cx.update(|cx| {
        bruhs_ui::init(cx);
        bruhs_ui::Theme::change(bruhs_ui::ThemeMode::Dark, None, cx);
        theme::apply_bruhs_theme(cx);
        cx.set_global(DesktopState::new().expect("desktop state should initialize"));
    });
    let cell: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let capture = cell.clone();
    let (_root, cx) = cx.add_window_view(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        *capture.borrow_mut() = Some(workspace.clone());
        bruhs_ui::Root::new(workspace, window, cx)
    });
    let workspace = cell.borrow().clone().expect("workspace entity");
    cx.simulate_resize(gpui::size(gpui::px(1280.0), gpui::px(800.0)));
    (workspace, cx)
}

/// Move in open on `start`, its scans answered by `fake`.
fn move_in<'a>(
    cx: &'a mut gpui::TestAppContext,
    fake: &Rc<FakeScans>,
    start: MoveInStart,
) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (workspace, cx) = workspace_window(cx);
    let scan = fake_scan(fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(scan);
        workspace.open_move_in(start, window, cx);
    });
    cx.run_until_parked();
    (workspace, cx)
}

fn start(tab: MoveInTab) -> MoveInStart {
    MoveInStart {
        tab,
        text: None,
        from: None,
    }
}

fn leak(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn drawn(cx: &mut VisualTestContext, selector: String) -> bool {
    cx.debug_bounds(leak(selector)).is_some()
}

fn focused(cx: &mut VisualTestContext) -> Option<FocusHandle> {
    cx.update(|window, cx| window.focused(cx))
}

/// A key press as the app gets it: gpui clicks a focused button on key up.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    let keystroke = gpui::Keystroke::parse(key).expect("keystroke");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, selector: String) {
    let bounds = cx
        .debug_bounds(leak(selector.clone()))
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

fn sent(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<ReactiveCommand> {
    let panel = workspace.read_with(cx, |workspace, _| workspace.reactive_automations.clone());
    panel.read_with(cx, |panel, _| panel.sent_for_test().to_vec())
}

fn set_answer(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, text: &str) {
    workspace.update_in(cx, |workspace, window, cx| {
        let answer = workspace
            .move_in
            .open
            .as_ref()
            .unwrap()
            .by_prompt
            .answer
            .clone();
        answer.update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    });
    cx.run_until_parked();
}

fn answer(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> String {
    workspace.read_with(cx, |workspace, cx| {
        let open = workspace.move_in.open.as_ref().unwrap();
        open.by_prompt.answer.read(cx).value().to_string()
    })
}

#[gpui::test]
fn the_move_in_tabs_read_from_the_shared_copy(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    *fake.openclaw.borrow_mut() = Some(web_scan());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::OpenClaw));
    let scan = web_scan();
    assert_eq!(*fake.calls.borrow(), [(ImportSource::OpenClaw, None)]);
    assert!(drawn(
        cx,
        format!("desktop-move-in-privacy:{AGENTS_PRIVACY}")
    ));
    assert!(drawn(
        cx,
        format!("desktop-move-in-places:{}", looked_in_line(&scan.looked_in))
    ));
    for name in ["Pinch", "Scout"] {
        assert!(drawn(cx, format!("desktop-move-in-name:{name}")), "{name}");
    }
    assert!(drawn(cx, "desktop-move-in-folder".into()));
    for label in [LOOK_HERE, CHOOSE_FOLDER, MAKE_BRUH] {
        assert!(
            drawn(cx, format!("desktop-move-in-button:{label}")),
            "{label}"
        );
    }
    assert!(!drawn(
        cx,
        format!("desktop-move-in-none:{NONE_FOUND_OPENCLAW}")
    ));
    // The chosen persona's preview: the shared lines.
    let lines = persona_lines(&scan.personas[0]);
    assert!(lines.len() > 2, "{lines:?}");
    for line in &lines {
        assert!(drawn(cx, format!("desktop-move-in-line:{line}")), "{line}");
    }
    // Choosing Scout shows Scout's.
    click(cx, "desktop-move-in-row:Scout".into());
    for line in persona_lines(&scan.personas[1]) {
        assert!(drawn(cx, format!("desktop-move-in-line:{line}")), "{line}");
    }
    assert!(!drawn(cx, format!("desktop-move-in-line:{MEMORY_WAITS}")));

    // Look here scans the typed path.
    workspace.update_in(cx, |workspace, window, cx| {
        let folder = workspace
            .move_in
            .open
            .as_ref()
            .unwrap()
            .openclaw
            .folder
            .clone();
        folder.update(cx, |input, cx| input.set_value("  ~/clawd  ", window, cx));
    });
    click(cx, format!("desktop-move-in-button:{LOOK_HERE}"));
    assert_eq!(
        fake.calls.borrow().last(),
        Some(&(ImportSource::OpenClaw, Some("~/clawd".to_string())))
    );

    // Choose folder: the native picker, folders only, one; then its scan.
    click(cx, format!("desktop-move-in-button:{CHOOSE_FOLDER}"));
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|options| {
        assert!(!options.files && options.directories && !options.multiple);
        assert_eq!(options.prompt.as_deref(), Some(CHOOSE_FOLDER));
        Some(vec![PathBuf::from("/home/example/agents/pinch")])
    });
    cx.run_until_parked();
    assert_eq!(
        fake.calls.borrow().last(),
        Some(&(
            ImportSource::OpenClaw,
            Some("/home/example/agents/pinch".to_string())
        ))
    );
    workspace.read_with(cx, |workspace, cx| {
        let folder = &workspace.move_in.open.as_ref().unwrap().openclaw.folder;
        assert_eq!(
            folder.read(cx).value().as_ref(),
            "/home/example/agents/pinch"
        );
    });

    // Nothing found: the source's own line, and no preview.
    *fake.openclaw.borrow_mut() = Some(AgentScan {
        personas: Vec::new(),
        looked_in: vec!["~/clawd".into()],
    });
    click(cx, format!("desktop-move-in-button:{LOOK_HERE}"));
    assert!(drawn(
        cx,
        format!("desktop-move-in-none:{NONE_FOUND_OPENCLAW}")
    ));
    assert!(!drawn(cx, format!("desktop-move-in-button:{MAKE_BRUH}")));
    click(cx, "desktop-move-in-tab:Hermes Agent".into());
    assert!(drawn(
        cx,
        format!("desktop-move-in-none:{NONE_FOUND_HERMES}")
    ));
    assert_eq!(
        fake.calls.borrow().last(),
        Some(&(ImportSource::Hermes, None)),
        "a tab scans its default places the first time it shows"
    );
}

#[gpui::test]
fn a_failed_scan_shows_the_shared_line(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    let refused: ScanCall = Rc::new(|_, _cx: &gpui::App| {
        Task::ready(Err(bruhs_models::imports::IMPORT_UNREACHABLE.to_string()))
    });
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(refused);
        workspace.open_move_in(start(MoveInTab::OpenClaw), window, cx);
    });
    cx.run_until_parked();
    assert!(drawn(
        cx,
        format!(
            "desktop-move-in-error:{}",
            bruhs_models::imports::IMPORT_UNREACHABLE
        )
    ));
}

#[gpui::test]
fn make_this_bruh_opens_new_bruh_prefilled_and_closes_move_in(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    *fake.openclaw.borrow_mut() = Some(web_scan());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::OpenClaw));
    crew(&workspace, cx, &[]);
    click(cx, format!("desktop-move-in-button:{MAKE_BRUH}"));
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(workspace.top_layer, None, "Move in closed");
        assert!(matches!(workspace.overlay, Some(Overlay::CreateBruh)));
        assert!(workspace.reactive_automations.read(cx).create_bruh_open());
    });
    for line in persona_lines(&web_scan().personas[0]) {
        assert!(
            drawn(cx, format!("desktop-create-bruh-line:{line}")),
            "{line}"
        );
    }
}

#[gpui::test]
fn the_by_prompt_tab_proposes_for_the_chosen_bruh(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::ByPrompt));
    assert!(fake.calls.borrow().is_empty(), "By prompt scans nothing");
    crew(
        &workspace,
        cx,
        &[("a", "Pinch", ""), ("b", "Scout", "Likes tea.")],
    );
    assert!(drawn(
        cx,
        format!("desktop-move-in-prompt:{MOVE_IN_PROMPT}")
    ));
    assert!(drawn(cx, format!("desktop-move-in-button:{COPY_PROMPT}")));
    assert!(drawn(cx, format!("desktop-move-in-hint:{MOVE_IN_HINT}")));
    assert!(drawn(cx, "desktop-move-in-bruh".into()));
    assert!(drawn(cx, "desktop-move-in-answer".into()));
    assert!(drawn(
        cx,
        format!("desktop-move-in-button:{PROPOSE_MEMORY}")
    ));
    assert!(!drawn(cx, format!("desktop-move-in-no-bruhs:{NO_BRUHS}")));

    // Copy prompt: the prompt on the clipboard, and Copied for a moment.
    click(cx, format!("desktop-move-in-button:{COPY_PROMPT}"));
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some(MOVE_IN_PROMPT));
    assert!(drawn(cx, format!("desktop-move-in-button:{COPIED}")));
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(3));
    cx.run_until_parked();
    assert!(drawn(cx, format!("desktop-move-in-button:{COPY_PROMPT}")));

    // Nothing to propose yet: nothing goes.
    click(cx, format!("desktop-move-in-button:{PROPOSE_MEMORY}"));
    assert!(sent(&workspace, cx).is_empty());

    // For bruh lists every bruh; Scout is chosen.
    click(cx, "desktop-move-in-bruh".into());
    for name in ["Pinch", "Scout"] {
        assert!(
            drawn(cx, format!("desktop-move-in-option:{name}")),
            "{name}"
        );
    }
    click(cx, "desktop-move-in-option:Scout".into());
    assert!(
        !drawn(cx, "desktop-move-in-option:Pinch".into()),
        "the list closes"
    );

    set_answer(&workspace, cx, "  Prefers mornings.  ");
    click(cx, format!("desktop-move-in-button:{PROPOSE_MEMORY}"));
    let heading = workspace.read_with(cx, |_, _| {
        bruhs_models::imports::moved_in_from(&chrono::Local::now().format("%Y-%m-%d").to_string())
    });
    let memory = moved_in_memory("Likes tea.", "Prefers mornings.", &heading).unwrap();
    assert_eq!(
        sent(&workspace, cx),
        [ReactiveCommand::Propose {
            change: ReactiveChange::SetMemory {
                agent_id: "b".into(),
                memory,
            }
        }]
    );
    let line = memory_proposed_line("Scout");
    assert!(
        drawn(cx, format!("desktop-move-in-status:{line}")),
        "{line}"
    );
    assert_eq!(answer(&workspace, cx), "", "the area clears");
}

#[gpui::test]
fn a_refused_proposal_keeps_the_answer(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::ByPrompt));
    crew(&workspace, cx, &[("a", "Pinch", "")]);
    let panel = workspace.read_with(cx, |workspace, _| workspace.reactive_automations.clone());
    set_answer(&workspace, cx, "Likes tea.");
    // The proposal goes, and the service hasn't answered yet.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.propose_move_in(window, cx);
        let panel = workspace.reactive_automations.clone();
        panel.update(cx, |panel, _| panel.set_busy_for_test(true));
    });
    cx.run_until_parked();
    // Waiting: another try does nothing.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.propose_move_in(window, cx)
    });
    assert_eq!(sent(&workspace, cx).len(), 1);
    workspace.read_with(cx, |workspace, _| {
        let tab = &workspace.move_in.open.as_ref().unwrap().by_prompt;
        assert_eq!(tab.proposing.as_ref().map(|(id, _)| id.as_str()), Some("a"));
    });
    panel.update(cx, |panel, cx| {
        panel.refuse_for_test("a", "Too many proposals.", cx)
    });
    cx.run_until_parked();
    assert!(drawn(
        cx,
        "desktop-move-in-error:Too many proposals.".into()
    ));
    assert_eq!(answer(&workspace, cx), "Likes tea.", "the text stays");
    // Over the cap: refused with its copy, and the text stays.
    let long = "x".repeat(MAX_TEXT_CHARS);
    set_answer(&workspace, cx, &long);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.propose_move_in(window, cx)
    });
    cx.run_until_parked();
    assert!(drawn(
        cx,
        format!("desktop-move-in-error:{MOVE_IN_TOO_LONG}")
    ));
    assert_eq!(answer(&workspace, cx), long);
}

#[gpui::test]
fn with_no_bruhs_make_a_bruh_carries_the_answer(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    let from = export_memory_from(ImportSource::ClaudeAi);
    let (workspace, cx) = move_in(
        cx,
        &fake,
        MoveInStart {
            tab: MoveInTab::ByPrompt,
            text: Some("Likes tea.".into()),
            from: Some(from.clone()),
        },
    );
    // Before the crew loads, neither shows.
    assert!(!drawn(cx, format!("desktop-move-in-no-bruhs:{NO_BRUHS}")));
    assert!(!drawn(cx, "desktop-move-in-bruh".into()));
    crew(&workspace, cx, &[]);
    assert!(drawn(cx, format!("desktop-move-in-no-bruhs:{NO_BRUHS}")));
    assert!(drawn(cx, format!("desktop-move-in-button:{MAKE_A_BRUH}")));
    assert!(!drawn(
        cx,
        format!("desktop-move-in-button:{PROPOSE_MEMORY}")
    ));
    assert_eq!(
        answer(&workspace, cx),
        "Likes tea.",
        "a prefilled start fills the area"
    );

    // Over the cap first: refused, the text stays, nothing opens.
    let long = "x".repeat(MAX_TEXT_CHARS);
    set_answer(&workspace, cx, &long);
    click(cx, format!("desktop-move-in-button:{MAKE_A_BRUH}"));
    assert!(drawn(
        cx,
        format!("desktop-move-in-error:{MOVE_IN_TOO_LONG}")
    ));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.top_layer, Some(TopLayer::MoveIn));
    });
    assert_eq!(answer(&workspace, cx), long);

    // The answer travels to New bruh as the new bruh's proposal.
    set_answer(&workspace, cx, "Likes tea.");
    click(cx, format!("desktop-move-in-button:{MAKE_A_BRUH}"));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.top_layer, None);
        assert!(matches!(workspace.overlay, Some(Overlay::CreateBruh)));
    });
    assert!(drawn(
        cx,
        format!("desktop-create-bruh-line:{MEMORY_WAITS}")
    ));
    let panel = workspace.read_with(cx, |workspace, _| workspace.reactive_automations.clone());
    panel.update_in(cx, |panel, window, cx| {
        panel.set_create_name_for_test("Ada", window, cx);
        panel.submit_create_bruh(cx);
    });
    let commands = sent(&workspace, cx);
    let Some(ReactiveCommand::CreateBruh { agent, .. }) = commands.last() else {
        panic!("{commands:?}");
    };
    assert_eq!(agent.memory, "");
    let id = agent.id.clone();
    panel.update(cx, |panel, cx| panel.created_bruh_for_test(id.clone(), cx));
    assert_eq!(
        sent(&workspace, cx).last(),
        Some(&ReactiveCommand::Propose {
            change: ReactiveChange::SetMemory {
                agent_id: id,
                memory: format!("## {from}\n\nLikes tea."),
            }
        })
    );
}

#[gpui::test]
fn the_palette_offers_move_in(cx: &mut gpui::TestAppContext) {
    let action = COMMAND_ACTIONS
        .iter()
        .position(|action| action.id == "move-in")
        .expect("the palette lists Move in");
    let import = COMMAND_ACTIONS
        .iter()
        .position(|action| action.id == "import-thread")
        .unwrap();
    assert_eq!(action, import + 1, "after Import thread");
    assert_eq!(COMMAND_ACTIONS[action].label, MOVE_IN_COMMAND);
    assert_eq!(COMMAND_ACTIONS[action].hint, MOVE_IN_COMMAND_HINT);
    assert_eq!(COMMAND_ACTIONS[action].group, "Agent");
    let (workspace, cx) = workspace_window(cx);
    let fake = Rc::new(FakeScans::default());
    let scan = fake_scan(&fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(scan);
        workspace.focus_command_bar(&super::super::FocusCommandBar, window, cx);
        workspace.execute_command_action("move-in", window, cx);
    });
    cx.run_until_parked();
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.top_layer, Some(TopLayer::MoveIn));
        assert_eq!(
            workspace.move_in.open.as_ref().unwrap().tab,
            MoveInTab::OpenClaw
        );
    });
    assert_eq!(*fake.calls.borrow(), [(ImportSource::OpenClaw, None)]);
}

#[gpui::test]
fn use_as_a_bruhs_memory_opens_by_prompt_with_the_text(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    let batch = ImportBatch {
        id: "0123456789abcdef0123456789abcdef".into(),
        memory: vec![MemorySuggestion {
            source: ImportSource::ClaudeAi,
            text: "Likes short answers.".into(),
        }],
        ..ImportBatch::default()
    };
    let discarded = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&discarded);
    let calls = ExportCalls {
        read: Rc::new(|_, _cx: &gpui::App| Task::ready(Err("unused".into()))),
        job: Rc::new(|_, _cx: &gpui::App| Task::ready(Err("unused".into()))),
        thread: Rc::new(|_, _cx: &gpui::App| Task::ready(Err("unused".into()))),
        discard: Rc::new(move |id: String| seen.borrow_mut().push(id)),
    };
    let fake = Rc::new(FakeScans::default());
    let scan = fake_scan(&fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(scan);
        workspace.import_picker.export.calls = Some(calls);
        workspace.open_import_thread(&super::super::ImportThread, window, cx);
        workspace.set_import_tab(ImportTab::Export, window, cx);
        workspace.import_picker.export.job = Some(ImportJob {
            id: batch.id.clone(),
            state: ImportJobState::Ready {
                batch: Box::new(batch.clone()),
            },
        });
        cx.notify();
    });
    cx.run_until_parked();
    let line = memory_found_line(ImportSource::ClaudeAi);
    assert!(
        drawn(cx, format!("desktop-import-export-memory:{line}")),
        "{line}"
    );
    click(cx, format!("desktop-import-export-button:{USE_AS_MEMORY}"));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.top_layer, Some(TopLayer::MoveIn));
        let open = workspace.move_in.open.as_ref().unwrap();
        assert_eq!(open.tab, MoveInTab::ByPrompt);
        assert_eq!(
            open.by_prompt.from.as_deref(),
            Some(export_memory_from(ImportSource::ClaudeAi).as_str())
        );
    });
    assert_eq!(answer(&workspace, cx), "Likes short answers.");
    // The batch went with the Import dialog; the text travelled (F23).
    assert_eq!(*discarded.borrow(), [batch.id]);
}

/// The handle with focus after each Tab, `count` times.
fn tab_through(cx: &mut VisualTestContext, count: usize) -> Vec<Option<FocusHandle>> {
    (0..count)
        .map(|_| {
            cx.simulate_keystrokes("tab");
            cx.run_until_parked();
            focused(cx)
        })
        .collect()
}

#[gpui::test]
fn tab_follows_the_agent_tab_in_order(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    *fake.openclaw.borrow_mut() = Some(web_scan());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::OpenClaw));
    let (tabs, folder, look, choose, rows, make) = workspace.read_with(cx, |workspace, cx| {
        let open = workspace.move_in.open.as_ref().unwrap();
        (
            open.tabs.clone(),
            open.openclaw.folder.focus_handle(cx),
            open.look.clone(),
            open.choose.clone(),
            open.openclaw.rows.clone(),
            open.make.clone(),
        )
    });
    assert!(
        focused(cx).as_ref() == Some(&folder),
        "Folder takes focus first"
    );
    // From the dialog's start: roving Tab lands once on the active tab, then
    // follows the panel's controls in order: Folder, Look here, Choose folder,
    // each persona, then Make this bruh. Arrow keys move between tabs.
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.modal_start_focus_handle.focus(window, cx);
    });
    let mut expected: Vec<FocusHandle> = vec![tabs[0].clone()];
    expected.extend([folder, look, choose]);
    expected.extend(rows.iter().cloned());
    expected.push(make);
    let seen = tab_through(cx, expected.len());
    for (at, (seen, want)) in seen.iter().zip(&expected).enumerate() {
        assert!(seen.as_ref() == Some(want), "stop {at}");
    }
    // And round again.
    let again = tab_through(cx, 1);
    assert!(
        again[0].as_ref() == Some(&tabs[0]),
        "the cycle wraps to the active tab"
    );

    // Right/Left arrows move across tabs and update the selected tab.
    press(cx, "right");
    assert!(
        focused(cx).as_ref() == Some(&tabs[1]),
        "Right moves focus to the next tab"
    );
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.move_in.open.as_ref().unwrap().tab,
            MoveInTab::Hermes
        );
    });
    press(cx, "left");
    assert!(
        focused(cx).as_ref() == Some(&tabs[0]),
        "Left returns focus to the previous tab"
    );
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            workspace.move_in.open.as_ref().unwrap().tab,
            MoveInTab::OpenClaw
        );
    });

    // Enter on a row chooses it.
    cx.update(|window, cx| window.focus(&rows[1], cx));
    press(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.move_in.open.as_ref().unwrap().openclaw.picked, 1);
    });
}

#[gpui::test]
fn escape_closes_move_in_and_returns_focus(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    let opener = cx.update(|window, cx| {
        let handle = cx.focus_handle();
        window.focus(&handle, cx);
        handle
    });
    let fake = Rc::new(FakeScans::default());
    let scan = fake_scan(&fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(scan);
        workspace.open_move_in(start(MoveInTab::OpenClaw), window, cx);
    });
    cx.run_until_parked();
    assert!(focused(cx).as_ref() != Some(&opener));
    press(cx, "escape");
    workspace.read_with(cx, |workspace, _| assert_eq!(workspace.top_layer, None));
    assert!(
        focused(cx).as_ref() == Some(&opener),
        "focus returns to the opener"
    );
    // Closed, Move in keeps nothing: no scans, no crew watch, no text (M1).
    workspace.read_with(cx, |workspace, _| assert!(workspace.move_in.open.is_none()));
}

#[gpui::test]
fn make_this_bruh_waits_for_the_crew(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    *fake.openclaw.borrow_mut() = Some(web_scan());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::OpenClaw));
    // Before the crew loads, Make this bruh keeps its place and does nothing.
    click(cx, format!("desktop-move-in-button:{MAKE_BRUH}"));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(workspace.top_layer, Some(TopLayer::MoveIn));
        assert!(workspace.overlay.is_none());
    });
    crew(&workspace, cx, &[]);
    click(cx, format!("desktop-move-in-button:{MAKE_BRUH}"));
    workspace.read_with(cx, |workspace, _| {
        assert!(matches!(workspace.overlay, Some(Overlay::CreateBruh)));
        assert!(
            workspace.move_in.open.is_none(),
            "Move in went with its text"
        );
    });
}

/// One more frame, as a real window draws after `on_next_frame`.
fn next_frame(cx: &mut VisualTestContext) {
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

/// Whether `inner` is drawn wholly inside the body's visible part.
fn in_view(cx: &mut VisualTestContext, inner: &'static str) -> bool {
    let body = cx.debug_bounds("desktop-move-in-body").expect("body");
    let inner = cx.debug_bounds(inner).unwrap_or_else(|| panic!("{inner}"));
    inner.top() >= body.top() && inner.bottom() <= body.bottom()
}

#[gpui::test]
fn the_focused_field_is_in_view_at_640_by_480(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = workspace_window(cx);
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    let fake = Rc::new(FakeScans::default());
    let scan = fake_scan(&fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(scan);
        workspace.open_move_in(
            MoveInStart {
                tab: MoveInTab::ByPrompt,
                text: Some("Likes tea.".into()),
                from: Some(export_memory_from(ImportSource::ClaudeAi)),
            },
            window,
            cx,
        );
    });
    cx.run_until_parked();
    crew(&workspace, cx, &[("a", "Pinch", "")]);
    // The prompt keeps to its own box.
    let prompt = cx
        .debug_bounds(leak(format!("desktop-move-in-prompt:{MOVE_IN_PROMPT}")))
        .expect("prompt");
    assert!(prompt.size.height <= gpui::px(162.0), "{prompt:?}");
    let answer_focus = workspace.read_with(cx, |workspace, cx| {
        workspace
            .move_in
            .open
            .as_ref()
            .unwrap()
            .by_prompt
            .answer
            .focus_handle(cx)
    });
    assert!(focused(cx).as_ref() == Some(&answer_focus));
    // The pinned Propose memory came with the crew and shrank the body:
    // the next frame (the test platform draws it on request) scrolls again.
    next_frame(cx);
    let body = cx.debug_bounds("desktop-move-in-body");
    let field = cx.debug_bounds("desktop-move-in-answer");
    assert!(
        in_view(cx, "desktop-move-in-answer"),
        "Their answer shows with focus in it: {body:?} {field:?}"
    );
}

#[gpui::test]
fn tab_scrolls_the_last_row_into_view_at_640_by_480(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    let mut many = web_scan();
    many.personas = (0..12).map(|n| persona(&format!("Bruh{n}"), "x")).collect();
    *fake.openclaw.borrow_mut() = Some(many);
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::OpenClaw));
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    cx.run_until_parked();
    let rows = workspace.read_with(cx, |workspace, _| {
        workspace
            .move_in
            .open
            .as_ref()
            .unwrap()
            .openclaw
            .rows
            .clone()
    });
    // From Folder: Look here, Choose folder, then every row.
    let seen = tab_through(cx, 2 + rows.len());
    assert!(seen.last().unwrap().as_ref() == rows.last());
    cx.run_until_parked();
    assert!(
        in_view(cx, "desktop-move-in-row:Bruh11"),
        "the last row shows"
    );
    let list = cx.debug_bounds("desktop-move-in-rows").expect("rows");
    let last = cx.debug_bounds("desktop-move-in-row:Bruh11").unwrap();
    // Each row keeps its two lines: the list scrolls, rows never squash.
    assert!(last.size.height >= gpui::px(48.0), "{last:?}");
    assert!(
        last.top() >= list.top() && last.bottom() <= list.bottom(),
        "{last:?} in {list:?}"
    );
}

#[gpui::test]
fn by_prompt_tabs_to_its_controls_and_cmd_enter_proposes(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    let (workspace, cx) = move_in(cx, &fake, start(MoveInTab::ByPrompt));
    crew(&workspace, cx, &[("a", "Pinch", "")]);
    let (copy, bruh, answer_focus, propose) = workspace.read_with(cx, |workspace, cx| {
        let tab = &workspace.move_in.open.as_ref().unwrap().by_prompt;
        (
            tab.copy.clone(),
            tab.bruh.clone(),
            tab.answer.focus_handle(cx),
            tab.propose.clone(),
        )
    });
    assert!(
        focused(cx).as_ref() == Some(&answer_focus),
        "Their answer takes focus first"
    );
    workspace.update_in(cx, |workspace, window, cx| {
        let tabs = workspace.move_in.open.as_ref().unwrap().tabs.clone();
        tabs[2].focus(window, cx);
    });
    let seen = tab_through(cx, 4);
    for (at, want) in [&copy, &bruh, &answer_focus, &propose]
        .into_iter()
        .enumerate()
    {
        assert!(seen[at].as_ref() == Some(want), "stop {at}");
    }
    // Cmd+Enter in Their answer adds no newline: while the crew is busy
    // nothing is proposed, and the area holds exactly what was typed (M6).
    let cmd_enter = if cfg!(target_os = "macos") {
        "cmd-enter"
    } else {
        "ctrl-enter"
    };
    let panel = workspace.read_with(cx, |workspace, _| workspace.reactive_automations.clone());
    panel.update(cx, |panel, _| panel.set_busy_for_test(true));
    set_answer(&workspace, cx, "Likes tea.");
    cx.update(|window, cx| window.focus(&answer_focus, cx));
    cx.simulate_keystrokes(cmd_enter);
    cx.run_until_parked();
    assert!(sent(&workspace, cx).is_empty());
    assert_eq!(answer(&workspace, cx), "Likes tea.");
    // Then it proposes.
    panel.update(cx, |panel, _| panel.set_busy_for_test(false));
    cx.simulate_keystrokes(cmd_enter);
    cx.run_until_parked();
    let commands = sent(&workspace, cx);
    let [ReactiveCommand::Propose {
        change: ReactiveChange::SetMemory { agent_id, memory },
    }] = commands.as_slice()
    else {
        panic!("{commands:?}");
    };
    assert_eq!(agent_id, "a");
    assert!(memory.ends_with("\n\nLikes tea."), "{memory:?}");
    assert!(drawn(
        cx,
        format!("desktop-move-in-status:{}", memory_proposed_line("Pinch"))
    ));
}

#[gpui::test]
fn the_pinned_actions_stay_in_view_at_640_by_480(cx: &mut gpui::TestAppContext) {
    let fake = Rc::new(FakeScans::default());
    let mut many = web_scan();
    many.personas = (0..12).map(|n| persona(&format!("Bruh{n}"), "x")).collect();
    *fake.openclaw.borrow_mut() = Some(many);
    let (_workspace, cx) = move_in(cx, &fake, start(MoveInTab::OpenClaw));
    cx.simulate_resize(gpui::size(gpui::px(640.0), gpui::px(480.0)));
    cx.run_until_parked();
    let dialog = cx.debug_bounds("desktop-move-in-dialog").expect("dialog");
    let make = cx
        .debug_bounds(leak(format!("desktop-move-in-button:{MAKE_BRUH}")))
        .expect("Make this bruh");
    assert!(dialog.bottom() <= gpui::px(480.0), "{dialog:?}");
    assert!(
        make.bottom() <= dialog.bottom() && make.top() >= dialog.top(),
        "{make:?} inside {dialog:?}"
    );
}

/// The drawn size of `selector`.
fn size_of(cx: &mut VisualTestContext, selector: &'static str) -> gpui::Size<gpui::Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .size
}

#[gpui::test]
fn the_status_and_error_lines_are_live_while_empty(cx: &mut gpui::TestAppContext) {
    use super::super::import_ui::live_line::{
        EXPORT_ERROR_LIVE, EXPORT_STATUS_LIVE, MOVE_IN_ERROR_LIVE, MOVE_IN_STATUS_LIVE,
    };
    use bruhs_models::imports::{IMPORT_UNREACHABLE, NOT_AN_EXPORT};

    // Export file: both lines are in the tree before anything is said,
    // and take no room.
    let (workspace, cx) = workspace_window(cx);
    let calls = ExportCalls {
        read: Rc::new(|_, _cx: &gpui::App| Task::ready(Err("unused".into()))),
        job: Rc::new(|_, _cx: &gpui::App| Task::ready(Err("unused".into()))),
        thread: Rc::new(|_, _cx: &gpui::App| Task::ready(Err("unused".into()))),
        discard: Rc::new(|_: String| {}),
    };
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.import_picker.export.calls = Some(calls);
        workspace.open_import_thread(&super::super::ImportThread, window, cx);
        workspace.set_import_tab(ImportTab::Export, window, cx);
    });
    cx.run_until_parked();
    for live in [EXPORT_STATUS_LIVE, EXPORT_ERROR_LIVE] {
        assert_eq!(size_of(cx, live), gpui::Size::default(), "{live}");
    }
    // A refusal lands in the same node, drawn with its words.
    workspace.update(cx, |workspace, cx| {
        workspace.import_picker.export.error = Some(NOT_AN_EXPORT.to_string());
        cx.notify();
    });
    cx.run_until_parked();
    assert!(size_of(cx, EXPORT_ERROR_LIVE).height > gpui::px(0.0));
    assert!(drawn(
        cx,
        format!("desktop-import-export-error:{NOT_AN_EXPORT}")
    ));
    assert_eq!(size_of(cx, EXPORT_STATUS_LIVE), gpui::Size::default());

    // Move in: an agent tab's error line, then By prompt's two lines.
    let fake = Rc::new(FakeScans::default());
    let scan = fake_scan(&fake);
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.move_in.scan = Some(scan);
        workspace.open_move_in(start(MoveInTab::OpenClaw), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(size_of(cx, MOVE_IN_ERROR_LIVE), gpui::Size::default());
    workspace.update(cx, |workspace, cx| {
        let open = workspace.move_in.open.as_mut().unwrap();
        open.pane_mut(ImportSource::OpenClaw).error = Some(IMPORT_UNREACHABLE.to_string());
        cx.notify();
    });
    cx.run_until_parked();
    assert!(size_of(cx, MOVE_IN_ERROR_LIVE).height > gpui::px(0.0));

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.open_move_in(start(MoveInTab::ByPrompt), window, cx);
    });
    cx.run_until_parked();
    for live in [MOVE_IN_STATUS_LIVE, MOVE_IN_ERROR_LIVE] {
        assert_eq!(size_of(cx, live), gpui::Size::default(), "{live}");
    }
    workspace.update(cx, |workspace, cx| {
        let open = workspace.move_in.open.as_mut().unwrap();
        open.by_prompt.status = Some(memory_proposed_line("Pinch"));
        cx.notify();
    });
    cx.run_until_parked();
    assert!(size_of(cx, MOVE_IN_STATUS_LIVE).height > gpui::px(0.0));
    assert!(drawn(
        cx,
        format!("desktop-move-in-status:{}", memory_proposed_line("Pinch"))
    ));
}
