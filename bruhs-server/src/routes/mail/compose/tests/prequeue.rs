use super::*;

#[tokio::test]
async fn prequeue_revoked_consent_keeps_the_http_error_and_recovers_terminal_status() {
    for provider in [ServiceProvider::Gmail, ServiceProvider::Outlook] {
        let f = Fixture::new(provider).await;
        println!("SLOT3_FIXTURE_PORTS {} {}", f.base, f.provider.uri());
        let mut draft = f.saved("prequeue-consent").await;
        let file = f
            .client
            .mail_draft_upload(
                &f.host(),
                &f.mailbox,
                "prequeue-consent",
                "retained-file",
                "retained.bin",
                b"retained private bytes".to_vec(),
            )
            .await
            .unwrap();
        draft.content.attachments.push(file.id.clone());
        draft = f
            .client
            .mail_draft_save(
                &f.mailbox,
                "prequeue-consent",
                &MailDraftSave {
                    host_id: f.host(),
                    expected_revision: draft.revision,
                    content: draft.content,
                },
            )
            .await
            .unwrap();
        let review = f.review("prequeue-consent", draft.revision).await;
        let request = f.request(&review);
        f.connections.disconnect(provider).await.unwrap();
        let response = f
            .http
            .post(f.route("/prequeue-consent/send"))
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(f.posts().await.is_empty());
        let status = f
            .client
            .mail_send_status(&f.host(), &f.mailbox, &request.operation_id)
            .await
            .expect(
                "a known pre-queue denial needs a durable status, not a ghost pending operation",
            );
        assert_eq!(status.state, MailDispatchState::Rejected);
        assert_eq!(status.problem.unwrap().kind, MailSendProblemKind::SignIn);
        let restored = f
            .client
            .mail_draft(&f.host(), &f.mailbox, "prequeue-consent")
            .await
            .unwrap();
        assert_eq!(restored.content, draft.content);
        assert_eq!(restored.uploads.len(), 1);
        assert_eq!(restored.uploads[0].sha256, file.sha256);
        assert_eq!(restored.uploads[0].size_bytes, file.size_bytes);
        assert!(f.posts().await.is_empty());
    }
}

#[tokio::test]
async fn prequeue_profile_401_is_recoverable_even_when_the_error_body_is_lost() {
    for provider in [ServiceProvider::Gmail, ServiceProvider::Outlook] {
        let f = Fixture::new(provider).await;
        println!("SLOT3_FIXTURE_PORTS {} {}", f.base, f.provider.uri());
        let draft = f.saved("prequeue-profile").await;
        let review = f.review("prequeue-profile", draft.revision).await;
        let request = f.request(&review);
        f.provider.reset().await;
        Mock::given(method("GET"))
            .and(path(if provider == ServiceProvider::Gmail {
                "/gmail/v1/users/me/profile"
            } else {
                "/v1.0/me"
            }))
            .respond_with(ResponseTemplate::new(401))
            .mount(&f.provider)
            .await;
        let response = f
            .http
            .post(f.route("/prequeue-profile/send"))
            .json(&request)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        drop(response);
        assert!(f.posts().await.is_empty());
        let status = f
            .client
            .mail_send_status(&f.host(), &f.mailbox, &request.operation_id)
            .await
            .expect("the existing status GET must recover a profile denial before queueing");
        assert_eq!(status.state, MailDispatchState::Rejected);
        assert_eq!(status.problem.unwrap().kind, MailSendProblemKind::SignIn);
        assert!(f.posts().await.is_empty());
    }
}
