use super::*;

fn mailbox(provider: ServiceProvider, subject: &str) -> MailboxId {
    MailboxId::from_broker_identity(provider, &"a".repeat(64), subject).unwrap()
}

#[test]
fn native_identifiers_match_independent_length_prefixed_python_vector() {
    // target/inbox-ui-qa/mail-identity-golden.json contains the independent
    // Python hashlib vector and its exact synthetic inputs.
    let mailbox = mailbox(ServiceProvider::Gmail, "subject-123");
    assert_eq!(
        mailbox.as_str(),
        "mailbox-v1-8d1ed6a52ab2c4ad285bd21db9ca0995c8efe6c8eadfc61e70a904bf87a6386d"
    );
    assert_eq!(
        mailbox.thread("thread/ABC+123=").unwrap().canonical_id,
        "mail-thread-v1-f3f47a3ed73c9f3075bf101874b1369ee8c20d745cff86937d3a0910633eda46"
    );
}

#[test]
fn same_account_reauthorization_retains_mailbox_and_thread_identity() {
    let first = mailbox(ServiceProvider::Gmail, "stable-subject");
    let reconnected = mailbox(ServiceProvider::Gmail, "stable-subject");
    assert_eq!(first, reconnected);
    assert_eq!(
        first.thread("thread-id").unwrap(),
        reconnected.thread("thread-id").unwrap()
    );
    let old = MailAuthorizationRef::from_broker_access(first, "auth-before").unwrap();
    let current = MailAuthorizationRef::from_broker_access(reconnected, "auth-after").unwrap();
    assert_eq!(
        old.validate_current(&current),
        Err(MailIdentityError::AuthorizationChanged)
    );
    assert_eq!(current.validate_current(&current), Ok(()));
}

#[test]
fn an_account_switch_cannot_reuse_a_previous_queued_authorization() {
    let old = MailAuthorizationRef::from_broker_access(
        mailbox(ServiceProvider::Gmail, "account-a"),
        "authorization-a",
    )
    .unwrap();
    let current = MailAuthorizationRef::from_broker_access(
        mailbox(ServiceProvider::Gmail, "account-b"),
        "authorization-b",
    )
    .unwrap();
    assert_eq!(
        old.validate_current(&current),
        Err(MailIdentityError::MailboxChanged)
    );
}

#[test]
fn provider_and_account_namespaces_isolate_equal_external_ids() {
    let ids = [
        mailbox(ServiceProvider::Gmail, "account-a"),
        mailbox(ServiceProvider::Gmail, "account-b"),
        mailbox(ServiceProvider::Outlook, "account-a"),
    ];
    for (index, first) in ids.iter().enumerate() {
        for second in ids.iter().skip(index + 1) {
            assert_ne!(first, second);
            assert_ne!(
                first.thread("same-external-id").unwrap().canonical_id,
                second.thread("same-external-id").unwrap().canonical_id
            );
            assert_ne!(
                first.message("same-external-id").unwrap().canonical_id,
                second.message("same-external-id").unwrap().canonical_id
            );
        }
    }
}

#[test]
fn message_and_thread_ids_have_separate_domains() {
    let mailbox = mailbox(ServiceProvider::Outlook, "account");
    assert_ne!(
        mailbox.thread("same-id").unwrap().canonical_id,
        mailbox.message("same-id").unwrap().canonical_id
    );
}

#[test]
fn opaque_subject_and_external_id_case_is_preserved() {
    let first = mailbox(ServiceProvider::Outlook, "CaseSensitiveSubject");
    let second = mailbox(ServiceProvider::Outlook, "casesensitivesubject");
    assert_ne!(first, second);
    assert_ne!(
        first.message("AAa=").unwrap().canonical_id,
        first.message("AAA=").unwrap().canonical_id
    );
}

#[test]
fn registration_changes_require_an_explicit_new_cache_namespace() {
    let first = mailbox(ServiceProvider::Gmail, "account");
    let changed =
        MailboxId::from_broker_identity(ServiceProvider::Gmail, &"b".repeat(64), "account")
            .unwrap();
    assert_ne!(first, changed);
}

#[test]
fn calendar_authorization_does_not_establish_a_mailbox() {
    for provider in [
        ServiceProvider::Google,
        ServiceProvider::Microsoft,
        ServiceProvider::Github,
    ] {
        assert_eq!(
            MailboxId::from_broker_identity(provider, &"a".repeat(64), "account"),
            Err(MailIdentityError::NotMailProvider)
        );
    }
}

#[test]
fn invalid_registration_and_empty_provider_ids_are_rejected() {
    assert_eq!(
        MailboxId::from_broker_identity(ServiceProvider::Gmail, "bad", "account"),
        Err(MailIdentityError::InvalidRegistration)
    );
    assert_eq!(
        MailboxId::from_broker_identity(ServiceProvider::Gmail, &"a".repeat(64), ""),
        Err(MailIdentityError::EmptyIdentifier)
    );
    let mailbox = mailbox(ServiceProvider::Gmail, "account");
    assert_eq!(mailbox.thread(""), Err(MailIdentityError::EmptyIdentifier));
    assert_eq!(mailbox.message(""), Err(MailIdentityError::EmptyIdentifier));
    assert_eq!(
        MailAuthorizationRef::from_broker_access(mailbox, ""),
        Err(MailIdentityError::EmptyIdentifier)
    );
}

#[test]
fn mailbox_metadata_does_not_retain_the_raw_private_subject() {
    let mailbox = mailbox(ServiceProvider::Gmail, "private-subject-is-not-a-label");
    assert_eq!(mailbox.provider(), ServiceProvider::Gmail);
    let authorization = MailAuthorizationRef::from_broker_access(mailbox, "auth-id").unwrap();
    assert!(!format!("{authorization:?}").contains("private-subject-is-not-a-label"));
    assert_eq!(authorization.mailbox().provider(), ServiceProvider::Gmail);
}

#[test]
fn offline_replicas_are_stable_per_host_and_isolated_between_hosts() {
    let thread = mailbox(ServiceProvider::Gmail, "account")
        .thread("provider-thread")
        .unwrap();
    let first = MailCacheHostId::from_host_record("persisted-host-a").unwrap();
    let restarted = MailCacheHostId::from_host_record("persisted-host-a").unwrap();
    let other = MailCacheHostId::from_host_record("persisted-host-b").unwrap();
    assert_eq!(thread.replica_id(&first), thread.replica_id(&restarted));
    assert_ne!(thread.replica_id(&first), thread.replica_id(&other));
    assert_ne!(thread.replica_id(&first).unwrap(), thread.canonical_id);
}

#[test]
fn a_replica_requires_a_host_identity_and_keeps_account_namespaces() {
    assert_eq!(
        MailCacheHostId::from_host_record(""),
        Err(MailIdentityError::EmptyIdentifier)
    );
    let host = MailCacheHostId::from_host_record("persisted-host").unwrap();
    let first = mailbox(ServiceProvider::Gmail, "a")
        .thread("same-id")
        .unwrap();
    let other = mailbox(ServiceProvider::Gmail, "b")
        .thread("same-id")
        .unwrap();
    assert_ne!(first.replica_id(&host), other.replica_id(&host));
}

#[test]
fn new_compose_drafts_have_stable_independent_identity_without_a_provider_thread() {
    let mailbox = mailbox(ServiceProvider::Gmail, "account");
    let first = mailbox.draft("client-draft-a", None).unwrap();
    assert!(first.reply_to.is_none());
    assert_eq!(first, mailbox.draft("client-draft-a", None).unwrap());
    assert_ne!(
        first.canonical_id,
        mailbox.draft("client-draft-b", None).unwrap().canonical_id
    );
    assert!(first.validate().is_ok());
}

#[test]
fn reply_drafts_cannot_attach_to_another_account_or_forged_thread_key() {
    let mailbox = mailbox(ServiceProvider::Outlook, "account-a");
    let foreign = super::tests::mailbox(ServiceProvider::Outlook, "account-b")
        .thread("thread")
        .unwrap();
    assert_eq!(
        mailbox.draft("draft", Some(&foreign)),
        Err(MailIdentityError::MailboxChanged)
    );
    let mut forged = mailbox.thread("thread").unwrap();
    forged.canonical_id = "wrong-canonical-key".into();
    assert_eq!(
        mailbox.draft("draft", Some(&forged)),
        Err(MailIdentityError::InvalidReference)
    );
}

#[test]
fn a_changed_draft_key_or_empty_client_id_is_rejected() {
    let mailbox = mailbox(ServiceProvider::Gmail, "account");
    assert_eq!(
        mailbox.draft("", None),
        Err(MailIdentityError::EmptyIdentifier)
    );
    let mut draft = mailbox.draft("client-draft", None).unwrap();
    draft.canonical_id = "a-different-draft-key".into();
    assert_eq!(draft.validate(), Err(MailIdentityError::InvalidReference));
}
