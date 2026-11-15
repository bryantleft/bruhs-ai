use super::*;
use crate::{
    connections::Connections,
    mail::{MailRuntime, MailboxId},
};
use bruhs_models::{connections::ServiceProvider, InboxConversation, InboxMessage, InboxSource};
use mail_parser::MimeHeaders;

fn prepare(
    session: &MailSession,
    sender: &VerifiedMailSender,
    draft: MimeDraft<'_>,
) -> Result<PreparedMime, ComposeError> {
    super::prepare(
        &MimeAuthority {
            host: session.runtime.host().clone(),
            authorization: session.authorization().clone(),
        },
        sender,
        draft,
    )
}

async fn fixture() -> (
    tempfile::TempDir,
    MailSession,
    VerifiedMailSender,
    MailDraftRef,
) {
    let folder = tempfile::tempdir().unwrap();
    let runtime = MailRuntime::open(folder.path().join("mail.sqlite3"))
        .await
        .unwrap();
    let connections = Connections::test_connected(ServiceProvider::Gmail, "synthetic-token").await;
    let session = runtime
        .session(&connections, ServiceProvider::Gmail, "synthetic-connection")
        .await
        .unwrap();
    let sender = VerifiedMailSender::from_profile(
        &session,
        MailAddress {
            name: "Sender équipe".into(),
            address: "sender@example.test".into(),
        },
    )
    .unwrap();
    let reference = session
        .authorization()
        .mailbox()
        .draft("compose-a", None)
        .unwrap();
    (folder, session, sender, reference)
}

fn content() -> MailComposeContent {
    MailComposeContent {
        to: "\"Doe, Jane\" <recipient@example.test>".into(),
        cc: "Copy équipe <copy@example.test>".into(),
        bcc: "hidden@example.test".into(),
        subject: "Résumé 東京".into(),
        body: "Plain <script> text\nRésumé 東京\n".into(),
        attachments: vec![],
    }
}

fn file(reference: &MailDraftRef, client: &str, bytes: &[u8]) -> DraftAttachmentBytes {
    DraftAttachmentBytes {
        metadata: crate::mail::DraftAttachment {
            id: reference.attachment(client).unwrap(),
            file_name: "résumé-東京.bin".into(),
            size_bytes: u64::try_from(bytes.len()).unwrap(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        },
        bytes: bytes.into(),
    }
}

fn input<'a>(
    reference: &'a MailDraftRef,
    content: &'a MailComposeContent,
    files: &'a [DraftAttachmentBytes],
) -> MimeDraft<'a> {
    MimeDraft {
        reference,
        source_revision: 1,
        content,
        reply: None,
        files,
        date: EpochMs::now(),
    }
}

#[tokio::test]
async fn generated_mime_round_trips_exact_recipients_subject_body_and_file_bytes() {
    let (_folder, session, sender, reference) = fixture().await;
    let mut content = content();
    let files = [
        file(&reference, "file-a", b"private\0binary\xff"),
        file(&reference, "empty", b""),
    ];
    content.attachments = files.iter().map(|file| file.metadata.id.clone()).collect();
    let prepared = prepare(&session, &sender, input(&reference, &content, &files)).unwrap();
    let message = MessageParser::default().parse(prepared.bytes()).unwrap();
    assert_eq!(message.subject(), Some("Résumé 東京"));
    assert_eq!(
        message.from().unwrap().first().unwrap().address(),
        Some("sender@example.test")
    );
    assert_eq!(
        message.to().unwrap().first().unwrap().address(),
        Some("recipient@example.test")
    );
    assert_eq!(
        message.cc().unwrap().first().unwrap().address(),
        Some("copy@example.test")
    );
    assert_eq!(
        message.bcc().unwrap().first().unwrap().address(),
        Some("hidden@example.test")
    );
    assert_eq!(
        message.to().unwrap().first().unwrap().name(),
        Some("Doe, Jane")
    );
    assert!(message
        .body_text(0)
        .unwrap()
        .contains("Plain <script> text"));
    assert!(message.body_text(0).unwrap().contains("Résumé 東京"));
    assert_eq!(message.attachments().count(), 2);
    for (actual, expected) in message.attachments().zip(&files) {
        assert_eq!(actual.attachment_name(), Some("résumé-東京.bin"));
        assert_eq!(actual.contents(), expected.bytes);
    }
    assert_eq!(message.message_id(), Some(prepared.message_id()));
    assert_eq!(
        prepared.sha256(),
        format!("{:x}", Sha256::digest(prepared.bytes()))
    );
    assert_eq!(prepared.files().len(), 2);
    assert!(prepared.reply_thread().is_none());
    assert!(!format!("{prepared:?} {sender:?}").contains("hidden@example.test"));
}

#[tokio::test]
async fn correlation_identity_is_stable_for_one_saved_revision_and_changes_for_another() {
    let (_folder, session, sender, reference) = fixture().await;
    let content = content();
    let one = prepare(&session, &sender, input(&reference, &content, &[])).unwrap();
    let two = prepare(&session, &sender, input(&reference, &content, &[])).unwrap();
    assert_eq!(one.message_id(), two.message_id());
    let mut newer = input(&reference, &content, &[]);
    newer.source_revision = 2;
    let newer = prepare(&session, &sender, newer).unwrap();
    assert_ne!(one.message_id(), newer.message_id());
    assert!(one.message_id().starts_with("bruhs."));
    assert!(!one.message_id().contains("sender"));
}

#[tokio::test]
async fn damaged_missing_or_relabelled_files_fail_before_any_provider_dispatch() {
    let (_folder, session, sender, reference) = fixture().await;
    let mut content = content();
    let mut files = [file(&reference, "file-a", b"original")];
    content.attachments = vec![files[0].metadata.id.clone()];
    assert!(matches!(
        prepare(&session, &sender, input(&reference, &content, &[])),
        Err(ComposeError::AttachmentChanged)
    ));
    files[0].bytes[0] = 0;
    assert!(matches!(
        prepare(&session, &sender, input(&reference, &content, &files)),
        Err(ComposeError::AttachmentChanged)
    ));
    files[0] = file(&reference, "file-a", b"original");
    files[0].metadata.file_name = "../foreign.bin".into();
    assert!(matches!(
        prepare(&session, &sender, input(&reference, &content, &files)),
        Err(ComposeError::AttachmentChanged)
    ));
}

#[tokio::test]
async fn sender_profiles_cannot_supply_headers_other_accounts_or_invalid_draft_revisions() {
    let (_folder, session, sender, reference) = fixture().await;
    let content = content();
    for address in [
        "sender@example.test\r\nBcc: hidden@example.test",
        "Another <sender@example.test>",
        "one@example.test,two@example.test",
    ] {
        assert!(matches!(
            VerifiedMailSender::from_profile(
                &session,
                MailAddress {
                    name: String::new(),
                    address: address.into()
                }
            ),
            Err(ComposeError::InvalidSender)
        ));
    }
    let mut changed = sender.clone();
    changed.authorization = MailAuthorizationRef::from_broker_access(
        MailboxId::from_broker_identity(ServiceProvider::Gmail, &"b".repeat(64), "another-account")
            .unwrap(),
        "another-generation",
    )
    .unwrap();
    assert!(prepare(&session, &changed, input(&reference, &content, &[])).is_err());
    let mut invalid = input(&reference, &content, &[]);
    invalid.source_revision = 0;
    assert!(matches!(
        prepare(&session, &sender, invalid),
        Err(ComposeError::InvalidDraft)
    ));
}

fn reply(reference: &MailDraftRef) -> (MailDraftRef, MailReplyTarget, ConnectedMailThread) {
    let thread = reference.mailbox.thread("source-thread").unwrap();
    let reference = reference.mailbox.draft("reply-a", Some(&thread)).unwrap();
    let id = reference
        .mailbox
        .message("source-message")
        .unwrap()
        .canonical_id;
    let mut message = InboxMessage::incoming("Source", "Saved source body", 1);
    message.id = id.clone();
    message.external_id = Some("source-message".into());
    let mut conversation = InboxConversation::new(InboxSource::Email, "Source subject", vec![], 1);
    conversation.id = thread.canonical_id;
    conversation.external_id = Some(thread.external_id.clone());
    conversation.receive(message);
    let metadata = MailMessageDetail {
        id: id.clone(),
        external_id: "source-message".into(),
        label_ids: vec![],
        envelope: MailEnvelope {
            subject: "Source subject".into(),
            message_id: Some("source@example.test".into()),
            references: vec![
                "older@example.test".into(),
                "malformed\r\nBcc: hidden@example.test".into(),
            ],
            ..Default::default()
        },
        attachments: vec![],
        body_from_html: false,
    };
    let target = MailReplyTarget {
        external_thread_id: thread.external_id.clone(),
        message_id: id,
    };
    let thread = ConnectedMailThread {
        conversation,
        connected_mail: ConnectedMailMetadata {
            provider: ServiceProvider::Gmail,
            mailbox_id: reference.mailbox.as_str().into(),
            external_thread_id: thread.external_id,
            messages: vec![metadata],
        },
    };
    (reference, target, thread)
}

#[tokio::test]
async fn replies_keep_checked_source_headers_and_subject_without_inferred_recipients() {
    let (_folder, session, sender, reference) = fixture().await;
    let (reference, target, thread) = reply(&reference);
    let mut content = content();
    content.subject = "Source subject".into();
    let mut draft = input(&reference, &content, &[]);
    draft.reply = Some((&target, &thread));
    let prepared = prepare(&session, &sender, draft).unwrap();
    let message = MessageParser::default().parse(prepared.bytes()).unwrap();
    assert_eq!(prepared.reply_thread(), Some("source-thread"));
    assert!(format!("{:?}", message.in_reply_to()).contains("source@example.test"));
    let refs = format!("{:?}", message.references());
    assert!(refs.contains("older@example.test"));
    assert!(!refs.contains("Bcc"));
    assert_eq!(
        message.to().unwrap().first().unwrap().address(),
        Some("recipient@example.test")
    );
    assert_eq!(
        message.bcc().unwrap().first().unwrap().address(),
        Some("hidden@example.test")
    );
}

#[tokio::test]
async fn changed_reply_source_or_subject_is_rejected_instead_of_guessing_a_new_thread() {
    let (_folder, session, sender, reference) = fixture().await;
    let (reference, mut target, mut thread) = reply(&reference);
    let mut content = content();
    let mut draft = input(&reference, &content, &[]);
    draft.reply = Some((&target, &thread));
    assert!(matches!(
        prepare(&session, &sender, draft),
        Err(ComposeError::ReplySubjectChanged)
    ));
    content.subject = "Source subject".into();
    target.message_id = "another-message".into();
    let mut draft = input(&reference, &content, &[]);
    draft.reply = Some((&target, &thread));
    assert!(matches!(
        prepare(&session, &sender, draft),
        Err(ComposeError::InvalidReply)
    ));
    target.message_id = thread.connected_mail.messages[0].id.clone();
    thread.connected_mail.messages[0].envelope.message_id = Some("bad\r\nheader".into());
    let mut draft = input(&reference, &content, &[]);
    draft.reply = Some((&target, &thread));
    assert!(matches!(
        prepare(&session, &sender, draft),
        Err(ComposeError::InvalidReply)
    ));
}

#[test]
fn writer_refuses_to_exceed_the_prepared_mime_budget() {
    let mut output = BoundedMime(vec![0; MAIL_PREPARED_MIME_MAX_BYTES]);
    assert!(output.write_all(b"x").is_err());
    assert_eq!(output.0.len(), MAIL_PREPARED_MIME_MAX_BYTES);
}
