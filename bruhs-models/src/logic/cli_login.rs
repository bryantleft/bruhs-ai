//! What a coding CLI's login status output means, shared by the server's
//! auth routes and the desktop app. Pure: the callers run the CLI.

/// The model picker's line for the Codex CLI login, on both apps.
pub const CODEX_SIGNED_IN_LINE: &str = "Codex CLI signed in";
pub const CODEX_SIGNED_OUT_LINE: &str = "Codex CLI not signed in. Run `codex login`";

/// The sign-in banner's title for a CLI named `name` (Claude, Codex), on
/// both apps: why its status could not be read, or that it is signed out.
pub fn sign_in_banner_title(name: &str, error: Option<&str>) -> String {
    match error {
        Some(error) => format!("Could not check {name} auth: {error}"),
        None => format!("{name} is not authenticated."),
    }
}

/// How a CLI answered its login status check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliLogin {
    /// Signed in; how, when the CLI said (Codex: "ChatGPT").
    SignedIn { method: Option<String> },
    /// The CLI said it is not signed in.
    SignedOut,
    /// The status could not be read, and why.
    Unknown(String),
}

/// Whether lowercased CLI output says it is logged out.
pub fn says_logged_out(lower: &str) -> bool {
    lower.contains("not logged in")
        || lower.contains("not authenticated")
        || lower.contains("unauthenticated")
        || lower.contains("login required")
}

/// Why a status is unknown, from whatever the CLI named `name` printed.
pub fn unrecognized_status(name: &str, exit_ok: bool, stdout: &str, stderr: &str) -> String {
    let detail = [stderr.trim(), stdout.trim()]
        .into_iter()
        .find(|text| !text.is_empty());
    match (detail, exit_ok) {
        (Some(detail), _) => format!("{name} reported: {detail}"),
        (None, true) => format!("{name} printed no login status"),
        (None, false) => format!("{name} failed without output"),
    }
}

/// Parse `codex login status`. Codex prints plain text such as `Logged in
/// using ChatGPT` (exit 0) when signed in, or a "not logged in" message.
/// Anything else, including a failed run, is unknown.
pub fn codex_login_status(exit_ok: bool, stdout: &str, stderr: &str) -> CliLogin {
    let lower = format!("{stdout}\n{stderr}").to_lowercase();
    if says_logged_out(&lower) {
        return CliLogin::SignedOut;
    }
    if !(exit_ok && lower.contains("logged in")) {
        return CliLogin::Unknown(unrecognized_status("Codex", exit_ok, stdout, stderr));
    }
    // "Logged in using ChatGPT" → "ChatGPT".
    let method = stdout
        .lines()
        .find_map(|line| line.trim().strip_prefix("Logged in using "))
        .map(|method| method.trim().to_string())
        .filter(|method| !method.is_empty());
    CliLogin::SignedIn { method }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_banner_titles_name_the_cli() {
        assert_eq!(
            sign_in_banner_title("Claude", None),
            "Claude is not authenticated."
        );
        assert_eq!(
            sign_in_banner_title("Codex", Some("Codex CLI not found. Install it first.")),
            "Could not check Codex auth: Codex CLI not found. Install it first."
        );
    }

    #[test]
    fn codex_login_status_reads_the_cli() {
        assert_eq!(
            codex_login_status(true, "Logged in using ChatGPT\n", ""),
            CliLogin::SignedIn {
                method: Some("ChatGPT".into())
            }
        );
        assert_eq!(
            codex_login_status(true, "Logged in using an API key\n", ""),
            CliLogin::SignedIn {
                method: Some("an API key".into())
            }
        );
        assert_eq!(
            codex_login_status(false, "Not logged in\n", ""),
            CliLogin::SignedOut
        );
        assert_eq!(
            codex_login_status(true, "Not logged in. Run `codex login`.\n", ""),
            CliLogin::SignedOut
        );
        assert_eq!(
            codex_login_status(false, "", "panic: boom"),
            CliLogin::Unknown("Codex reported: panic: boom".into())
        );
        assert_eq!(
            codex_login_status(true, "", ""),
            CliLogin::Unknown("Codex printed no login status".into())
        );
    }
}
