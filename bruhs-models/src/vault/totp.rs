//! One-time code setup: a base32 setup key or an `otpauth://totp` link
//! (D6). Codes themselves are generated in core at fill time (RFC 6238).

use serde::{Deserialize, Serialize};

pub const TOTP_SETUP_KEY: &str = "Enter a setup key of at least 16 letters and digits from 2 to 7.";
pub const TOTP_DIGITS: &str = "Use 6 or 8 digits.";
pub const TOTP_PERIOD: &str = "Use a period from 15 to 120 seconds.";
pub const TOTP_ALGORITHM: &str = "Use SHA1, SHA256 or SHA512.";
pub const TOTP_TIME_BASED: &str = "Use a time-based (TOTP) code.";

/// Fewest base32 characters a setup key may have (80 bits).
const MIN_KEY_CHARS: usize = 16;
/// Most bytes of setup text read: a key or link is far shorter.
const MAX_SETUP_BYTES: usize = 2048;
const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TotpAlgorithm {
    Sha1,
    Sha256,
    Sha512,
}

/// A parsed setup. Holds the shared secret: deliberately no `Debug` or
/// `Clone`.
pub struct TotpSetup {
    pub secret: Vec<u8>,
    pub digits: u32,
    pub period: u64,
    pub algorithm: TotpAlgorithm,
}

/// Parse a pasted setup key (spaces, hyphens and `=` padding ignored, any
/// case) or an `otpauth://totp/…?secret=…` link with optional `digits` (6
/// or 8), `period` (15 to 120, default 30) and `algorithm` (SHA1, SHA256,
/// SHA512, default SHA1). Errors are fixed copy and never repeat the text.
pub fn parse_totp_setup(text: &str) -> Result<TotpSetup, &'static str> {
    let text = text.trim();
    if text.len() > MAX_SETUP_BYTES {
        return Err(TOTP_SETUP_KEY);
    }
    let Some(link) = strip_prefix_ignore_case(text, "otpauth://") else {
        return Ok(TotpSetup {
            secret: setup_key(text)?,
            digits: 6,
            period: 30,
            algorithm: TotpAlgorithm::Sha1,
        });
    };
    let (kind, rest) = link.split_once('/').ok_or(TOTP_TIME_BASED)?;
    if !kind.eq_ignore_ascii_case("totp") {
        return Err(TOTP_TIME_BASED);
    }
    let query = rest.split_once('?').map_or("", |(_, query)| query);
    let query = query.split('#').next().unwrap_or_default();
    let mut setup = TotpSetup {
        secret: Vec::new(),
        digits: 6,
        period: 30,
        algorithm: TotpAlgorithm::Sha1,
    };
    let mut secret = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let value = percent_encoding::percent_decode_str(value)
            .decode_utf8()
            .map_err(|_| TOTP_SETUP_KEY)?;
        match key.to_ascii_lowercase().as_str() {
            "secret" => secret = Some(setup_key(&value)?),
            "digits" => {
                setup.digits = match value.trim() {
                    "6" => 6,
                    "8" => 8,
                    _ => return Err(TOTP_DIGITS),
                }
            }
            "period" => {
                setup.period = value
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|period| (15..=120).contains(period))
                    .ok_or(TOTP_PERIOD)?;
            }
            "algorithm" => {
                setup.algorithm = match value.trim().to_ascii_uppercase().as_str() {
                    "SHA1" => TotpAlgorithm::Sha1,
                    "SHA256" => TotpAlgorithm::Sha256,
                    "SHA512" => TotpAlgorithm::Sha512,
                    _ => return Err(TOTP_ALGORITHM),
                }
            }
            _ => {}
        }
    }
    setup.secret = secret.ok_or(TOTP_SETUP_KEY)?;
    Ok(setup)
}

fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

/// A setup key's bytes: at least 16 base32 characters once spaces, hyphens
/// and padding are dropped.
fn setup_key(text: &str) -> Result<Vec<u8>, &'static str> {
    let characters = text
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '='))
        .count();
    if characters < MIN_KEY_CHARS {
        return Err(TOTP_SETUP_KEY);
    }
    base32_decode(text)
        .filter(|bytes| !bytes.is_empty())
        .ok_or(TOTP_SETUP_KEY)
}

/// RFC 4648 base32 (`A` to `Z`, `2` to `7`, any case). Spaces, hyphens and
/// `=` padding are ignored. `None` for any other character or a length no
/// encoder produces.
#[must_use]
pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
    let mut bytes = Vec::with_capacity(text.len() * 5 / 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    let mut count: usize = 0;
    for c in text.bytes() {
        if matches!(c, b' ' | b'-' | b'=') {
            continue;
        }
        let upper = c.to_ascii_uppercase();
        let index = ALPHABET.iter().position(|&a| a == upper)?;
        buffer = (buffer << 5) | u32::try_from(index).ok()?;
        bits += 5;
        count += 1;
        if bits >= 8 {
            bits -= 8;
            bytes.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
            buffer &= (1 << bits) - 1;
        }
    }
    // 1, 3 and 6 trailing characters can't come from whole bytes.
    if matches!(count % 8, 1 | 3 | 6) {
        return None;
    }
    Some(bytes)
}

/// RFC 4648 base32 without padding.
#[must_use]
pub fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(
                ALPHABET[usize::try_from((buffer >> bits) & 31).unwrap_or(0)],
            ));
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(
            ALPHABET[usize::try_from((buffer << (5 - bits)) & 31).unwrap_or(0)],
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_keys_and_links_parse() {
        let key = parse_totp_setup("GEZD GNBV GY3T QOJQ GEZD GNBV GY3T QOJQ").unwrap();
        assert_eq!(
            (key.secret.as_slice(), key.digits, key.period, key.algorithm),
            (&b"12345678901234567890"[..], 6, 30, TotpAlgorithm::Sha1)
        );
        let link = parse_totp_setup(
            "otpauth://totp/Site:me?secret=GEZDGNBVGY3TQOJQ&digits=8&period=60&algorithm=SHA256",
        )
        .unwrap();
        assert_eq!(
            (link.digits, link.period, link.algorithm),
            (8, 60, TotpAlgorithm::Sha256)
        );
        assert_eq!(
            parse_totp_setup("otpauth://hotp/x?secret=GEZDGNBV").err(),
            Some("Use a time-based (TOTP) code.")
        );
        assert!(
            parse_totp_setup("GEZD1").is_err()
                && parse_totp_setup("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=7").is_err()
        );
        assert_eq!(
            base32_encode(b"12345678901234567890"),
            "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"
        );
    }

    #[test]
    fn setups_refuse_with_fixed_copy() {
        assert_eq!(parse_totp_setup("GEZD1").err(), Some(TOTP_SETUP_KEY));
        assert_eq!(
            parse_totp_setup("GEZDGNBVGY3TQOJ").err(),
            Some(TOTP_SETUP_KEY)
        );
        assert_eq!(
            parse_totp_setup("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits=7").err(),
            Some(TOTP_DIGITS)
        );
        for period in ["14", "121", "x", ""] {
            let link = format!("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&period={period}");
            assert_eq!(parse_totp_setup(&link).err(), Some(TOTP_PERIOD), "{period}");
        }
        assert_eq!(
            parse_totp_setup("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&algorithm=MD5").err(),
            Some(TOTP_ALGORITHM)
        );
        assert_eq!(
            parse_totp_setup("otpauth://totp/x?digits=6").err(),
            Some(TOTP_SETUP_KEY)
        );
        assert_eq!(
            parse_totp_setup("otpauth://steam/x?secret=GEZDGNBVGY3TQOJQ").err(),
            Some(TOTP_TIME_BASED)
        );
        let long = "A".repeat(MAX_SETUP_BYTES + 8);
        assert_eq!(parse_totp_setup(&long).err(), Some(TOTP_SETUP_KEY));
    }

    #[test]
    fn setups_accept_common_spellings() {
        let lower = parse_totp_setup("gezd-gnbv-gy3t-qojq====").unwrap();
        assert_eq!(lower.secret, b"1234567890");
        let link = parse_totp_setup(
            "OTPAUTH://TOTP/Site%3Ame?issuer=Site&secret=gezd%20gnbv%20gy3t%20qojq&algorithm=sha512",
        )
        .unwrap();
        assert_eq!(
            (
                link.secret.as_slice(),
                link.digits,
                link.period,
                link.algorithm
            ),
            (&b"1234567890"[..], 6, 30, TotpAlgorithm::Sha512)
        );
        assert_eq!(
            serde_json::to_value(TotpAlgorithm::Sha256).unwrap(),
            serde_json::json!("sha256")
        );
    }

    #[test]
    fn base32_round_trips_rfc_4648_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "MY"),
            ("fo", "MZXQ"),
            ("foo", "MZXW6"),
            ("foob", "MZXW6YQ"),
            ("fooba", "MZXW6YTB"),
            ("foobar", "MZXW6YTBOI"),
        ] {
            assert_eq!(base32_encode(plain.as_bytes()), encoded);
            assert_eq!(base32_decode(encoded).as_deref(), Some(plain.as_bytes()));
        }
        assert_eq!(
            base32_decode("MZXW6YTBOI======").as_deref(),
            Some(&b"foobar"[..])
        );
        assert_eq!(base32_decode("MZXW6YTB1"), None);
        assert_eq!(base32_decode("M"), None);
        assert_eq!(base32_decode("MZX"), None);
    }

    #[test]
    fn setup_errors_never_repeat_the_text() {
        crate::vault::tests::assert_no_debug_or_clone!(TotpSetup);
        // The `1` keeps a canary from ever being valid base32.
        let canary = format!("canary1{}", uuid::Uuid::new_v4().simple());
        for text in [
            format!("otpauth://totp/x?secret={canary}"),
            format!("otpauth://totp/x?secret=GEZDGNBVGY3TQOJQ&digits={canary}"),
            canary.clone(),
        ] {
            let error = parse_totp_setup(&text).err().unwrap();
            assert!(!error.contains(&canary));
        }
    }
}
