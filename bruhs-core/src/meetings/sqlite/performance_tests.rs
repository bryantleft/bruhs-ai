use super::*;
use crate::meetings::PeopleStore;
use bruhs_models::PersonProfile;
use std::time::{Duration, Instant};

#[tokio::test(flavor = "current_thread")]
async fn held_database_worker_does_not_block_async_timers_or_queued_requests() {
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(
        SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
            .await
            .unwrap(),
    );
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let held = store.clone();
    let worker = tokio::spawn(async move {
        held.call(move |_| {
            ready_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        })
        .await
    });
    ready_rx.await.unwrap();
    let mut queued = tokio::task::JoinSet::new();
    for _ in 0..100 {
        let store = store.clone();
        queued.spawn(async move { store.get_person("absent").await });
    }
    let start = Instant::now();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let elapsed = start.elapsed();
    let still_held = !worker.is_finished();
    release_tx.send(()).unwrap();
    worker.await.unwrap().unwrap();
    while let Some(result) = queued.join_next().await {
        assert!(result.unwrap().unwrap().is_none());
    }
    assert!(still_held, "Database work ran on the async runtime thread");
    assert!(
        elapsed < Duration::from_secs(1),
        "Async timer stalled: {elapsed:?}"
    );
}

#[tokio::test]
#[ignore = "manual local timing fixture; not a hardware-independent performance assertion"]
async fn indexed_contact_operations_with_twenty_thousand_voice_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("private.sqlite3"))
        .await
        .unwrap();
    store
        .call(|connection| {
            write(connection, |tx| {
                for index in 0..20_000 {
                    let mut person = PersonProfile::new(format!("Contact {index:05}"));
                    person.id = format!("person-{index:05}");
                    person.voice_embedding = Some(vec![0.0625; 256]);
                    contacts::save(tx, &person)?;
                }
                Ok(())
            })
        })
        .await
        .unwrap();
    let mut scan_ms = Vec::new();
    let mut indexed_ms = Vec::new();
    let mut commit_ms = Vec::new();
    for index in 0..30 {
        let start = Instant::now();
        let old = store
            .list_people()
            .await
            .unwrap()
            .into_iter()
            .find(|p| p.id == "person-19999")
            .unwrap();
        scan_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        assert_eq!(store.get_person("person-19999").await.unwrap(), Some(old));
        indexed_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        let start = Instant::now();
        store
            .rename_person(&"person-19999".parse().unwrap(), format!("Renamed {index}"))
            .await
            .unwrap();
        commit_ms.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    fn distribution(mut values: Vec<f64>) -> serde_json::Value {
        values.sort_by(f64::total_cmp);
        serde_json::json!({"p50_ms":values[14], "p95_ms":values[28], "p99_ms":values[29], "samples":values.len()})
    }
    println!(
        "SQLITE_CONTACT_TIMING {}",
        serde_json::json!({
            "profiles":20_000, "voice_dimensions":256,
            "old_full_scan":distribution(scan_ms),
            "indexed_lookup":distribution(indexed_ms),
            "durable_rename":distribution(commit_ms)
        })
    );
}
