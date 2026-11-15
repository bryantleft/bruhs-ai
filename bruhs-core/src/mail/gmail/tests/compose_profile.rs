use super::*;

#[tokio::test]
async fn compose_sender_comes_from_the_leased_gmail_profile_and_never_client_content() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    reply(
        &fixture.server,
        "profile",
        json!({"emailAddress":"verified@example.test","historyId":"123"}),
    )
    .await;
    let sender = fixture.client.sender(&active).await.unwrap();
    drop(active);
    assert_eq!(sender.address().address, "verified@example.test");
    assert!(sender.address().name.is_empty());
    assert!(!format!("{sender:?}").contains("verified@example.test"));
    let requests = fixture.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method.as_str(), "GET");
    assert_eq!(requests[0].url.path(), format!("{API}/profile"));
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap(),
        format!("Bearer {TOKEN}")
    );
}

#[tokio::test]
async fn malformed_profile_sender_is_rejected_without_echoing_the_address_or_headers() {
    let fixture = Fixture::new().await;
    let active = fixture.session().await;
    reply(&fixture.server,"profile",json!({"emailAddress":"sender@example.test\r\nBcc: PRIVATE@example.test","historyId":"123"})).await;
    let error = fixture.client.sender(&active).await.unwrap_err();
    drop(active);
    assert!(matches!(error, GmailError::InvalidResponse));
    assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    assert_eq!(fixture.server.received_requests().await.unwrap().len(), 1);
}
