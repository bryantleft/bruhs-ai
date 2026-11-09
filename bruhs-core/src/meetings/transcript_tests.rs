use super::*;

fn segment(id: &str) -> TranscriptSegment {
    TranscriptSegment {
        id: id.into(),
        meeting_id: "meeting".into(),
        speaker_id: Some("them".into()),
        start_ms: 0,
        end_ms: 1000,
        text: "Final speech".into(),
        confidence: 1.0,
        partial: false,
        speaker_confidence: None,
    }
}

async fn meeting(store: &dyn MeetingStore, id: &str) {
    let mut meeting = Meeting::new("Transcript fixture", 1.0);
    meeting.id = id.into();
    store.create_meeting(meeting).await.unwrap();
}

async fn snapshot_contract(store: &dyn MeetingStore) {
    meeting(store, "meeting").await;
    let initial = vec![segment("a")];
    assert_eq!(
        store
            .cache_transcript("meeting", vec![], initial.clone())
            .await
            .unwrap(),
        initial
    );
    let mut diarized = initial.clone();
    diarized[0].speaker_id = Some("them_1".into());
    diarized[0].speaker_confidence = Some(0.8);
    assert_eq!(
        store
            .cache_transcript("meeting", initial.clone(), diarized.clone())
            .await
            .unwrap(),
        diarized
    );
    assert!(
        store
            .cache_transcript("meeting", initial.clone(), initial)
            .await
            .is_err(),
        "stale capture reply must fail its baseline check"
    );
    assert!(
        store
            .cache_transcript("meeting", diarized.clone(), vec![])
            .await
            .is_err(),
        "a shorter snapshot must not delete finalized words"
    );
    for mutation in [
        "text",
        "foreign",
        "duplicate",
        "confidence",
        "partial",
        "time",
    ] {
        let mut invalid = diarized.clone();
        match mutation {
            "text" => invalid[0].text = "Changed final words".into(),
            "foreign" => invalid[0].meeting_id = "another-meeting".into(),
            "duplicate" => invalid.push(invalid[0].clone()),
            "confidence" => invalid[0].confidence = f32::NAN,
            "partial" => invalid[0].partial = true,
            "time" => invalid[0].end_ms = 0,
            _ => unreachable!(),
        }
        assert!(
            store
                .cache_transcript("meeting", diarized.clone(), invalid)
                .await
                .is_err(),
            "invalid {mutation} accepted"
        );
        assert_eq!(
            store.list_segments("meeting").await.unwrap(),
            diarized,
            "invalid {mutation} changed saved data"
        );
    }
    let mut incoming = diarized.clone();
    incoming.push(segment("b"));
    incoming.reverse();
    let saved = store
        .cache_transcript("meeting", diarized, incoming)
        .await
        .unwrap();
    assert_eq!(
        saved.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(
        store
            .cache_transcript("meeting", saved.clone(), saved.clone())
            .await
            .unwrap(),
        saved
    );
    meeting(store, "other").await;
    let mut foreign = segment("a");
    foreign.meeting_id = "other".into();
    assert!(store
        .cache_transcript("other", vec![], vec![foreign])
        .await
        .is_err());
    assert!(store.list_segments("other").await.unwrap().is_empty());
    assert_eq!(store.list_segments("meeting").await.unwrap(), saved);
    assert!(store
        .cache_transcript("missing", vec![], vec![])
        .await
        .is_err());
}

async fn removal_contract(store: &dyn MeetingStore) {
    meeting(store, "meeting").await;
    let kept = segment("kept");
    let mut secret = segment("secret");
    secret.text = "password 1234".into();
    secret.start_ms = 2_000;
    secret.end_ms = 3_000;
    store.append_segment(kept.clone()).await.unwrap();
    store.append_segment(secret.clone()).await.unwrap();
    assert!(!store.segment_removed("meeting", "secret").await.unwrap());
    store.remove_segment("meeting", "secret").await.unwrap();
    store.remove_segment("meeting", "secret").await.unwrap();
    assert!(store.segment_removed("meeting", "secret").await.unwrap());
    assert_eq!(
        store.removed_segment_ids("meeting").await.unwrap(),
        vec!["secret".to_string()]
    );
    assert_eq!(
        store.list_segments("meeting").await.unwrap(),
        vec![kept.clone()]
    );
    let mut replay = secret.clone();
    replay.text = "password 1234 again".into();
    store.append_segment(replay).await.unwrap();
    assert_eq!(
        store.list_segments("meeting").await.unwrap(),
        vec![kept.clone()]
    );
    let saved = store
        .cache_transcript(
            "meeting",
            vec![kept.clone(), secret.clone()],
            vec![secret.clone(), kept.clone()],
        )
        .await
        .unwrap();
    assert_eq!(saved, vec![kept.clone()]);
    assert_eq!(store.list_segments("meeting").await.unwrap(), vec![kept]);
    assert!(matches!(
        store.remove_segment("missing", "secret").await,
        Err(MeetingStoreError::MeetingNotFound { .. })
    ));
    assert!(matches!(
        store.removed_segment_ids("missing").await,
        Err(MeetingStoreError::MeetingNotFound { .. })
    ));
    assert!(!store.segment_removed("missing", "secret").await.unwrap());
    for id in ["", " ", "bad\nid"] {
        assert!(
            matches!(
                store.remove_segment("meeting", id).await,
                Err(MeetingStoreError::InvalidInput(_))
            ),
            "{id:?} should be rejected"
        );
    }
}

#[tokio::test]
async fn removed_transcript_chunks_stay_hidden_from_replays_and_snapshots() {
    removal_contract(&InMemoryMeetingStore::new()).await;
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("transcripts.sqlite3"))
        .await
        .unwrap();
    removal_contract(&store).await;
}

#[tokio::test]
async fn memory_transcript_snapshots_preserve_final_words_and_reject_stale_or_malformed_replies() {
    snapshot_contract(&InMemoryMeetingStore::new()).await;
}

#[tokio::test]
async fn sqlite_transcript_snapshots_preserve_final_words_and_reject_stale_or_malformed_replies() {
    let directory = tempfile::tempdir().unwrap();
    let store = SqliteMeetingStore::open(directory.path().join("transcripts.sqlite3"))
        .await
        .unwrap();
    snapshot_contract(&store).await;
}

#[tokio::test]
async fn sqlite_failed_snapshot_rolls_back_every_annotation_and_new_segment() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("transcripts.sqlite3");
    let store = SqliteMeetingStore::open(&path).await.unwrap();
    meeting(&store, "meeting").await;
    let initial = vec![segment("a")];
    store
        .cache_transcript("meeting", vec![], initial.clone())
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_snapshot BEFORE INSERT ON segments WHEN NEW.id='b' BEGIN SELECT RAISE(ABORT,'injected snapshot failure'); END;").unwrap();
    let mut next = initial.clone();
    next[0].speaker_id = Some("them_2".into());
    next.push(segment("b"));
    assert!(store
        .cache_transcript("meeting", initial.clone(), next)
        .await
        .is_err());
    assert_eq!(store.list_segments("meeting").await.unwrap(), initial);
    drop(store);
    let reopened = SqliteMeetingStore::open(&path).await.unwrap();
    assert_eq!(reopened.list_segments("meeting").await.unwrap(), initial);
}

#[tokio::test]
async fn canonical_snapshot_can_replace_temporary_partial_id_without_losing_final_segments() {
    for sqlite in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let store: Box<dyn MeetingStore> = if sqlite {
            Box::new(
                SqliteMeetingStore::open(directory.path().join("transcripts.sqlite3"))
                    .await
                    .unwrap(),
            )
        } else {
            Box::new(InMemoryMeetingStore::new())
        };
        meeting(store.as_ref(), "meeting").await;
        let mut partial = segment("temporary");
        partial.partial = true;
        store.append_segment(partial.clone()).await.unwrap();
        store
            .cache_transcript("meeting", vec![partial], vec![segment("final")])
            .await
            .unwrap();
        assert_eq!(
            store.list_segments("meeting").await.unwrap(),
            vec![segment("final")]
        );
    }
}
