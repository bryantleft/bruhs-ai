//! One-time codes from a Saved login's setup (RFC 6238), made at fill
//! time (D13).

use super::storage::StoredTotp;
use bruhs_models::vault::TotpAlgorithm;
use hmac::{Hmac, Mac};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A code with less than this left in its period isn't used: the next
/// period's code is, after waiting for it, so a site never gets a code
/// that expires in flight.
const LAST_MILLIS: u64 = 2_000;

/// The code for `unix_secs`: HMAC of the big-endian counter, dynamic
/// truncation, modulo `10^digits`, zero-padded. A setup outside D6's rules
/// never panics here; the keychain blob reader refuses one first.
pub(super) fn totp_code(setup: &StoredTotp, unix_secs: u64) -> String {
    let counter = (unix_secs / setup.period.max(1)).to_be_bytes();
    let digest = match setup.algorithm {
        TotpAlgorithm::Sha1 => mac::<Hmac<sha1::Sha1>>(&setup.secret, &counter),
        TotpAlgorithm::Sha256 => mac::<Hmac<sha2::Sha256>>(&setup.secret, &counter),
        TotpAlgorithm::Sha512 => mac::<Hmac<sha2::Sha512>>(&setup.secret, &counter),
    };
    let offset = usize::from(digest.last().copied().unwrap_or_default() & 0x0f);
    let mut word = [0u8; 4];
    word.copy_from_slice(&digest[offset..offset + 4]);
    let binary = u32::from_be_bytes(word) & 0x7fff_ffff;
    // 10^9 is the largest power of ten a u32 holds.
    let digits = setup.digits.clamp(1, 9);
    let code = binary % 10_u32.pow(digits);
    let width = usize::try_from(digits).unwrap_or(9);
    format!("{code:0width$}")
}

/// The code to type now. With less than 2 s left in the period, wait for
/// the next one (at most 2 s) and use it.
pub(super) async fn totp_now(setup: &StoredTotp) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let now = u64::try_from(now.as_millis()).unwrap_or(u64::MAX);
    let (wait, at) = code_time(now, setup.period);
    if wait > 0 {
        tokio::time::sleep(Duration::from_millis(wait)).await;
    }
    totp_code(setup, at)
}

/// How long to wait (ms) and which second's code to make, at `now_ms`.
fn code_time(now_ms: u64, period: u64) -> (u64, u64) {
    let period_ms = period.max(1).saturating_mul(1_000);
    let left = period_ms - now_ms % period_ms;
    if left < LAST_MILLIS {
        (left, now_ms.saturating_add(left) / 1_000)
    } else {
        (0, now_ms / 1_000)
    }
}

/// An HMAC tag. HMAC takes a key of any length.
fn mac<M: Mac + hmac::digest::KeyInit>(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut mac =
        <M as hmac::digest::KeyInit>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    #[test]
    fn codes_match_rfc_6238() {
        let setup = |secret: &[u8], algorithm| StoredTotp {
            secret: Zeroizing::new(secret.to_vec()),
            digits: 8,
            period: 30,
            algorithm,
        };
        assert_eq!(
            totp_code(&setup(b"12345678901234567890", TotpAlgorithm::Sha1), 59),
            "94287082"
        );
        assert_eq!(
            totp_code(
                &setup(b"12345678901234567890", TotpAlgorithm::Sha1),
                1_111_111_109
            ),
            "07081804"
        );
        assert_eq!(
            totp_code(
                &setup(b"12345678901234567890123456789012", TotpAlgorithm::Sha256),
                59
            ),
            "46119246"
        );
        assert_eq!(
            totp_code(
                &setup(
                    b"1234567890123456789012345678901234567890123456789012345678901234",
                    TotpAlgorithm::Sha512
                ),
                59
            ),
            "90693936"
        );
        let six = StoredTotp {
            digits: 6,
            ..setup(b"12345678901234567890", TotpAlgorithm::Sha1)
        };
        assert_eq!(totp_code(&six, 59), "287082");
    }

    #[test]
    fn every_rfc_6238_vector_matches() {
        // RFC 6238 Appendix B: (time, SHA1, SHA256, SHA512), 8 digits, 30 s.
        let sha1: &[u8] = b"12345678901234567890";
        let sha256: &[u8] = b"12345678901234567890123456789012";
        let sha512: &[u8] = b"1234567890123456789012345678901234567890123456789012345678901234";
        let vectors = [
            (59, "94287082", "46119246", "90693936"),
            (1_111_111_109, "07081804", "68084774", "25091201"),
            (1_111_111_111, "14050471", "67062674", "99943326"),
            (1_234_567_890, "89005924", "91819424", "93441116"),
            (2_000_000_000, "69279037", "90698825", "38618901"),
            (20_000_000_000, "65353130", "77737706", "47863826"),
        ];
        let code = |secret: &[u8], algorithm, time| {
            totp_code(
                &StoredTotp {
                    secret: Zeroizing::new(secret.to_vec()),
                    digits: 8,
                    period: 30,
                    algorithm,
                },
                time,
            )
        };
        for (time, one, two, five) in vectors {
            assert_eq!(code(sha1, TotpAlgorithm::Sha1, time), one, "SHA1 at {time}");
            assert_eq!(
                code(sha256, TotpAlgorithm::Sha256, time),
                two,
                "SHA256 at {time}"
            );
            assert_eq!(
                code(sha512, TotpAlgorithm::Sha512, time),
                five,
                "SHA512 at {time}"
            );
        }
    }

    #[test]
    fn a_bad_setup_never_panics() {
        let setup = StoredTotp {
            secret: Zeroizing::new(b"12345678901234567890".to_vec()),
            digits: 40,
            period: 0,
            algorithm: TotpAlgorithm::Sha1,
        };
        assert_eq!(totp_code(&setup, 59).len(), 9);
    }

    #[test]
    fn a_code_near_its_period_end_waits_for_the_next() {
        // 10 s left in the period: no wait, this second's code.
        assert_eq!(code_time(20_000, 30), (0, 20));
        // 1.5 s left: wait it out and use the next period's first second.
        assert_eq!(code_time(28_500, 30), (1_500, 30));
        // Exactly 2 s left still uses this period.
        assert_eq!(code_time(28_000, 30), (0, 28));
        assert_eq!(code_time(59_999, 30), (1, 60));
    }
}
