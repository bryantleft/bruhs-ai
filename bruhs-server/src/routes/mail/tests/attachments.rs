use super::*;

#[tokio::test]
async fn attachment_response_is_explicit_binary_private_and_bound_to_the_current_host_account_and_cached_part(
) {
    let fixture = Fixture::new().await;
    let bytes = b"\x00\xff<script>downloaded bytes</script>";
    reply(
        &fixture,
        "profile",
        json!({"emailAddress":"you@example.test","historyId":"100"}),
    )
    .await;
    reply(&fixture, "threads", json!({"threads":[{"id":"thread-a"}]})).await;
    reply(&fixture,"threads/thread-a",json!({"id":"thread-a","messages":[{"id":"message-a","threadId":"thread-a","internalDate":"1794652800000","labelIds":["INBOX"],"payload":{"mimeType":"multipart/mixed","headers":[{"name":"From","value":"Sender <sender@example.test>"}],"parts":[{"partId":"0","mimeType":"text/plain","body":{"size":4,"data":URL_SAFE_NO_PAD.encode("Body")}}, {"partId":"1","mimeType":"text/html","filename":"evil\r\n<script>.html","body":{"size":bytes.len(),"attachmentId":"provider-attachment"}}]}}]})).await;
    let synced = fixture.client.sync_gmail(&fixture.request()).await.unwrap();
    let record = fixture
        .client
        .mail_thread(
            fixture.runtime.host().as_str(),
            &synced.mailbox_id,
            "thread-a",
        )
        .await
        .unwrap();
    let attachment = &record.connected_mail.messages[0].attachments[0];
    let route = format!(
        "{}/api/mail/mailboxes/{}/threads/thread-a/attachments/{}",
        fixture.base, synced.mailbox_id, attachment.id
    );
    fixture.provider.reset().await;
    reply(
        &fixture,
        "messages/message-a/attachments/provider-attachment",
        json!({"size":bytes.len(),"data":URL_SAFE_NO_PAD.encode(bytes)}),
    )
    .await;
    for (host, mailbox, connection, status) in [
        (
            "other-host",
            synced.mailbox_id.as_str(),
            "synthetic-connection",
            StatusCode::CONFLICT,
        ),
        (
            fixture.runtime.host().as_str(),
            "other-account",
            "synthetic-connection",
            StatusCode::CONFLICT,
        ),
        (
            fixture.runtime.host().as_str(),
            synced.mailbox_id.as_str(),
            "old-generation",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let guarded = route.replace(
            &format!("/mailboxes/{}/", synced.mailbox_id),
            &format!("/mailboxes/{mailbox}/"),
        );
        let response = fixture
            .http
            .get(guarded)
            .query(&[("host_id", host), ("connection_id", connection)])
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
    assert!(
        fixture
            .provider
            .received_requests()
            .await
            .unwrap()
            .is_empty(),
        "Host/account/generation guards precede any provider read"
    );
    let response = fixture
        .http
        .get(&route)
        .query(&[
            ("host_id", fixture.runtime.host().as_str()),
            ("connection_id", "synthetic-connection"),
        ])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/octet-stream");
    assert_eq!(response.headers()[CONTENT_DISPOSITION], "attachment");
    assert_eq!(response.headers()[X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(response.headers()[axum::http::header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("no-store"));
    assert_eq!(response.bytes().await.unwrap().as_ref(), bytes);
    assert_eq!(fixture.provider.received_requests().await.unwrap().len(), 1);
    let saved = fixture._directory.path().join("selected-attachment.bin");
    std::fs::write(&saved, b"Previously saved bytes").unwrap();
    fixture
        .client
        .save_mail_attachment(
            fixture.runtime.host().as_str(),
            &synced.mailbox_id,
            "thread-a",
            attachment,
            &saved,
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read(&saved).unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&saved).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    fixture.provider.reset().await;
    reply(
        &fixture,
        "messages/message-a/attachments/provider-attachment",
        json!({"size":bytes.len(),"data":URL_SAFE_NO_PAD.encode("Truncated")}),
    )
    .await;
    assert!(fixture
        .client
        .save_mail_attachment(
            fixture.runtime.host().as_str(),
            &synced.mailbox_id,
            "thread-a",
            attachment,
            &saved
        )
        .await
        .is_err());
    assert_eq!(
        std::fs::read(&saved).unwrap(),
        bytes,
        "A failed retry must preserve the selected file's previous contents"
    );
    assert!(std::fs::read_dir(fixture._directory.path())
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
}
