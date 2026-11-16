use super::super::{Job, Phase};
use super::{ServiceConnection, Target};
use bruhs_core::mail::device::{DeviceDraft, DeviceDraftScope};
use bruhs_models::{
    connections::{ConnectionState, ServiceProvider},
    mail::*,
};
use std::collections::HashMap;
fn value() -> DeviceDraft {
    {
        let mut value = DeviceDraft::new(
            DeviceDraftScope {
                origin: "http://127.0.0.1:9".into(),
                host_id: "host-a".into(),
                mailbox_id: "mailbox-a".into(),
            },
            ServiceProvider::Gmail,
            None,
            MailComposeContent {
                body: "Newer retained text 東京".into(),
                ..Default::default()
            },
        );
        value.client_id = "client-a".into();
        value.server_revision = 9;
        value.pending_send = Some(pending());
        value.latest_send = Some(status());
        value.dirty = true;
        value
    }
}
fn job() -> Job {
    Job {
        value: value(),
        revision: Some(1),
        sequence: 7,
        saved: 7,
        running: false,
        selecting_files: false,
        phase: Phase::Ready,
        added: HashMap::new(),
        review: None,
        error: None,
    }
}

fn status() -> MailSendStatus {
    MailSendStatus {
        host_id: "host-a".into(),
        mailbox_id: "mailbox-a".into(),
        operation_id: "original-operation".into(),
        draft_id: "reviewed-draft".into(),
        draft_revision: 4,
        state: MailDispatchState::Uncertain,
        provider_message_id: None,
        provider_thread_id: None,
        problem: Some(MailSendProblem::new(MailSendProblemKind::Unconfirmed)),
    }
}
fn pending() -> MailSendRequest {
    MailSendRequest {
        host_id: "host-a".into(),
        connection_id: "old-connection".into(),
        operation_id: "original-operation".into(),
        revision: 4,
        digest: "a".repeat(64),
    }
}
fn snapshot() -> MailboxesSnapshot {
    MailboxesSnapshot {
        host_id: "host-a".into(),
        mailboxes: vec![MailboxSummary {
            mailbox_id: "mailbox-a".into(),
            provider: ServiceProvider::Gmail,
            last_connection_id: "stale-cached-connection".into(),
            account_label: "display-only@example.test".into(),
            revision: 1,
            threads: 0,
            drafts: 1,
        }],
    }
}
fn connection() -> ServiceConnection {
    ServiceConnection {
        id: Some("renewed-connection".into()),
        provider: ServiceProvider::Gmail,
        configured: true,
        account: Some("display-only@example.test".into()),
        scopes: vec![],
        state: ConnectionState::Connected,
    }
}

#[test]
fn check_sent_only_uncertain_operations_are_eligible() {
    let mut value = value();
    for state in [
        MailDispatchState::Queued,
        MailDispatchState::Dispatching,
        MailDispatchState::Accepted,
        MailDispatchState::NeedsReview,
        MailDispatchState::Cancelled,
        MailDispatchState::Rejected,
    ] {
        value.latest_send.as_mut().unwrap().state = state;
        assert!(Target::capture(&value, 7).is_none(), "{state:?}");
    }
    value.latest_send.as_mut().unwrap().state = MailDispatchState::Uncertain;
    assert!(Target::capture(&value, 7).is_some());
    value.latest_send = None;
    assert!(Target::capture(&value, 7).is_none());
}

#[test]
fn check_sent_rejects_foreign_or_incoherent_persisted_identity() {
    for field in [
        "host",
        "mailbox",
        "pending-operation",
        "pending-revision",
        "pending-host",
        "operation-empty",
        "draft-empty",
    ] {
        let mut value = value();
        match field {
            "host" => value.latest_send.as_mut().unwrap().host_id = "other-host".into(),
            "mailbox" => value.latest_send.as_mut().unwrap().mailbox_id = "other-mailbox".into(),
            "pending-operation" => {
                value.pending_send.as_mut().unwrap().operation_id = "newer-operation".into()
            }
            "pending-revision" => value.pending_send.as_mut().unwrap().revision += 1,
            "pending-host" => value.pending_send.as_mut().unwrap().host_id = "other-host".into(),
            "operation-empty" => value.latest_send.as_mut().unwrap().operation_id.clear(),
            "draft-empty" => value.latest_send.as_mut().unwrap().draft_id.clear(),
            _ => unreachable!(),
        }
        assert!(Target::capture(&value, 7).is_none(), "{field}");
    }
}

#[test]
fn check_sent_uses_current_connection_and_never_sends_a_browser_proof() {
    let target = Target::capture(&value(), 7).unwrap();
    let request = target.request(&snapshot(), connection()).unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::json!({"host_id":"host-a","connection_id":"renewed-connection"})
    );
    assert_ne!(connection().id.as_deref(), Some("old-connection"));
}

#[test]
fn check_sent_requires_current_host_mailbox_provider_and_connected_id() {
    let target = Target::capture(&value(), 7).unwrap();
    let mut other = snapshot();
    other.host_id = "other-host".into();
    assert!(target.request(&other, connection()).is_err());
    other = snapshot();
    other.mailboxes.clear();
    assert!(target.request(&other, connection()).is_err());
    other = snapshot();
    other.mailboxes[0].provider = ServiceProvider::Outlook;
    assert!(target.request(&other, connection()).is_err());
    let mut changed = connection();
    changed.provider = ServiceProvider::Outlook;
    assert!(target.request(&snapshot(), changed).is_err());
    for id in [None, Some(String::new())] {
        let mut changed = connection();
        changed.id = id;
        assert!(target.request(&snapshot(), changed).is_err());
    }
    let mut changed = connection();
    changed.state = ConnectionState::NeedsSignIn;
    assert!(target.request(&snapshot(), changed).is_err());
}

#[test]
fn check_sent_reply_is_bound_to_reviewed_identity_and_never_infers_rejection() {
    let target = Target::capture(&value(), 7).unwrap();
    for state in [MailDispatchState::Uncertain, MailDispatchState::Accepted] {
        let mut reply = status();
        reply.state = state;
        assert!(target.response_matches(&reply));
    }
    for state in [
        MailDispatchState::Queued,
        MailDispatchState::Dispatching,
        MailDispatchState::NeedsReview,
        MailDispatchState::Rejected,
        MailDispatchState::Cancelled,
    ] {
        let mut reply = status();
        reply.state = state;
        assert!(!target.response_matches(&reply));
    }
    for field in ["host", "mailbox", "operation", "draft", "revision"] {
        let mut reply = status();
        reply.state = MailDispatchState::Accepted;
        match field {
            "host" => reply.host_id = "other-host".into(),
            "mailbox" => reply.mailbox_id = "other-mailbox".into(),
            "operation" => reply.operation_id = "other-op".into(),
            "draft" => reply.draft_id = "other-draft".into(),
            "revision" => reply.draft_revision += 1,
            _ => unreachable!(),
        }
        assert!(!target.response_matches(&reply), "{field}");
    }
}

#[test]
fn check_sent_preserves_newer_edits_and_supports_a_server_restored_uncertain_row() {
    let mut value = value();
    let before = serde_json::to_value(&value).unwrap();
    let target = Target::capture(&value, 7).unwrap();
    assert_eq!(target.revision, 4);
    assert_eq!(value.server_revision, 9);
    assert_eq!(serde_json::to_value(&value).unwrap(), before);
    value.pending_send = None;
    assert!(
        Target::capture(&value, 7).is_some(),
        "lookup must not fabricate or require a send request for a server-restored row"
    );
    value.provider = ServiceProvider::Outlook;
    let target = Target::capture(&value, 7).unwrap();
    let mut current = snapshot();
    current.mailboxes[0].provider = ServiceProvider::Outlook;
    let mut renewed = connection();
    renewed.provider = ServiceProvider::Outlook;
    assert!(target.request(&current, renewed).is_ok());
}

#[test]
fn check_sent_waiter_cannot_replace_a_newer_job_sequence_or_existing_acceptance() {
    let mut job = job();
    let target = Target::start(&job).unwrap();
    job.phase = Phase::Reconcile;
    assert!(target.current(&job));
    job.sequence += 1;
    job.value.content.body = "Even newer text".into();
    assert!(!target.current(&job));
    job.sequence -= 1;
    job.value.latest_send.as_mut().unwrap().state = MailDispatchState::Accepted;
    assert!(!target.current(&job));
    job.value.latest_send.as_mut().unwrap().state = MailDispatchState::Uncertain;
    job.value.client_id = "another-client".into();
    assert!(!target.current(&job));
}

#[test]
fn check_sent_refuses_busy_phases_and_a_duplicate_click() {
    let mut job = job();
    for phase in [
        Phase::Local,
        Phase::Server,
        Phase::Upload,
        Phase::Review,
        Phase::Send,
        Phase::Status,
        Phase::Reconcile,
    ] {
        job.phase = phase;
        assert!(Target::start(&job).is_none());
    }
    for phase in [
        Phase::Ready,
        Phase::Error,
        Phase::SentUnconfirmed,
        Phase::SentError,
    ] {
        job.phase = phase;
        assert!(Target::start(&job).is_some());
    }
    job.running = true;
    assert!(Target::start(&job).is_none());
}

#[test]
fn check_sent_waits_for_file_selection_to_end() {
    let mut job = job();
    job.selecting_files = true;
    assert!(Target::start(&job).is_none());
}

#[test]
fn check_sent_older_acceptance_never_labels_newer_edits_or_delivery_as_sent() {
    let mut job = job();
    job.value.pending_send = None;
    job.value.latest_send.as_mut().unwrap().state = MailDispatchState::Accepted;
    let copy = super::super::render::status_text(&job);
    assert!(copy.contains("Newer edits remain a draft"));
    assert!(copy.contains("delivery is not confirmed"));
}
