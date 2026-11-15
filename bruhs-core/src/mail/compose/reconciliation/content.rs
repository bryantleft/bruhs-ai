use super::super::PreparedMime;
use std::collections::BTreeMap;

// Reuse the canonical Gmail reader's 1 MiB inbound-header budget. The body
// must equal the bounded frozen MIME, so transport headers cannot unbound it.
pub(super) const MAX_HEADERS_BYTES: usize = 1 << 20;

fn split(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    let limit = raw.len().min(MAX_HEADERS_BYTES);
    let end = raw
        .get(..limit)?
        .windows(4)
        .position(|part| part == b"\r\n\r\n")?;
    Some((raw.get(..end)?, raw.get(end.checked_add(4)?..)?))
}

fn transport_header(name: &str) -> bool {
    matches!(
        name,
        "received"
            | "return-path"
            | "delivered-to"
            | "authentication-results"
            | "dkim-signature"
            | "arc-authentication-results"
            | "arc-message-signature"
            | "arc-seal"
            | "x-received"
            | "x-google-smtp-source"
            | "x-gm-message-state"
    )
}

fn store(headers: &mut BTreeMap<String, String>, field: Option<(String, String)>) -> Option<()> {
    if let Some((name, value)) = field {
        if !transport_header(&name) && headers.insert(name, value).is_some() {
            return None;
        }
    }
    Some(())
}

fn authored_headers(raw: &[u8]) -> Option<BTreeMap<String, String>> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut headers = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in text.split("\r\n") {
        if line
            .chars()
            .any(|value| value.is_control() && value != '\t')
        {
            return None;
        }
        if line.starts_with([' ', '\t']) {
            current.as_mut()?.1.push_str(line);
            continue;
        }
        store(&mut headers, current.take())?;
        let (name, value) = line.split_once(':')?;
        if name.is_empty() || !name.bytes().all(|b| (33..=126).contains(&b) && b != b':') {
            return None;
        }
        current = Some((name.to_ascii_lowercase(), value.to_owned()));
    }
    store(&mut headers, current)?;
    Some(headers)
}

pub(super) fn matches(prepared: &PreparedMime, candidate: &[u8]) -> bool {
    matches_bytes(prepared.bytes(), prepared.message_id(), candidate)
}

fn matches_bytes(original: &[u8], message_id: &str, candidate: &[u8]) -> bool {
    let Some((original_headers, original_body)) = split(original) else {
        return false;
    };
    let Some((candidate_headers, candidate_body)) = split(candidate) else {
        return false;
    };
    if original_body != candidate_body {
        return false;
    }
    let Some(original_headers) = authored_headers(original_headers) else {
        return false;
    };
    let Some(candidate_headers) = authored_headers(candidate_headers) else {
        return false;
    };
    let expected_id = format!("<{message_id}>");
    original_headers
        .get("message-id")
        .is_some_and(|value| value.trim_start_matches([' ', '\t']) == expected_id)
        && original_headers
            .get("from")
            .is_some_and(|value| !value.trim().is_empty())
        && original_headers == candidate_headers
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "bruhs.immutable@example.test";
    const MIME: &[u8] = b"From: sender@example.test\r\nTo: first@example.test, second@example.test\r\nBcc: private@example.test\r\nSubject: Reviewed\r\nMessage-ID: <bruhs.immutable@example.test>\r\nMIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\nExact reviewed body\r\n";

    #[test]
    fn sent_copy_accepts_only_transport_additions_and_lossless_header_folding() {
        let mut candidate =
            b"Received: from provider.example.test\r\nX-Received: by provider\r\n".to_vec();
        candidate.extend_from_slice(MIME);
        assert!(matches_bytes(MIME, ID, &candidate));
        let folded = String::from_utf8(MIME.to_vec()).unwrap().replace(
            "first@example.test, second",
            "first@example.test,\r\n second",
        );
        assert!(matches_bytes(MIME, ID, folded.as_bytes()));
    }

    #[test]
    fn sent_copy_rejects_changed_identity_headers_body_and_unreviewed_headers() {
        let original = std::str::from_utf8(MIME).unwrap();
        for (before, after) in [
            ("sender@example.test", "foreign@example.test"),
            ("first@example.test", "different@example.test"),
            ("Bcc: private@example.test\r\n", ""),
            ("Reviewed", "Different"),
            ("Subject: Reviewed\r\n", "Subject: Reviewed \r\n"),
            ("bruhs.immutable", "bruhs.other"),
            ("Exact reviewed body", "Different body"),
            (
                "MIME-Version: 1.0",
                "Sender: foreign@example.test\r\nMIME-Version: 1.0",
            ),
            (
                "Message-ID:",
                "Message-ID: <duplicate@example.test>\r\nMessage-ID:",
            ),
        ] {
            assert!(
                !matches_bytes(MIME, ID, original.replace(before, after).as_bytes()),
                "{before}"
            );
        }
        assert!(!matches_bytes(MIME, "different@example.test", MIME));
    }

    #[test]
    fn sent_copy_never_uses_lossy_text_or_equivalent_reencoding_as_a_positive_proof() {
        let mut changed = MIME.to_vec();
        changed.push(0xff);
        assert!(!matches_bytes(MIME, ID, &changed));
        let original = b"From: sender@example.test\r\nMessage-ID: <bruhs.immutable@example.test>\r\nContent-Transfer-Encoding: 8bit\r\n\r\nhello";
        let reencoded = b"From: sender@example.test\r\nMessage-ID: <bruhs.immutable@example.test>\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=";
        assert!(!matches_bytes(original, ID, reencoded));
    }

    #[test]
    fn sent_copy_rejects_invalid_headers_and_an_unbounded_header_prefix() {
        let malformed = std::str::from_utf8(MIME).unwrap().replace(
            "Subject: Reviewed",
            "Subject: Reviewed\nSender: bad@example.test",
        );
        assert!(!matches_bytes(MIME, ID, malformed.as_bytes()));
        let mut huge = vec![b'a'; MAX_HEADERS_BYTES];
        huge.extend_from_slice(MIME);
        assert!(!matches_bytes(MIME, ID, &huge));
    }
}
