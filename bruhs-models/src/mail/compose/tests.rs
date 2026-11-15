use super::*;

fn content(to: &str) -> MailComposeContent {
    MailComposeContent {
        to: to.into(),
        subject: "Résumé 東京".into(),
        body: "Plain <script> text\nSecond line\r\n\tindent".into(),
        ..Default::default()
    }
}

#[test]
fn quoted_names_local_parts_unicode_and_semicolon_pastes_keep_exact_recipient_intent() {
    let draft = content(
        r#""Doe, Jane" <jane@example.test>; "a,b"@example.test, Équipe 東京 <equipe@example.test>,"#,
    );
    let recipients = draft.validate_send().unwrap();
    assert_eq!(recipients.to.len(), 3);
    assert_eq!(
        recipients.to[0],
        MailAddress {
            name: "Doe, Jane".into(),
            address: "jane@example.test".into()
        }
    );
    assert_eq!(recipients.to[1].address, r#""a,b"@example.test"#);
    assert_eq!(recipients.to[2].name, "Équipe 東京");
    let escaped = content(r#""Doe \"Nick\", Jane" <jane@example.test>"#)
        .validate_send()
        .unwrap();
    assert_eq!(escaped.to[0].name, "Doe \"Nick\", Jane");
}

#[test]
fn placement_is_preserved_and_duplicates_do_not_reveal_bcc_in_another_field() {
    let mut draft = content("Original <alice@EXAMPLE.test>");
    draft.cc = "Another name <alice@example.test>, copy@example.test".into();
    draft.bcc = "copy@example.test, Private <hidden@example.test>".into();
    let recipients = draft.validate_send().unwrap();
    assert_eq!(recipients.to.len(), 1);
    assert_eq!(recipients.cc.len(), 1);
    assert_eq!(recipients.bcc.len(), 1);
    assert_eq!(recipients.to[0].name, "Original");
    assert_eq!(recipients.bcc[0].address, "hidden@example.test");
    assert!(!recipients
        .to
        .iter()
        .chain(&recipients.cc)
        .any(|address| address.address == "hidden@example.test"));
    draft.to.clear();
    draft.cc.clear();
    assert!(
        !draft.validate_send().unwrap().bcc.is_empty(),
        "Bcc-only mail is valid"
    );
    let recipients = content("Case@example.test, case@example.test")
        .validate_send()
        .unwrap();
    assert_eq!(
        recipients.to.len(),
        2,
        "Local-part case is preserved rather than guessed"
    );
}

#[test]
fn incomplete_text_is_savable_but_every_malformed_send_token_is_rejected() {
    for value in [
        "unfinished",
        "valid@example.test, unfinished",
        "\"Unclosed <a@example.test>",
        "<a@example.test",
        "a@example.test>",
        "a@example.test\r\nBcc: hidden@example.test",
        "a@example.test\0",
    ] {
        let draft = content(value);
        draft.validate_storage().unwrap();
        assert!(
            matches!(
                draft.validate_send(),
                Err(MailComposeError::InvalidRecipient {
                    field: MailRecipientField::To,
                    ..
                })
            ),
            "{value:?}"
        );
    }
    assert_eq!(
        content(" , ; ").validate_send().unwrap_err(),
        MailComposeError::MissingRecipients
    );
    let mut draft = content("valid@example.test");
    draft.cc = "unfinished".into();
    assert!(matches!(
        draft.validate_send(),
        Err(MailComposeError::InvalidRecipient {
            field: MailRecipientField::Cc,
            index: 1
        })
    ));
}

#[test]
fn bounds_apply_to_utf8_bytes_and_total_recipients_across_all_fields() {
    let mut draft = content("valid@example.test");
    draft.body = "é".repeat(MAIL_COMPOSE_BODY_MAX_BYTES / 2);
    draft.validate_storage().unwrap();
    draft.body.push('é');
    assert_eq!(
        draft.validate_storage(),
        Err(MailComposeError::BodyTooLarge)
    );
    draft.body.clear();
    draft.to = "x".repeat(MAIL_COMPOSE_RECIPIENTS_MAX_BYTES + 1);
    assert_eq!(
        draft.validate_storage(),
        Err(MailComposeError::RecipientsTooLarge)
    );
    draft.to = (0..MAIL_COMPOSE_MAX_RECIPIENTS)
        .map(|i| format!("person-{i}@example.test"))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(draft.validate_send().unwrap().to.len(), 100);
    draft.bcc = "one-more@example.test".into();
    assert_eq!(
        draft.validate_send(),
        Err(MailComposeError::TooManyRecipients)
    );
    draft.bcc = "person-0@example.test".into();
    assert_eq!(draft.validate_send().unwrap().bcc.len(), 0);
    draft.subject = "s".repeat(MAIL_COMPOSE_SUBJECT_MAX_BYTES + 1);
    assert_eq!(
        draft.validate_storage(),
        Err(MailComposeError::SubjectTooLarge)
    );
}

#[test]
fn attachments_are_bounded_opaque_ids_and_cannot_be_duplicated_or_paths() {
    let mut draft = content("valid@example.test");
    draft.attachments = vec!["mail-draft-attachment-v1-id".into()];
    draft.validate_storage().unwrap();
    draft.attachments.push(draft.attachments[0].clone());
    assert_eq!(
        draft.validate_storage(),
        Err(MailComposeError::InvalidAttachment)
    );
    for value in [
        "",
        "../foreign/file",
        "https://example.test/file",
        "file\nheader",
    ] {
        draft.attachments = vec![value.into()];
        assert_eq!(
            draft.validate_storage(),
            Err(MailComposeError::InvalidAttachment)
        );
    }
    draft.attachments = (0..21).map(|i| format!("file-{i}")).collect();
    assert_eq!(
        draft.validate_storage(),
        Err(MailComposeError::TooManyAttachments)
    );
}

#[test]
fn header_and_body_controls_are_retained_for_editing_and_refused_at_send_review() {
    let mut draft = content("valid@example.test");
    draft.subject = "Subject\r\nBcc: hidden@example.test".into();
    draft.validate_storage().unwrap();
    assert_eq!(draft.validate_send(), Err(MailComposeError::InvalidSubject));
    draft.subject.clear();
    draft.body.push('\0');
    draft.validate_storage().unwrap();
    assert_eq!(draft.validate_send(), Err(MailComposeError::InvalidBody));
    draft.body.clear();
    draft.validate_send().unwrap();
}

#[test]
fn writable_wire_contracts_cannot_accept_sender_mime_receipt_or_reply_overrides() {
    let content = serde_json::to_value(content("valid@example.test")).unwrap();
    let base = serde_json::json!({"host_id":"host-a","expected_revision":2,"content":content});
    serde_json::from_value::<MailDraftSave>(base.clone()).unwrap();
    for field in ["from", "prepared", "reply", "mime", "state", "receipt"] {
        let mut changed = base.clone();
        changed[field] = serde_json::json!("untrusted");
        assert!(serde_json::from_value::<MailDraftSave>(changed).is_err());
    }
    let send = serde_json::json!({"host_id":"host-a","connection_id":"generation-a","operation_id":"operation-a","revision":2,"digest":"opaque-review-digest","to":"another@example.test"});
    assert!(serde_json::from_value::<MailSendRequest>(send).is_err());
}

#[test]
fn diagnostics_do_not_dump_recipient_names_or_message_content() {
    let mut draft = content("Private recipient <private@example.test>");
    draft.body = "PRIVATE-BODY".into();
    let recipients = draft.validate_send().unwrap();
    let text = format!("{draft:?} {recipients:?}");
    assert!(!text.contains("private@example.test"));
    assert!(!text.contains("PRIVATE-BODY"));
    assert!(!text.contains("Private recipient"));
}
#[test]
fn older_send_status_without_problem_still_loads_and_redacted_problem_round_trips() {
    let legacy = serde_json::json!({"host_id":"host","mailbox_id":"mailbox","operation_id":"op","draft_id":"draft","draft_revision":2,"state":"accepted","provider_message_id":null,"provider_thread_id":null});
    let status: MailSendStatus = serde_json::from_value(legacy).unwrap();
    assert!(status.problem.is_none());
    let problem = MailSendProblem {
        kind: MailSendProblemKind::RateLimited,
        retry_after_seconds: Some(3),
    };
    let restored: MailSendProblem =
        serde_json::from_str(&serde_json::to_string(&problem).unwrap()).unwrap();
    assert_eq!(restored, problem);
    assert!(problem.to_string().contains("limiting"));
}
