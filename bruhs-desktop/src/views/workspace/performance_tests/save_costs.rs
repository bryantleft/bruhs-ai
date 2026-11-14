//! Snapshot ownership and acknowledged, private temporary-file save diagnostics.
use super::super::{
    desktop_state_backup_path, save_desktop_state, AtomicOrdering, DiskVersion,
    PersistenceCoordinator,
};
use super::*;
use crate::views::meetings_hub::PeopleSnapshot;
use bruhs_models::{
    FinanceTransaction, LifeContext, ObservationKind, PersonObservation, PersonProfile,
};
use std::sync::{Arc, Mutex};

fn people(count: usize) -> Vec<Arc<PersonProfile>> {
    (0..count)
        .map(|index| {
            let mut person = PersonProfile::new(format!("Synthetic person {index}"));
            person.id = format!("person-{index}");
            person.created_at = 1;
            person.updated_at = 1;
            person.aliases = vec![format!("Alias {index}")];
            person.voice_embedding = Some(vec![0.125; 64]);
            person.observations = (0..20)
                .map(|observation| PersonObservation {
                    at_meeting_id: format!("meeting-{observation}"),
                    kind: ObservationKind::Topic,
                    text: format!(
                        "Person {index}, observation {observation}: {}",
                        "Complete saved context, café and 👩🏽‍💻. ".repeat(10)
                    ),
                    created_at: i64::from(observation),
                })
                .collect();
            Arc::new(person)
        })
        .collect()
}

fn life(count: usize) -> LifeContext {
    let mut context = LifeContext::default();
    context.finance.transactions = (0..count)
        .map(|index| {
            let mut transaction = FinanceTransaction::manual(
                format!("Synthetic merchant {index}"),
                "Synthetic category",
                i64::try_from(index % 20_000).unwrap() - 10_000,
                1_600_000_000_000 + i64::try_from(index).unwrap(),
            );
            transaction.id = format!("transaction-{index}");
            transaction.note = Some("Complete local memo and receipt context. ".repeat(6));
            Arc::new(transaction)
        })
        .collect();
    context
}

#[gpui::test]
#[ignore = "manual optimized snapshot and acknowledged filesystem timing; cargo test -p bruhs-desktop --bin bruhs-desktop --release acknowledged_save_action_cost -- --ignored --nocapture"]
fn acknowledged_save_action_cost(cx: &mut TestAppContext) {
    let (workspace, cx) = fixture(cx);
    let root = tempfile::tempdir().unwrap();
    for (label, contacts, transactions) in [
        ("people-1000", 1_000, 0),
        ("people-5000", 5_000, 0),
        ("finance-10000", 0, 10_000),
        ("finance-100000", 0, 100_000),
        ("people-finance-mixed", 2_500, 50_000),
    ] {
        workspace.update(cx, |workspace, cx| {
            workspace.sidebar_visible = false;
            workspace.active_view = AppView::Calendar;
            workspace.life_context = life(transactions);
            workspace.meetings_hub.update(cx, |hub, _| {
                hub.restore_people(PeopleSnapshot {
                    records: people(contacts),
                    ..Default::default()
                });
            });
        });
        cx.run_until_parked();
        let path = root.path().join(format!("{label}.json"));
        let coordinator = Arc::new(PersistenceCoordinator {
            write_lock: Mutex::new(DiskVersion::Missing),
            ..Default::default()
        });
        let mut snapshots = Vec::new();
        let mut encodings = Vec::new();
        let mut worker = Vec::new();
        let mut previous = None;
        let mut encoded_len = 0;
        for generation in 1..=9_u64 {
            let started = Instant::now();
            let mut state = workspace.read_with(cx, |workspace, cx| workspace.snapshot(cx));
            let snapshot = started.elapsed();
            state.current_model = Some(format!("synthetic-generation-{generation}"));
            let started = Instant::now();
            let expected = serde_json::to_vec(&state).unwrap();
            let encoding = started.elapsed();
            encoded_len = expected.len();
            coordinator
                .latest_generation
                .store(generation, AtomicOrdering::Release);
            let writer_path = path.clone();
            let writer_coordinator = Arc::clone(&coordinator);
            let (duration, result) = std::thread::spawn(move || {
                let started = Instant::now();
                let result =
                    save_desktop_state(Some(writer_path), state, writer_coordinator, generation);
                (started.elapsed(), result)
            })
            .join()
            .unwrap();
            result.unwrap();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                expected,
                "acknowledged bytes are complete"
            );
            if let Some(previous) = previous {
                assert_eq!(
                    std::fs::read(desktop_state_backup_path(&path)).unwrap(),
                    previous,
                    "backup is the exact previously acknowledged state"
                );
            }
            previous = Some(expected);
            if generation > 2 {
                snapshots.push(snapshot.as_secs_f64() * 1000.0);
                encodings.push(encoding.as_secs_f64() * 1000.0);
                worker.push(duration.as_secs_f64() * 1000.0);
            }
        }
        let mut stale = Vec::new();
        for _ in 0..7 {
            let state = workspace.read_with(cx, |workspace, cx| workspace.snapshot(cx));
            let started = Instant::now();
            let result = save_desktop_state(Some(path.clone()), state, Arc::clone(&coordinator), 1);
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            result.unwrap();
            stale.push(elapsed);
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            previous.unwrap(),
            "obsolete saves leave current bytes unchanged"
        );
        println!(
            "ACKNOWLEDGED_SAVE_COST {}",
            serde_json::json!({
                "fixture": label, "people": contacts, "transactions": transactions, "bytes": encoded_len,
                "snapshot_ms": snapshots, "encoding_ms": encodings, "worker_ms": worker,
                "stale_current_ms": stale,
            })
        );
    }
}
