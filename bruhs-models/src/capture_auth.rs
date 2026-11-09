//! The capture daemon's per-launch bearer token: where the daemon writes it
//! and what it looks like. The daemon (`bruhs-capture`) writes a fresh token
//! at every launch; native clients (`bruhs-core`, `bruhs-desktop`) read it and
//! send `Authorization: Bearer <token>`. Browsers never see it: the web app
//! reaches the daemon through bruhs-server's `/api/capture/*` proxy.

/// Overrides the token file location for the daemon and every client.
pub const TOKEN_FILE_ENV: &str = "BRUHS_CAPTURE_TOKEN_FILE";
/// Directory under the app's local data directory
/// (`~/Library/Application Support/dev.bruhs.bruhs-ai` on macOS).
pub const TOKEN_DIR: &str = "capture";
/// File name inside [`TOKEN_DIR`].
pub const TOKEN_FILE: &str = "daemon-token";
/// 32 random bytes, hex encoded.
pub const TOKEN_HEX_LEN: usize = 64;

/// The token as read from its file, if it has the expected shape
/// (64 lowercase hex digits, surrounding whitespace ignored).
pub fn parse_token(contents: &str) -> Option<&str> {
    let token = contents.trim();
    (token.len() == TOKEN_HEX_LEN
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_token_accepts_only_the_daemon_shape() {
        let token = "0123456789abcdef".repeat(4);
        assert_eq!(parse_token(&token), Some(token.as_str()));
        assert_eq!(parse_token(&format!("{token}\n")), Some(token.as_str()));
        for bad in [
            String::new(),
            token[1..].to_string(),
            format!("{token}0"),
            token.to_uppercase(),
            format!("{}g", &token[1..]),
            format!("Bearer {token}"),
        ] {
            assert_eq!(parse_token(&bad), None, "{bad:?}");
        }
    }
}
