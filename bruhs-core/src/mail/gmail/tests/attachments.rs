use super::*;

fn with_attachment(id: &str, bytes: &[u8], external: bool) -> Value {
    let mut item = message(&format!("{id}-message"), id, "Saved text");
    let headers = item["payload"]["headers"].clone();
    let body = if external {
        json!({"size":bytes.len(),"attachmentId":"provider-attachment"})
    } else {
        json!({"size":bytes.len(),"data":URL_SAFE_NO_PAD.encode(bytes)})
    };
    item["payload"] = json!({"mimeType":"multipart/mixed","headers":headers,"parts":[plain("0","Saved text"),{"partId":"1","mimeType":"application/octet-stream","filename":"設計\r\n<script>.bin","body":body}]});
    item
}

async fn save_attachment(active: &MailSession, message: Value) -> String {
    let thread_id = message["threadId"].as_str().unwrap();
    let record = projection::thread(
        active.authorization().mailbox(),
        "you@example.test",
        serde_json::from_value(json!({"id":thread_id,"messages":[message]})).unwrap(),
    )
    .unwrap();
    let attachment = record.connected_mail.messages[0].attachments[0].id.clone();
    active
        .apply_sync(
            active.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: active.authorization().mailbox().thread(thread_id).unwrap(),
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    attachment
}

#[tokio::test]
async fn explicit_attachment_read_uses_its_cached_canonical_owner_and_preserves_sync_state() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let bytes = b"\x00\xff<html><script>data only</script>";
    let id = save_attachment(&active, with_attachment("thread-a", bytes, true)).await;
    let revision = active.revision().await.unwrap();
    reply(
        &fixture.server,
        "messages/thread-a-message/attachments/provider-attachment",
        json!({"size":bytes.len(),"data":URL_SAFE_NO_PAD.encode(bytes)}),
    )
    .await;
    let downloaded = fixture
        .client
        .attachment(&active, "thread-a", &id)
        .await
        .unwrap();
    assert_eq!(downloaded.bytes, bytes);
    assert_eq!(downloaded.metadata.id, id);
    assert_eq!(downloaded.metadata.file_name, "設計\r\n<script>.bin");
    assert_eq!(active.revision().await.unwrap(), revision);
    drop(active);
    let requests = fixture.server.received_requests().await.unwrap();
    assert_eq!(
        requests.len(),
        1,
        "Only the explicitly requested bytes may be read"
    );
    assert_eq!(
        requests[0].url.path(),
        format!("{API}/messages/thread-a-message/attachments/provider-attachment")
    );
}

#[tokio::test]
async fn embedded_and_empty_attachments_are_read_from_only_the_owning_message() {
    for bytes in [b"inline bytes".as_slice(), b"".as_slice()] {
        let fixture = Fixture::new().await;
        let active = fixture.session().await;
        let message = with_attachment("embedded", bytes, false);
        let id = save_attachment(&active, message.clone()).await;
        reply(&fixture.server, "messages/embedded-message", message).await;
        let downloaded = fixture
            .client
            .attachment(&active, "embedded", &id)
            .await
            .unwrap();
        assert_eq!(downloaded.bytes, bytes);
        drop(active);
        let requests = fixture.server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].url.query(), Some("format=full"));
    }
}

#[tokio::test]
async fn unknown_thread_or_attachment_cannot_read_an_arbitrary_provider_id() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let id = save_attachment(&active, with_attachment("thread-a", b"Saved", true)).await;
    for (thread, attachment) in [
        ("other-thread", id.as_str()),
        ("thread-a", "provider-attachment"),
        ("thread-a", "other-mailbox-canonical-id"),
    ] {
        assert!(matches!(
            fixture.client.attachment(&active, thread, attachment).await,
            Err(GmailError::NotFound)
        ));
    }
    drop(active);
    assert!(fixture.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn truncated_malformed_missing_or_limited_attachment_reads_never_advance_the_cache() {
    for (status, body) in [
        (
            200,
            json!({"size":6,"data":URL_SAFE_NO_PAD.encode(b"Saved")}),
        ),
        (200, json!({"size":5,"data":URL_SAFE_NO_PAD.encode(b"Cut")})),
        (200, json!({"size":5,"data":"***"})),
        (200, json!({"size":5})),
        (404, json!({"error":"PRIVATE-PROVIDER-DIAGNOSTIC"})),
        (429, json!({"error":"PRIVATE-PROVIDER-DIAGNOSTIC"})),
    ] {
        let fixture = Fixture::new().await;
        let active = fixture.session().await;
        let id = save_attachment(&active, with_attachment("thread-a", b"Saved", true)).await;
        let reference = active.authorization().mailbox().thread("thread-a").unwrap();
        let before = fixture
            .runtime
            .cached_thread(reference.clone())
            .await
            .unwrap();
        let revision = active.revision().await.unwrap();
        Mock::given(method("GET"))
            .and(path(format!(
                "{API}/messages/thread-a-message/attachments/provider-attachment"
            )))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&fixture.server)
            .await;
        let result = fixture.client.attachment(&active, "thread-a", &id).await;
        let error = match result {
            Ok(_) => panic!("Invalid attachment must fail"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains("PRIVATE-PROVIDER"));
        assert_eq!(
            fixture.runtime.cached_thread(reference).await.unwrap(),
            before
        );
        assert_eq!(active.revision().await.unwrap(), revision);
        drop(active);
    }
}

#[tokio::test]
async fn mismatched_message_or_ambiguous_embedded_part_is_rejected() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let mut message = with_attachment("embedded", b"Saved", false);
    let id = save_attachment(&active, message.clone()).await;
    message["threadId"] = json!("other-thread");
    reply(
        &fixture.server,
        "messages/embedded-message",
        message.clone(),
    )
    .await;
    assert!(matches!(
        fixture.client.attachment(&active, "embedded", &id).await,
        Err(GmailError::InvalidResponse)
    ));
    fixture.server.reset().await;
    message["threadId"] = json!("embedded");
    let duplicate = message["payload"]["parts"][1].clone();
    message["payload"]["parts"]
        .as_array_mut()
        .unwrap()
        .push(duplicate);
    reply(&fixture.server, "messages/embedded-message", message).await;
    assert!(matches!(
        fixture.client.attachment(&active, "embedded", &id).await,
        Err(GmailError::InvalidResponse)
    ));
    drop(active);
}

#[tokio::test]
async fn a_container_without_a_complete_body_cannot_be_saved_as_an_empty_file() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    let mut message = with_attachment("container", b"", false);
    message["payload"]["parts"][1] = json!({"partId":"1","mimeType":"multipart/mixed","filename":"forwarded.eml","body":{"size":0},"parts":[plain("1.0","Nested message content")]});
    let id = save_attachment(&active, message.clone()).await;
    let revision = active.revision().await.unwrap();
    reply(
        &fixture.server,
        "messages/container-message",
        message.clone(),
    )
    .await;
    assert!(matches!(
        fixture.client.attachment(&active, "container", &id).await,
        Err(GmailError::InvalidResponse)
    ));
    assert_eq!(active.revision().await.unwrap(), revision);
    fixture.server.reset().await;
    message["payload"]["parts"][1]["body"]["data"] = json!("");
    reply(&fixture.server, "messages/container-message", message).await;
    assert!(matches!(
        fixture.client.attachment(&active, "container", &id).await,
        Err(GmailError::InvalidResponse)
    ));
    assert_eq!(active.revision().await.unwrap(), revision);
    drop(active);
}
