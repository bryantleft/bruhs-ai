use std::hint::black_box;
use std::time::{Duration, Instant};

use bruhs_models::{
    suggested_follow_ups, InboxConversation, InboxMessage, InboxParticipant, InboxSource,
    RelationshipItem, RelationshipItemKind, RelationshipProfile,
};

const RECORD_COUNT: usize = 10_000;
const SAMPLE_COUNT: usize = 101;

/// Bench indices stay below the record count, far inside `i64`.
fn int(index: usize) -> i64 {
    i64::try_from(index).expect("bench index fits in i64")
}

fn fixtures() -> (Vec<InboxConversation>, Vec<RelationshipProfile>) {
    let conversations = (0..RECORD_COUNT)
        .map(|index| {
            let person_id = format!("person-{index}");
            let mut conversation = InboxConversation::new(
                InboxSource::Email,
                format!("Conversation {index}"),
                vec![InboxParticipant {
                    id: format!("participant-{index}"),
                    display_name: format!("Person {index}"),
                    address: format!("person-{index}@example.com"),
                    person_id: Some(person_id),
                }],
                int(index),
            );
            conversation.receive(InboxMessage::incoming("Sender", "Following up", int(index)));
            conversation
        })
        .collect();
    let relationships = (0..RECORD_COUNT)
        .map(|index| {
            let mut profile = RelationshipProfile::new(format!("person-{index}"), int(index));
            if index % 10 == 0 {
                let mut item = RelationshipItem::new(
                    RelationshipItemKind::Commitment,
                    "Send the promised update",
                    int(index),
                );
                item.due_at_ms = Some(int(index));
                profile.add_item(item, int(index));
            }
            profile
        })
        .collect();
    (conversations, relationships)
}

fn percentile_95(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
}

fn asserted_budget() -> Option<Duration> {
    let args = std::env::args().collect::<Vec<_>>();
    let position = args.iter().position(|arg| arg == "--assert-ms")?;
    Some(Duration::from_millis(args.get(position + 1)?.parse().ok()?))
}

fn main() {
    let (conversations, relationships) = fixtures();
    let now = int(RECORD_COUNT) + 1;
    black_box(suggested_follow_ups(&relationships, &conversations, now));

    let follow_up_samples = (0..SAMPLE_COUNT)
        .map(|_| {
            let started = Instant::now();
            black_box(suggested_follow_ups(
                black_box(&relationships),
                black_box(&conversations),
                black_box(now),
            ));
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let follow_up_p95 = percentile_95(follow_up_samples);

    let mutation_samples = (0..SAMPLE_COUNT)
        .map(|index| {
            let mut conversation = conversations[index].clone();
            let started = Instant::now();
            conversation.mark_read(true);
            conversation.snooze_until(Some(now + 86_400_000));
            conversation.upsert_draft("Fast local reply", now);
            black_box(conversation);
            started.elapsed()
        })
        .collect::<Vec<_>>();
    let mutation_p95 = percentile_95(mutation_samples);

    println!(
        "inbox follow-ups: {RECORD_COUNT} conversations + relationships, p95 {:.3} ms; optimistic triage p95 {:.3} ms",
        follow_up_p95.as_secs_f64() * 1_000.0,
        mutation_p95.as_secs_f64() * 1_000.0,
    );
    if let Some(budget) = asserted_budget() {
        assert!(
            follow_up_p95 <= budget,
            "follow-up p95 exceeded the frame budget"
        );
        assert!(
            mutation_p95 <= Duration::from_millis(50),
            "triage p95 exceeded 50 ms"
        );
    }
}
