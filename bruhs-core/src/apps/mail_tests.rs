//! `send_mail` through the compose path against a synthetic Gmail
//! connection and the shared fake transport: nothing reaches a provider.

use crate::apps::{AppHttp, AppsError, AppsHost};
use crate::mail::compose::test_support::{ConnectedMail, FakeTransport, Mode};
use crate::mail::gmail::GmailClient;
use crate::mail::outlook::OutlookClient;
use crate::mail::{MailRuntime, SyncBatch, ThreadWrite};
use crate::secrets::{SecretManager, SecretStore};
use bruhs_models::connections::ServiceProvider;
use bruhs_models::crew::{
    MailSendInput, MAIL_MAY_HAVE_SENT, MAIL_NOT_GRANTED, MAIL_NOT_SENT, MAIL_QUEUED,
    MAIL_REPLY_UNKNOWN, MAIL_SENT, MAIL_UNAVAILABLE,
};
use bruhs_models::mail::{
    ConnectedMailMetadata, ConnectedMailThread, MailAddress, MailDispatchState, MailEnvelope,
    MailMessageDetail, MailReplyTarget, MailSendProblemKind, MailSendStatus,
};
use bruhs_models::{InboxConversation, InboxMessage, InboxSource};
use std::sync::Arc;
use std::time::Duration;

/// A host whose mail is `mail`'s cache and connection, sending through
/// `transport`. No app call is made, so the app client points nowhere.
fn mail_host(mail: &ConnectedMail, transport: Arc<FakeTransport>) -> AppsHost {
    bare(
        SecretManager::new(SecretStore::in_memory()).with_connections(mail.connections.clone()),
        Some(mail.runtime.clone()),
    )
    .with_mail_transport(transport)
}

fn bare(vault: SecretManager, mail: Option<MailRuntime>) -> AppsHost {
    let origin = "http://127.0.0.1:9";
    let http = AppHttp::with_upstream("127.0.0.1:9".parse().unwrap()).unwrap();
    let gmail = Arc::new(GmailClient::for_test_origin(origin).unwrap());
    let outlook = Arc::new(OutlookClient::for_test_origin(origin).unwrap());
    AppsHost::new(vault, http, mail, gmail, outlook)
}

fn lunch() -> MailSendInput {
    MailSendInput {
        from: None,
        to: "ann@x.test".into(),
        cc: String::new(),
        bcc: String::new(),
        subject: Some("Lunch".into()),
        body: "See you at 12.".into(),
        reply: None,
    }
}

fn parsed(bytes: &[u8]) -> mail_parser::Message<'_> {
    mail_parser::MessageParser::default().parse(bytes).unwrap()
}

/// The editable drafts left in the mailbox.
async fn drafts(mail: &ConnectedMail) -> usize {
    mail.runtime
        .cached_drafts(mail.mailbox.clone())
        .await
        .unwrap()
        .len()
}

/// Caches a thread whose one message the mailbox received, and returns a
/// reply target naming that message.
async fn seed_thread(mail: &ConnectedMail) -> MailReplyTarget {
    let session = mail.session().await;
    let mailbox = session.authorization().mailbox().clone();
    let remote = mailbox.thread("source-thread").unwrap();
    let id = mailbox.message("source-message").unwrap().canonical_id;
    let mut message = InboxMessage::incoming("Source", "Source body", 1);
    message.id = id.clone();
    message.external_id = Some("source-message".into());
    let mut conversation = InboxConversation::new(InboxSource::Email, "Plans", vec![], 1);
    conversation.id = remote.canonical_id.clone();
    conversation.external_id = Some(remote.external_id.clone());
    conversation.receive(message);
    let address = |value: &str| MailAddress {
        name: String::new(),
        address: value.into(),
    };
    let record = ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Gmail,
            mailbox_id: mailbox.as_str().into(),
            external_thread_id: remote.external_id.clone(),
            messages: vec![MailMessageDetail {
                id: id.clone(),
                external_id: "source-message".into(),
                label_ids: vec![],
                attachments: vec![],
                body_from_html: false,
                envelope: MailEnvelope {
                    from: vec![address("bo@x.test")],
                    to: vec![address("sender@example.test")],
                    subject: "Plans for Friday".into(),
                    message_id: Some("source@x.test".into()),
                    ..Default::default()
                },
            }],
        },
    };
    session
        .apply_sync(
            session.revision().await.unwrap(),
            SyncBatch {
                upserts: vec![ThreadWrite {
                    reference: remote,
                    payload: serde_json::to_string(&record).unwrap(),
                }],
                confirmed_deleted: vec![],
                cursor: None,
            },
        )
        .await
        .unwrap();
    drop(session);
    MailReplyTarget {
        external_thread_id: "source-thread".into(),
        message_id: id,
    }
}

#[tokio::test]
async fn a_granted_mailbox_sends_through_the_journal() {
    let mail = ConnectedMail::gmail().await;
    let transport = FakeTransport::new(Mode::Accepted, false);
    let host = mail_host(&mail, transport.clone());

    let outcome = host
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap();
    assert_eq!(outcome.state, MailDispatchState::Accepted);
    assert_eq!(outcome.message, MAIL_SENT);
    assert!(outcome.operation_id.is_some());
    assert_eq!(transport.sends(), 1);
    let sent = transport.sent();
    let message = parsed(&sent[0]);
    assert_eq!(message.subject(), Some("Lunch"));
    assert_eq!(message.body_text(0).as_deref(), Some("See you at 12."));
    let journal = mail.send_operations();
    assert_eq!(journal.len(), 1, "{journal:?}");
    assert_eq!(journal[0].1, "accepted");
    // A sent draft leaves the editable drafts.
    assert_eq!(drafts(&mail).await, 0);
    // Each send is a new draft and a new operation.
    host.send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap();
    let journal = mail.send_operations();
    assert_eq!(journal.len(), 2);
    assert_ne!(journal[0].0, journal[1].0);
    let json = serde_json::to_value(&outcome).unwrap();
    assert_eq!(json["state"], "accepted");
    assert_eq!(json["message"], MAIL_SENT);
}

#[tokio::test]
async fn a_reply_binds_to_its_thread() {
    let mail = ConnectedMail::gmail().await;
    let transport = FakeTransport::new(Mode::Accepted, false);
    let host = mail_host(&mail, transport.clone());
    let target = seed_thread(&mail).await;
    let card_subject = host.reply_subject(mail.mailbox.as_str(), &target).await;
    assert_eq!(card_subject.as_deref(), Some("Plans for Friday"));

    let reply = MailSendInput {
        subject: None,
        body: "Friday works.".into(),
        reply: Some(target.clone()),
        ..lunch()
    };
    let outcome = host.send_mail(mail.mailbox.as_str(), reply).await.unwrap();
    assert_eq!(outcome.message, MAIL_SENT);
    let sent = transport.sent();
    let message = parsed(&sent[0]);
    // The card's subject is exactly the one the email carries: the
    // compose path sends a reply with the source's subject, no "Re: " added.
    assert_eq!(message.subject(), card_subject.as_deref());
    assert!(format!("{:?}", message.in_reply_to()).contains("source@x.test"));
    assert!(format!("{:?}", message.references()).contains("source@x.test"));
    // The bruh's To stands, not the thread's sender.
    assert!(format!("{:?}", message.to()).contains("ann@x.test"));
    assert!(!format!("{:?}", message.to()).contains("bo@x.test"));

    // A reply to a message the store doesn't hold sends nothing and leaves
    // no draft.
    let unknown = MailSendInput {
        subject: None,
        reply: Some(MailReplyTarget {
            message_id: "no-such-message".into(),
            ..target
        }),
        ..lunch()
    };
    let error = host
        .send_mail(mail.mailbox.as_str(), unknown.clone())
        .await
        .unwrap_err();
    // Trying again can't help, so the bruh is told what to do instead.
    assert!(
        matches!(error, AppsError::Invalid(message) if message == MAIL_REPLY_UNKNOWN),
        "{error:?}"
    );
    assert_eq!(transport.sends(), 1);
    assert_eq!(mail.send_operations().len(), 1);
    assert_eq!(drafts(&mail).await, 0);
    assert_eq!(
        host.reply_subject(mail.mailbox.as_str(), unknown.reply.as_ref().unwrap())
            .await,
        None
    );
}

#[tokio::test]
async fn a_slow_send_says_it_may_have_sent_and_keeps_one_operation() {
    let mail = ConnectedMail::gmail().await;
    // The send is held at the provider until released, so the budget
    // always runs out after the journal holds the operation.
    let transport = FakeTransport::new(Mode::Accepted, true);
    let host = mail_host(&mail, transport.clone()).with_send_budget(Duration::from_millis(1000));

    let started = std::time::Instant::now();
    let outcome = host
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(outcome.message, MAIL_MAY_HAVE_SENT);
    assert_eq!(outcome.state, MailDispatchState::Dispatching);
    let operation = outcome.operation_id.clone().unwrap();
    assert_eq!(mail.send_operations().len(), 1);

    // The dispatch keeps running on its own and finishes once.
    transport.release.as_ref().unwrap().notify_one();
    let accepted = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status = mail
                .runtime
                .compose_send_status(
                    mail.runtime.host().as_str().into(),
                    mail.mailbox.as_str().into(),
                    operation.clone(),
                )
                .await
                .unwrap()
                .unwrap();
            if status.state == MailDispatchState::Accepted {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(accepted.operation_id, operation);
    let journal = mail.send_operations();
    assert_eq!(journal.len(), 1, "{journal:?}");
    assert_eq!(journal[0].1, "accepted");
    assert_eq!(transport.sends(), 1);
}

#[tokio::test]
async fn a_timeout_before_the_journal_says_not_sent_and_leaves_no_draft() {
    let mail = ConnectedMail::gmail().await;
    let transport = FakeTransport::slow_profile(Mode::Accepted, Duration::from_secs(5));
    let host = mail_host(&mail, transport.clone()).with_send_budget(Duration::from_millis(200));

    let error = host
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppsError::TimedOut(message) if message == MAIL_NOT_SENT),
        "{error:?}"
    );
    assert_eq!(drafts(&mail).await, 0);
    assert!(mail.send_operations().is_empty());
    assert_eq!(transport.sends(), 0);
}

#[tokio::test]
async fn a_refused_sender_check_deletes_the_draft_and_says_why() {
    let mail = ConnectedMail::gmail().await;
    let transport = FakeTransport::refusing_sender(MailSendProblemKind::SignIn);
    let host = mail_host(&mail, transport.clone());

    // The draft is created and saved before the sender check fails.
    let error = host
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppsError::Unavailable(message)
            if message == "Reconnect this mail account and review the email again."),
        "{error:?}"
    );
    assert_eq!(drafts(&mail).await, 0);
    assert!(mail.send_operations().is_empty());
    assert_eq!(transport.sends(), 0);
}

#[tokio::test]
async fn provider_problems_come_back_as_their_own_copy() {
    let mail = ConnectedMail::gmail().await;
    let transport = FakeTransport::new(Mode::Refused(MailSendProblemKind::SignIn), false);
    let host = mail_host(&mail, transport.clone());

    let outcome = host
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap();
    assert_eq!(outcome.state, MailDispatchState::Rejected);
    assert_eq!(
        outcome.message,
        "Reconnect this mail account and review the email again."
    );
    assert!(outcome.operation_id.is_some());
    assert_eq!(transport.sends(), 1);
    assert_eq!(mail.send_operations().len(), 1);
    // The journal holds it, so its draft stays beside its send status.
    assert_eq!(drafts(&mail).await, 1);

    // A provider that can't confirm the send: it may have gone out.
    let uncertain = FakeTransport::new(Mode::Uncertain, false);
    let outcome = mail_host(&mail, uncertain)
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap();
    assert_eq!(outcome.state, MailDispatchState::Uncertain);
    assert_eq!(outcome.message, MAIL_MAY_HAVE_SENT);
}

#[tokio::test]
async fn only_a_connected_mailbox_named_by_from_sends() {
    let mail = ConnectedMail::gmail().await;
    let transport = FakeTransport::new(Mode::Accepted, false);
    let host = mail_host(&mail, transport.clone());
    let label = host.mailboxes().await.unwrap()[0].account.clone();

    let other = MailSendInput {
        from: Some("other@x.test".into()),
        ..lunch()
    };
    let error = host
        .send_mail(mail.mailbox.as_str(), other)
        .await
        .unwrap_err();
    assert!(
        matches!(error, AppsError::Refused(message) if message == MAIL_NOT_GRANTED),
        "{error:?}"
    );
    let error = host.send_mail("gmail:nope", lunch()).await.unwrap_err();
    assert!(
        matches!(error, AppsError::Refused(message) if message == MAIL_NOT_GRANTED),
        "{error:?}"
    );
    // A mailbox whose connection is gone doesn't send either.
    let gone = bare(
        SecretManager::new(SecretStore::in_memory())
            .with_connections(crate::connections::Connections::in_memory()),
        Some(mail.runtime.clone()),
    )
    .with_mail_transport(transport.clone());
    let error = gone
        .send_mail(mail.mailbox.as_str(), lunch())
        .await
        .unwrap_err();
    assert!(matches!(error, AppsError::Refused(_)), "{error:?}");
    assert_eq!(transport.sends(), 0);
    assert_eq!(drafts(&mail).await, 0);

    // The label, in any case, sends.
    let named = MailSendInput {
        from: Some(label.to_uppercase()),
        ..lunch()
    };
    let outcome = host.send_mail(mail.mailbox.as_str(), named).await.unwrap();
    assert_eq!(outcome.message, MAIL_SENT);
}

#[tokio::test]
async fn queued_means_on_its_way_only_when_the_dispatch_returned() {
    let mail = ConnectedMail::gmail().await;
    let send = super::MailSend {
        mail: &mail.runtime,
        host: mail.runtime.host().as_str().to_owned(),
        mailbox: super::Mailbox {
            id: mail.mailbox.as_str(),
            provider: ServiceProvider::Gmail,
            connection_id: "synthetic-connection",
        },
        client_id: "bruh-a".into(),
        operation_id: "operation-a".into(),
        dispatched: std::sync::atomic::AtomicBool::new(true),
    };
    for state in [MailDispatchState::Queued, MailDispatchState::Dispatching] {
        let status = MailSendStatus {
            host_id: send.host.clone(),
            mailbox_id: mail.mailbox.as_str().into(),
            operation_id: "operation-a".into(),
            draft_id: "d".into(),
            draft_revision: 1,
            state,
            provider_message_id: None,
            provider_thread_id: None,
            problem: None,
        };
        assert_eq!(send.outcome(&status, None, true).message, MAIL_QUEUED);
        // Left there by a failed or timed out dispatch, it may have gone out.
        assert_eq!(
            send.outcome(&status, None, false).message,
            MAIL_MAY_HAVE_SENT
        );
    }
}

#[tokio::test]
async fn no_mail_runtime_means_no_mail() {
    let host = bare(SecretManager::new(SecretStore::in_memory()), None);
    let error = host.send_mail("gmail:m", lunch()).await.unwrap_err();
    assert!(
        matches!(error, AppsError::Unavailable(message) if message == MAIL_UNAVAILABLE),
        "{error:?}"
    );
}
