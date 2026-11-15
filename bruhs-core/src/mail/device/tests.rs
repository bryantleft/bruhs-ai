use super::*;
fn scope() -> DeviceDraftScope {
    DeviceDraftScope {
        origin: "http://127.0.0.1:3001".into(),
        host_id: "host-a".into(),
        mailbox_id: "mailbox-a".into(),
    }
}
fn draft() -> DeviceDraft {
    DeviceDraft::new(
        scope(),
        ServiceProvider::Gmail,
        None,
        MailComposeContent {
            to: "Incomplete <".into(),
            body: "private text".into(),
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn text_and_file_commit_together_survive_reopen_and_reject_stale_instance() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("device.sqlite3");
    let store = MailDeviceStore::open(&path).await.unwrap();
    let mut value = draft();
    let client = value.client_id.clone();
    value.files.push(DeviceDraftFile {
        id: "file-a".into(),
        file_name: "résumé.bin".into(),
        size_bytes: 4,
        server: None,
    });
    let saved = store
        .save(
            value.clone(),
            None,
            vec![DeviceFileBytes {
                id: "file-a".into(),
                bytes: vec![0, 1, 2, 255],
            }],
        )
        .await
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(
        store
            .file(scope(), client.clone(), "file-a".into())
            .await
            .unwrap(),
        vec![0, 1, 2, 255]
    );
    let mut stale = value.clone();
    stale.content.body = "older must not replace".into();
    assert!(matches!(
        store.save(stale, None, vec![]).await,
        Err(DeviceDraftError::Conflict)
    ));
    assert_eq!(
        store
            .load(scope(), client.clone())
            .await
            .unwrap()
            .unwrap()
            .value
            .content
            .body,
        "private text"
    );
    assert!(matches!(
        MailDeviceStore::open(&path).await,
        Err(DeviceDraftError::Busy)
    ));
    drop(store);
    let reopened = MailDeviceStore::open(&path).await.unwrap();
    assert_eq!(
        reopened
            .load(scope(), client.clone())
            .await
            .unwrap()
            .unwrap()
            .value
            .content
            .to,
        "Incomplete <"
    );
    assert_eq!(
        reopened
            .file(scope(), client.clone(), "file-a".into())
            .await
            .unwrap(),
        vec![0, 1, 2, 255]
    );
    let mut foreign = scope();
    foreign.host_id = "host-b".into();
    assert!(reopened
        .load(foreign, client.clone())
        .await
        .unwrap()
        .is_none());
    let mut foreign = scope();
    foreign.origin = "http://127.0.0.1:3002".into();
    assert!(reopened.load(foreign, client).await.unwrap().is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn missing_or_changed_file_rolls_back_the_entire_text_transaction() {
    let directory = tempfile::tempdir().unwrap();
    let store = MailDeviceStore::open(directory.path().join("device.sqlite3"))
        .await
        .unwrap();
    let mut value = draft();
    let client = value.client_id.clone();
    let initial = store.save(value.clone(), None, vec![]).await.unwrap();
    value.content.body = "must roll back".into();
    value.files.push(DeviceDraftFile {
        id: "file-a".into(),
        file_name: "file.bin".into(),
        size_bytes: 3,
        server: None,
    });
    assert!(matches!(
        store
            .save(value.clone(), Some(initial.revision), vec![])
            .await,
        Err(DeviceDraftError::Corrupt)
    ));
    assert_eq!(
        store
            .load(scope(), client.clone())
            .await
            .unwrap()
            .unwrap()
            .value
            .content
            .body,
        "private text"
    );
    let saved = store
        .save(
            value.clone(),
            Some(initial.revision),
            vec![DeviceFileBytes {
                id: "file-a".into(),
                bytes: vec![1, 2, 3],
            }],
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .save(
                value,
                Some(saved.revision),
                vec![DeviceFileBytes {
                    id: "file-a".into(),
                    bytes: vec![3, 2, 1]
                }]
            )
            .await,
        Err(DeviceDraftError::Conflict)
    ));
    assert_eq!(
        store.file(scope(), client, "file-a".into()).await.unwrap(),
        vec![1, 2, 3]
    );
}

#[tokio::test]
async fn list_is_thin_and_removed_files_and_accepted_work_disappear_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let store = MailDeviceStore::open(directory.path().join("device.sqlite3"))
        .await
        .unwrap();
    let mut value = draft();
    let client = value.client_id.clone();
    value.files.push(DeviceDraftFile {
        id: "file-a".into(),
        file_name: "file.bin".into(),
        size_bytes: 1,
        server: None,
    });
    let saved = store
        .save(
            value.clone(),
            None,
            vec![DeviceFileBytes {
                id: "file-a".into(),
                bytes: vec![42],
            }],
        )
        .await
        .unwrap();
    let rows = store.list(scope(), 0).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].recipients, "Incomplete <");
    value.files.clear();
    value.dirty = false;
    value.server_revision = 4;
    value.latest_send = Some(MailSendStatus {
        host_id: scope().host_id,
        mailbox_id: scope().mailbox_id,
        operation_id: "op".into(),
        draft_id: "draft".into(),
        draft_revision: 4,
        state: MailDispatchState::Accepted,
        provider_message_id: None,
        provider_thread_id: None,
        problem: None,
    });
    store
        .save(value, Some(saved.revision), vec![])
        .await
        .unwrap();
    assert!(store.list(scope(), 0).await.unwrap().is_empty());
    assert!(store.file(scope(), client, "file-a".into()).await.is_err());
}

#[tokio::test]
async fn clean_online_refresh_preserves_original_device_file_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let store = MailDeviceStore::open(directory.path().join("device.sqlite3"))
        .await
        .unwrap();
    let mut value = draft();
    let server = MailDraftFile {
        id: "mail-draft-attachment-v1-server-id".into(),
        file_name: "private.bin".into(),
        size_bytes: 3,
        sha256: digest(&[0, 42, 255]),
    };
    value.files.push(DeviceDraftFile {
        id: "chooser-uuid".into(),
        file_name: server.file_name.clone(),
        size_bytes: 3,
        server: Some(server.clone()),
    });
    value.content.attachments = vec![server.id.clone()];
    value.server_revision = 7;
    value.dirty = false;
    let saved = store
        .save(
            value.clone(),
            None,
            vec![DeviceFileBytes {
                id: "chooser-uuid".into(),
                bytes: vec![0, 42, 255],
            }],
        )
        .await
        .unwrap();
    let refreshed = value
        .refresh_from_server(MailComposeDraft {
            host_id: scope().host_id,
            mailbox_id: scope().mailbox_id,
            client_id: value.client_id.clone(),
            draft_id: "canonical-draft".into(),
            revision: 7,
            reply: None,
            content: value.content.clone(),
            uploads: vec![server],
            latest_send: None,
        })
        .unwrap();
    assert_eq!(refreshed.files[0].id, "chooser-uuid");
    store
        .save(refreshed, Some(saved.revision), vec![])
        .await
        .unwrap();
    assert_eq!(
        store
            .file(scope(), value.client_id, "chooser-uuid".into())
            .await
            .unwrap(),
        vec![0, 42, 255]
    );
}

#[tokio::test]
async fn final_shutdown_snapshot_cannot_interleave_with_an_older_queued_writer() {
    let directory = tempfile::tempdir().unwrap();
    let store = MailDeviceStore::open(directory.path().join("device.sqlite3"))
        .await
        .unwrap();
    let initial = store.save(draft(), None, vec![]).await.unwrap();
    let mut older = initial.value.clone();
    older.content.body = "older in-flight snapshot".into();
    let mut latest = initial.value.clone();
    latest.content.body = "last typed input".into();
    let client = latest.client_id.clone();
    latest.files.push(DeviceDraftFile {
        id: "final-file".into(),
        file_name: "final.bin".into(),
        size_bytes: 3,
        server: None,
    });
    // Tokio's fair mutex gives the final operation the first slot, with the
    // old writer already waiting behind it. A split load/save would release
    // between those operations and let the old snapshot invalidate its CAS.
    let blocker = store.db.clone().lock_owned().await;
    let (final_started, final_ready) = tokio::sync::oneshot::channel();
    let final_store = store.clone();
    let final_write = tokio::spawn(async move {
        let _ = final_started.send(());
        final_store
            .save_on_shutdown(
                latest,
                vec![DeviceFileBytes {
                    id: "final-file".into(),
                    bytes: vec![0, 42, 255],
                }],
            )
            .await
    });
    final_ready.await.unwrap();
    tokio::task::yield_now().await;
    let (old_started, old_ready) = tokio::sync::oneshot::channel();
    let old_store = store.clone();
    let old_write = tokio::spawn(async move {
        let _ = old_started.send(());
        old_store.save(older, Some(initial.revision), vec![]).await
    });
    old_ready.await.unwrap();
    tokio::task::yield_now().await;
    drop(blocker);
    final_write.await.unwrap().unwrap();
    assert!(matches!(
        old_write.await.unwrap(),
        Err(DeviceDraftError::Conflict)
    ));
    assert_eq!(
        store
            .load(scope(), client.clone())
            .await
            .unwrap()
            .unwrap()
            .value
            .content
            .body,
        "last typed input"
    );
    assert_eq!(
        store
            .file(scope(), client, "final-file".into())
            .await
            .unwrap(),
        vec![0, 42, 255]
    );
}
