use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn prose(key: &str, text: &str) -> TimelineRow {
    TimelineRow::AssistantText {
        key: key.into(),
        message_id: "assistant".into(),
        markdown: text.into(),
        streaming: true,
    }
}

#[test]
fn a_token_keeps_the_key_index_and_unchanged_row_consumers_cached() {
    let owner = Owner::new();
    owner.with(|| {
        let source = ArcRwSignal::new(vec![prose("first", "saved"), prose("live", "partial")]);
        let input = source.clone();
        let rows = Memo::new(move |_| input.get());
        let directory = RowDirectory::new(rows);
        let first = directory.row("first".into());
        let renders = Arc::new(AtomicUsize::new(0));
        let render_count = Arc::clone(&renders);
        let rendered = Memo::new(move |_| {
            render_count.fetch_add(1, Ordering::Relaxed);
            first.get()
        });
        assert_eq!(rendered.get_untracked(), Some(prose("first", "saved")));
        let index_allocation = directory.positions.with_untracked(|index| {
            index
                .keys()
                .find(|key| key.as_str() == "first")
                .expect("first exists")
                .as_ptr()
        });
        source.update(|rows| rows[1] = prose("live", "partial plus token"));
        assert_eq!(directory.keys.get_untracked(), ["first", "live"]);
        assert_eq!(rendered.get_untracked(), Some(prose("first", "saved")));
        assert_eq!(renders.load(Ordering::Relaxed), 1);
        assert_eq!(
            directory.positions.with_untracked(|index| {
                index
                    .keys()
                    .find(|key| key.as_str() == "first")
                    .expect("first exists")
                    .as_ptr()
            }),
            index_allocation
        );
        assert_eq!(
            untrack(|| directory.lookup("live")),
            Some(prose("live", "partial plus token"))
        );
    });
}

#[test]
fn reordering_inserting_removing_and_settling_resolve_rows_by_current_key() {
    let owner = Owner::new();
    owner.with(|| {
        let source = ArcRwSignal::new(vec![prose("first", "saved"), prose("live", "partial")]);
        let input = source.clone();
        let directory = RowDirectory::new(Memo::new(move |_| input.get()));
        let first = directory.row("first".into());
        assert_eq!(first.get_untracked(), Some(prose("first", "saved")));
        source.update(|rows| {
            rows.reverse();
            rows.insert(1, prose("inserted", "inserted full text"));
        });
        assert_eq!(
            directory.keys.get_untracked(),
            ["live", "inserted", "first"]
        );
        assert_eq!(first.get_untracked(), Some(prose("first", "saved")));
        assert_eq!(
            untrack(|| directory.lookup("inserted")),
            Some(prose("inserted", "inserted full text"))
        );
        source.update(|rows| rows.retain(|row| row.key() != "first"));
        assert_eq!(first.get_untracked(), None);
        source.update(|rows| {
            rows.push(prose("first", "restored"));
            if let TimelineRow::AssistantText { streaming, .. } = &mut rows[0] {
                *streaming = false;
            }
        });
        assert_eq!(first.get_untracked(), Some(prose("first", "restored")));
        assert!(matches!(
            untrack(|| directory.lookup("live")),
            Some(TimelineRow::AssistantText {
                streaming: false,
                ..
            })
        ));
        source.set(Vec::new());
        assert!(directory.keys.get_untracked().is_empty());
        assert_eq!(first.get_untracked(), None);
    });
}
