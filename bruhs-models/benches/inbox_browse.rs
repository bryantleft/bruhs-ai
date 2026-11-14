//! Cold index and per-query costs, including the replaced full-history path.
use bruhs_models::{InboxConversation, InboxDirectory, InboxFilter, InboxMessage, InboxSource};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

fn measure<T>(label: &str, mut run: impl FnMut() -> T) {
    black_box(run());
    let mut samples = (0..51)
        .map(|_| {
            let start = Instant::now();
            black_box(run());
            start.elapsed()
        })
        .collect::<Vec<Duration>>();
    samples.sort_unstable();
    println!(
        "{label}: p50 {:.3} ms, p95 {:.3} ms",
        samples[25].as_secs_f64() * 1000.0,
        samples[48].as_secs_f64() * 1000.0
    );
}

fn main() {
    let body = "Review launch research and write a follow-up. ".repeat(70);
    let conversations = (0..10_000)
        .map(|index| {
            let mut item = InboxConversation::new(
                InboxSource::Email,
                format!("Conversation {index}"),
                vec![],
                index,
            );
            item.id = format!("conversation-{index}");
            item.receive(InboxMessage::incoming("Person", &body, index));
            item
        })
        .collect::<Vec<_>>();
    // The previous web library lowercased and cloned every matching history,
    // then sorted it, on each search keystroke before constructing all rows.
    measure("legacy inbox query / 10,000 histories", || {
        let mut items = conversations
            .iter()
            .filter(|item| {
                item.messages
                    .iter()
                    .any(|message| message.body.to_lowercase().contains(black_box("launch")))
            })
            .cloned()
            .collect::<Vec<_>>();
        items.sort_unstable_by_key(|item| std::cmp::Reverse(item.updated_at_ms));
        items
    });
    measure("cold inbox directory / 10,000 histories", || {
        InboxDirectory::new(black_box(&conversations))
    });
    let directory = InboxDirectory::new(&conversations);
    measure("indexed inbox query and page / 10,000 histories", || {
        let matches = directory.matching(black_box("launch"), InboxFilter::Inbox, 20_000);
        directory.page(&matches, 0)
    });
}
