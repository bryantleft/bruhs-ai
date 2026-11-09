//! PKCE verifier/challenge generation per RFC 7636.
//!
//! Verifier: 43–128 chars from the URL-safe set `[A-Za-z0-9-._~]`.
//! Challenge: BASE64URL-NO-PAD(SHA256(verifier)).
//!
//! We generate 128-char verifiers — the maximum allowed — for maximum
//! entropy. The state token is generated the same way but only needs to
//! be unique and unguessable; 32 chars is plenty.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::Rng;
use sha2::{Digest, Sha256};

const VERIFIER_LEN: usize = 128;
const STATE_LEN: usize = 32;
const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

/// Random URL-safe string of `len` characters drawn from the RFC 7636
/// unreserved set.
fn random_unreserved(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Returns `(verifier, challenge)`. Verifier is plain text; challenge is
/// BASE64URL(SHA256(verifier)) without padding.
pub fn generate_verifier_and_challenge() -> (String, String) {
    let verifier = random_unreserved(VERIFIER_LEN);
    let challenge = challenge_for_verifier(&verifier);
    (verifier, challenge)
}

/// Pure derivation step — exposed so the OAuth code can re-derive the
/// challenge for log/debug without holding onto it separately.
pub fn challenge_for_verifier(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Random URL-safe state token. Single-use, TTL'd by the PendingPkceCache.
pub fn generate_state() -> String {
    random_unreserved(STATE_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_length_is_128() {
        let (v, _) = generate_verifier_and_challenge();
        assert_eq!(v.len(), 128);
    }

    #[test]
    fn verifier_uses_only_unreserved_chars() {
        let (v, _) = generate_verifier_and_challenge();
        for ch in v.chars() {
            let is_unreserved = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~');
            assert!(
                is_unreserved,
                "{ch:?} is not in the RFC 7636 unreserved set"
            );
        }
    }

    #[test]
    fn challenge_is_base64url_no_pad_sha256() {
        // Fixed verifier from RFC 7636 Appendix B
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(challenge_for_verifier(verifier), expected);
    }

    #[test]
    fn two_verifiers_are_different() {
        let (a, _) = generate_verifier_and_challenge();
        let (b, _) = generate_verifier_and_challenge();
        assert_ne!(a, b);
    }

    #[test]
    fn state_length_is_32_and_unique() {
        let a = generate_state();
        let b = generate_state();
        assert_eq!(a.len(), 32);
        assert_eq!(b.len(), 32);
        assert_ne!(a, b);
    }
}
