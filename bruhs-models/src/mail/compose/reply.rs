//! Suggestions from a cached envelope remain editable and are checked again at
//! review. Bcc is never inferred from received mail.
use super::*;
use crate::{mail::MailMessageDetail, MessageDirection};

pub fn reply_content(
    detail: &MailMessageDetail,
    direction: MessageDirection,
    own_address: &str,
    all: bool,
) -> MailComposeContent {
    let envelope = &detail.envelope;
    let primary = if direction == MessageDirection::Outgoing {
        &envelope.to
    } else if envelope.reply_to.is_empty() {
        &envelope.from
    } else {
        &envelope.reply_to
    };
    let mut seen = HashSet::new();
    let mut keep = |address: &MailAddress| -> Option<String> {
        if address.address.eq_ignore_ascii_case(own_address) {
            return None;
        }
        let content = MailComposeContent {
            to: address.address.clone(),
            ..Default::default()
        };
        let parsed = content.validate_send().ok()?;
        let parsed = parsed.to.first()?;
        if parsed.address != address.address || !parsed.name.is_empty() {
            return None;
        }
        let (local, domain) = parsed.address.rsplit_once('@')?;
        seen.insert(format!("{local}@{}", domain.to_ascii_lowercase()))
            .then(|| parsed.address.clone())
    };
    let mut to = primary.iter().filter_map(&mut keep).collect::<Vec<_>>();
    let mut cc = Vec::new();
    if all {
        to.extend(envelope.to.iter().filter_map(&mut keep));
        cc.extend(envelope.cc.iter().filter_map(&mut keep));
    }
    MailComposeContent {
        to: to.join(", "),
        cc: cc.join(", "),
        subject: envelope.subject.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn detail() -> MailMessageDetail {
        serde_json::from_value(serde_json::json!({"id":"message","external_id":"external","label_ids":[],"envelope":{"from":[{"name":"Sender","address":"sender@example.test"}],"to":[{"name":"Me","address":"me@example.test"},{"name":"Other","address":"other@example.test"}],"cc":[{"name":"Again","address":"other@EXAMPLE.test"},{"name":"Cc","address":"cc@example.test"}],"bcc":[{"name":"Private","address":"hidden@example.test"}],"reply_to":[{"name":"Reply","address":"reply@example.test"}],"subject":"Original exact subject","message_id":null,"in_reply_to":[],"references":[],"headers":[]},"attachments":[],"body_from_html":false})).unwrap()
    }
    #[test]
    fn incoming_reply_uses_reply_to_and_never_infers_bcc() {
        let value = reply_content(
            &detail(),
            MessageDirection::Incoming,
            "me@example.test",
            false,
        );
        assert_eq!(value.to, "reply@example.test");
        assert!(value.cc.is_empty());
        assert!(value.bcc.is_empty());
        assert!(value.attachments.is_empty());
        assert_eq!(value.subject, "Original exact subject");
    }
    #[test]
    fn reply_all_excludes_self_deduplicates_domain_case_and_keeps_cc() {
        let value = reply_content(
            &detail(),
            MessageDirection::Incoming,
            "ME@example.test",
            true,
        );
        assert_eq!(value.to, "reply@example.test, other@example.test");
        assert_eq!(value.cc, "cc@example.test");
        assert!(value.bcc.is_empty());
    }
    #[test]
    fn outgoing_reply_targets_original_to_instead_of_the_sender() {
        let value = reply_content(
            &detail(),
            MessageDirection::Outgoing,
            "me@example.test",
            false,
        );
        assert_eq!(value.to, "other@example.test");
    }
    #[test]
    fn malformed_source_addresses_never_become_a_header() {
        let mut detail = detail();
        detail.envelope.reply_to[0].address = "safe@example.test\r\nBcc: bad@example.test".into();
        let value = reply_content(
            &detail,
            MessageDirection::Incoming,
            "me@example.test",
            false,
        );
        assert!(value.to.is_empty());
        assert!(value.validate_send().is_err());
    }
}
