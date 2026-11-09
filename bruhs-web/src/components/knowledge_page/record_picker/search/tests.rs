use super::*;

#[test]
fn full_list_navigation_clamps_and_crosses_page_boundaries() {
    assert_eq!(navigate(23, 10_000, "ArrowDown"), Some(24));
    assert_eq!(navigate(24, 10_000, "ArrowUp"), Some(23));
    assert_eq!(navigate(0, 10_000, "End"), Some(9_999));
    assert_eq!(navigate(9_999, 10_000, "Home"), Some(0));
    assert_eq!(navigate(9_999, 10_000, "PageDown"), Some(9_999));
    assert_eq!(navigate(2, 10_000, "PageUp"), Some(0));
    assert_eq!(navigate(24, 10_000, "PageDown"), Some(48));
    assert_eq!(navigate(usize::MAX, 0, "ArrowDown"), Some(0));
    assert_eq!(navigate(0, 1, "ArrowUp"), Some(0));
    assert_eq!(navigate(0, 10_000, "Tab"), None);
}

#[test]
fn bounded_pages_reach_all_ten_thousand_options_in_input_order() {
    let index = Index::new(
        &(0..10_000)
            .map(|i| DropdownOption::new(i.to_string(), format!("Record {i}")))
            .collect::<Vec<_>>(),
    );
    let matches = index.matching("");
    assert_eq!(matches.len(), 10_000);
    let ids = (0..10_000)
        .step_by(PAGE_SIZE)
        .flat_map(|position| {
            let page = index.page(&matches, position);
            assert!(page.len() <= PAGE_SIZE);
            page
        })
        .map(|(_, option)| option.value)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 10_000);
    assert_eq!(ids[0], "0");
    assert_eq!(ids[9_999], "9999");
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        10_000
    );
    assert_eq!(index.page(&matches, usize::MAX).len(), 16);
}

#[test]
fn casefolded_full_label_and_group_match_without_cross_field_false_matches() {
    let index = Index::new(&[
        DropdownOption::new("exact-deep-id", format!("{} Café 世界", "x".repeat(500)))
            .in_group("MiXeD"),
        DropdownOption::new("other", "Other").in_group("Tasks"),
    ]);
    assert_eq!(index.matching("  CAFÉ 世界  "), vec![0]);
    assert_eq!(index.matching("mixed"), vec![0]);
    assert_eq!(index.matching("TASKS"), vec![1]);
    assert!(index.matching("Other Tasks").is_empty());
    assert_eq!(
        index.value(&index.matching("世界"), 0),
        Some("exact-deep-id".into())
    );
}

#[test]
fn empty_and_narrowed_queries_never_choose_hidden_or_missing_records() {
    let index = Index::new(&[DropdownOption::new("only", "Only")]);
    let empty = index.matching("missing");
    assert!(index.page(&empty, usize::MAX).is_empty());
    assert_eq!(index.value(&empty, 0), None);
    assert_eq!(index.value(&index.matching("only"), 1), None);
    assert_eq!(
        index.page(&index.matching("only"), 9_999)[0].1.value,
        "only"
    );
}
