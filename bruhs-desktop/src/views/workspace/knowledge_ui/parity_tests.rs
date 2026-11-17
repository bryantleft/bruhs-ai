//! Web parity for the Knowledge library: search and the document preview.

use super::super::performance_tests::fixture;
use super::super::{element_key, AppView, Workspace};
use bruhs_models::{KnowledgeDocument, SourceClip};
use gpui::{Entity, VisualTestContext};

fn document_card(id: &str) -> &'static str {
    Box::leak(format!("desktop-knowledge-document-{}", element_key(id)).into_boxed_str())
}

fn source_card(id: &str) -> &'static str {
    Box::leak(format!("desktop-knowledge-source-{}", element_key(id)).into_boxed_str())
}

fn shown(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

/// A full key press. GPUI clicks a focused control on key up, which
/// `simulate_keystrokes` never sends.
fn press_key(cx: &mut VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key).expect("test keystroke should parse");
    cx.simulate_event(gpui::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(gpui::KeyUpEvent { keystroke });
    cx.run_until_parked();
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

/// Types `query` into the library's search box, as a person would.
fn search(workspace: &Entity<Workspace>, cx: &mut VisualTestContext, query: &str) {
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.knowledge.search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
    });
    cx.simulate_input(query);
    cx.run_until_parked();
}

/// PM-12: like the web's `KnowledgeLibrary`, one search box filters both
/// sections by full document text, tags, source excerpts and URLs, case
/// folded; a new query starts each section on its first page; a saved edit is
/// searchable at once.
#[gpui::test]
fn knowledge_search_filters_documents_and_sources_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update(cx, |workspace, cx| {
        let mut roadmap = KnowledgeDocument::new("Roadmap", "# Plan\nShip the Q3 plan.", 2);
        roadmap.id = "roadmap".into();
        roadmap.tags = vec!["product".into()];
        let mut notes = KnowledgeDocument::new("Notes", "Deep CAFÉ details", 1);
        notes.id = "notes".into();
        let mut survey = SourceClip::new(
            "Survey",
            "https://example.invalid/Survey",
            "Customer interviews",
            2,
        );
        survey.id = "survey".into();
        let mut paper = SourceClip::new("Paper", "https://research.example/p", "Findings", 1);
        paper.id = "paper".into();
        paper.tags = vec!["mixed".into()];
        workspace.knowledge_documents = vec![notes, roadmap];
        workspace.source_clips = vec![paper, survey];
        workspace.active_view = AppView::Knowledge;
        cx.notify();
    });
    cx.run_until_parked();
    assert!(shown(cx, "desktop-knowledge-search"));
    for card in [
        document_card("roadmap"),
        document_card("notes"),
        source_card("survey"),
        source_card("paper"),
    ] {
        assert!(shown(cx, card), "a blank search shows every record");
    }
    // Latest change first, as on the web.
    let roadmap = cx.debug_bounds(document_card("roadmap")).unwrap();
    let notes = cx.debug_bounds(document_card("notes")).unwrap();
    assert!(roadmap.top() < notes.top() || roadmap.left() < notes.left());

    workspace.update(cx, |workspace, _| {
        workspace.library_pages.documents = 3;
        workspace.library_pages.sources = 2;
    });
    search(&workspace, cx, "café");
    assert!(shown(cx, document_card("notes")), "body text, case folded");
    assert!(!shown(cx, document_card("roadmap")));
    assert!(!shown(cx, source_card("survey")));
    assert!(!shown(cx, source_card("paper")));
    workspace.read_with(cx, |workspace, _| {
        assert_eq!(
            (
                workspace.library_pages.documents,
                workspace.library_pages.sources
            ),
            (0, 0),
            "a new query starts both sections on their first page"
        );
    });

    search(&workspace, cx, "PRODUCT");
    assert!(shown(cx, document_card("roadmap")), "a tag");
    assert!(!shown(cx, document_card("notes")));

    search(&workspace, cx, "example.invalid/survey");
    assert!(shown(cx, source_card("survey")), "a source's URL");
    assert!(!shown(cx, source_card("paper")));
    assert!(!shown(cx, document_card("roadmap")));

    search(&workspace, cx, "Mixed");
    assert!(shown(cx, source_card("paper")), "a source's tag");
    assert!(!shown(cx, source_card("survey")));

    // An edit saved through the workspace is searchable without a reload.
    workspace.update(cx, |workspace, cx| {
        workspace.knowledge_documents[0].body_markdown = "Mixed signals".into();
        workspace.persist_and_notify(cx);
    });
    cx.run_until_parked();
    assert!(shown(cx, document_card("notes")), "the saved edit matches");

    search(&workspace, cx, "");
    for card in [
        document_card("roadmap"),
        document_card("notes"),
        source_card("survey"),
        source_card("paper"),
    ] {
        assert!(shown(cx, card), "clearing the search shows every record");
    }
}

/// PM-13: like the web's `DocumentEditor`, Preview shows the body's rendered
/// Markdown, unsaved edits included, in place of the editor; Edit returns to
/// the editor; every document opens in the editor.
#[gpui::test]
fn document_preview_shows_the_unsaved_body_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let mut plan = KnowledgeDocument::new("Plan", "# Saved", 2);
        plan.id = "plan".into();
        let mut other = KnowledgeDocument::new("Other", "Other body", 1);
        other.id = "other".into();
        workspace.knowledge_documents = vec![plan, other];
        workspace.select_knowledge_document("plan", window, cx);
        workspace.knowledge_body_input.update(cx, |input, cx| {
            input.set_value("# Draft\n\nSome **bold** text.", window, cx);
        });
    });
    cx.run_until_parked();
    assert!(shown(cx, "desktop-knowledge-body"));
    assert!(!shown(cx, "desktop-knowledge-document-preview"));

    click(cx, "desktop-knowledge-preview");
    let preview = cx
        .debug_bounds("desktop-knowledge-document-preview")
        .expect("Preview shows the rendered body");
    assert!(
        preview.size.height >= gpui::px(320.0),
        "web's 320px minimum"
    );
    assert!(
        !shown(cx, "desktop-knowledge-body"),
        "the preview replaces the editor"
    );
    workspace.read_with(cx, |workspace, cx| {
        assert!(workspace.knowledge.preview);
        assert_eq!(
            workspace.knowledge_body_input.read(cx).value().as_str(),
            "# Draft\n\nSome **bold** text.",
            "the unsaved draft is kept"
        );
        assert_eq!(
            workspace.knowledge_documents[0].body_markdown, "# Saved",
            "previewing does not save"
        );
    });

    click(cx, "desktop-knowledge-preview");
    assert!(shown(cx, "desktop-knowledge-body"), "Edit returns");
    assert!(!shown(cx, "desktop-knowledge-document-preview"));

    click(cx, "desktop-knowledge-preview");
    workspace.update_in(cx, |workspace, window, cx| {
        workspace.select_knowledge_document("other", window, cx);
    });
    cx.run_until_parked();
    assert!(
        shown(cx, "desktop-knowledge-body"),
        "another document opens in the editor"
    );
    workspace.read_with(cx, |workspace, _| assert!(!workspace.knowledge.preview));

    // From the title field, Shift-Tab reaches Preview: Save waits for a
    // change, and like a disabled web button it is skipped. One Enter
    // toggles Preview once.
    cx.simulate_keystrokes("shift-tab");
    press_key(cx, "enter");
    workspace.read_with(cx, |workspace, _| {
        assert!(
            workspace.knowledge.preview,
            "Enter on Preview shows it once"
        );
    });
    assert!(shown(cx, "desktop-knowledge-document-preview"));
}

/// Like the web, Ask agent is on a document's page, not the library, and it
/// puts the document's prompt in the composer.
#[gpui::test]
fn ask_agent_is_on_the_document_page_like_web(cx: &mut gpui::TestAppContext) {
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let mut plan = KnowledgeDocument::new("Plan", "Ship the beta", 1);
        plan.id = "plan".into();
        workspace.knowledge_documents = vec![plan];
        workspace.open_knowledge_view(window, cx);
    });
    cx.run_until_parked();
    assert!(shown(cx, document_card("plan")));
    assert!(
        !shown(cx, "desktop-knowledge-ask-agent"),
        "the library offers no Ask agent"
    );

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.select_knowledge_document("plan", window, cx);
    });
    cx.run_until_parked();
    click(cx, "desktop-knowledge-ask-agent");
    workspace.read_with(cx, |workspace, cx| {
        let prompt = bruhs_models::document_agent_prompt(
            &workspace.knowledge_documents[0],
            &workspace.source_clips,
        );
        assert_eq!(workspace.active_view, AppView::Chat);
        let chat = workspace.active_thread().expect("a thread").chat.read(cx);
        assert_eq!(chat.input_state.read(cx).value().as_str(), prompt);
    });
}

/// PM-43: like the web, the body field is at least 384px tall, the line
/// under the tags says whether the document is saved, Save waits for a
/// change, and an empty Backlinks panel says so.
#[gpui::test]
fn document_page_shows_its_body_and_save_state_like_web(cx: &mut gpui::TestAppContext) {
    use bruhs_models::logic::knowledge_library::document_copy;
    let (workspace, cx) = fixture(cx);
    workspace.update_in(cx, |workspace, window, cx| {
        let mut plan = KnowledgeDocument::new("Plan", "# Saved\n\nLine two", 1);
        plan.id = "plan".into();
        workspace.knowledge_documents = vec![plan];
        workspace.select_knowledge_document("plan", window, cx);
    });
    cx.run_until_parked();
    let body = cx
        .debug_bounds("desktop-knowledge-body")
        .expect("the body field is drawn");
    assert!(
        body.size.height >= gpui::px(384.0),
        "web's 384px body field, not one line: {body:?}"
    );
    let line = |workspace: &gpui::Entity<Workspace>, cx: &mut VisualTestContext| {
        workspace.read_with(cx, |workspace, cx| workspace.document_save_line(cx))
    };
    assert_eq!(line(&workspace, cx), document_copy::SAVED);
    workspace.read_with(cx, |workspace, cx| {
        assert!(
            !workspace.current_document_dirty(cx),
            "Save waits for a change"
        );
    });
    assert!(cx.debug_bounds("desktop-knowledge-save-line").is_some());

    workspace.update_in(cx, |workspace, window, cx| {
        workspace.knowledge_body_input.update(cx, |input, cx| {
            input.set_value("# Saved\n\nLine two, edited", window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(line(&workspace, cx), document_copy::UNSAVED);
    click(cx, "desktop-knowledge-save");
    workspace.read_with(cx, |workspace, cx| {
        assert_eq!(
            workspace.knowledge_documents[0].body_markdown,
            "# Saved\n\nLine two, edited"
        );
        assert!(!workspace.current_document_dirty(cx));
    });
    assert_eq!(line(&workspace, cx), document_copy::SAVED);
}
