use super::*;

#[test]
fn bounded_pages_reach_every_record_in_stable_latest_first_order() {
    let documents = (0..101)
        .map(|n| {
            let mut document = KnowledgeDocument::new(format!("Document {n}"), "Body", n);
            document.id = format!("doc-{n:03}");
            document
        })
        .collect::<Vec<_>>();
    let index = Index::documents(&documents);
    let matches = index.matching("");
    assert_eq!(matches.len(), 101);
    assert_eq!(index.page(&matches, 0).len(), PAGE_SIZE);
    assert_eq!(index.page(&matches, usize::MAX).len(), 5);
    let ids = (0..=last_page(matches.len()))
        .flat_map(|page| index.page(&matches, page))
        .map(|card| card.id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 101);
    assert_eq!(ids[0], "doc-100");
    assert_eq!(ids[100], "doc-000");
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        101
    );
    assert!(index
        .page(&index.matching("missing"), usize::MAX)
        .is_empty());
}

#[test]
fn document_search_includes_full_body_and_casefolds_each_tag() {
    let body = format!("# Preview\n{}\nDeep Café 世界", "x".repeat(1000));
    let mut document = KnowledgeDocument::new("Title", body, 1);
    document.tags = vec!["MiXeDTag".into()];
    let index = Index::documents(&[document]);
    for query in ["  TITLE ", "DEEP CAFÉ 世界", "mixedtag", "MIXEDTAG"] {
        assert_eq!(index.matching(query), vec![0]);
    }
    assert!(index.matching("Title Preview").is_empty());
    assert_eq!(index.page(&[0], 0)[0].preview, "Preview");
}

#[test]
fn source_search_keeps_full_excerpt_and_url_beyond_card_preview() {
    let mut source = SourceClip::new(
        "Title",
        "https://example.invalid/path/Needle",
        format!("{} DeepExcerpt", "é".repeat(200)),
        1,
    );
    source.tags = vec!["MiXeDTag".into()];
    let index = Index::sources(&[source]);
    for query in ["EXAMPLE.INVALID/PATH/NEEDLE", "DEEPEXcerpt", "mixedtag"] {
        assert_eq!(index.matching(query), vec![0]);
    }
    let card = &index.page(&[0], 0)[0];
    assert_eq!(card.preview.chars().count(), 160);
    assert_eq!(card.detail, "example.invalid");
}

#[test]
fn tied_timestamps_sort_by_identity_and_narrowed_pages_clamp() {
    let mut a = KnowledgeDocument::new("A", "Body", 1);
    a.id = "a".into();
    let mut b = KnowledgeDocument::new("B", "Body", 1);
    b.id = "b".into();
    let index = Index::documents(&[b, a]);
    assert_eq!(index.page(&index.matching("body"), 0)[0].id, "a");
    assert_eq!(index.page(&index.matching("A"), 100)[0].id, "a");
    assert_eq!(last_page(0), 0);
    assert_eq!(last_page(PAGE_SIZE), 0);
    assert_eq!(last_page(PAGE_SIZE + 1), 1);
}
