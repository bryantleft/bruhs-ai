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
