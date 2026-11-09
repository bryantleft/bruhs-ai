//! Which GitHub pull request the right panel opens for a thread's branch.
//!
//! The lookup itself is `gh` on the host. This module only decides when that
//! result should take the panel, and how a directory in the Files surface
//! walks up.

/// Parent of a relative directory in the Files surface.
///
/// `""` is the project root and has no parent. `"src"` walks up to the root
/// (`Some("")`). `"src/a"` walks up to `"src"`.
pub fn parent_dir(relative: &str) -> Option<&str> {
    let relative = relative.trim().trim_matches('/');
    if relative.is_empty() || relative == "." {
        None
    } else {
        Some(match relative.rfind('/') {
            Some(index) => &relative[..index],
            None => "",
        })
    }
}

/// Identity of one pull request the panel has already opened for a thread.
pub fn branch_pull_key(thread_id: &str, branch: &str, number: u64) -> String {
    format!("{thread_id}\n{branch}\n{number}")
}

/// Open the panel when `gh` is signed in, a pull request exists, and this
/// key has not already opened the panel. Closing the panel keeps the key,
/// so the same pull request does not open again until the thread, branch,
/// or number changes.
pub fn should_open_branch_pull(
    signed_in: bool,
    number: Option<u64>,
    opened_key: Option<&str>,
    key: &str,
) -> bool {
    signed_in && number.is_some() && opened_key != Some(key)
}

/// A pull request URL the panel may hand to the system browser.
pub fn github_pull_url(url: &str) -> Option<&str> {
    let url = url.trim();
    if url.starts_with("https://") || url.starts_with("http://") {
        Some(url)
    } else {
        None
    }
}

/// Nothing in the pull request panel can be copied.
pub const NO_PULL_OPEN: &str = "No pull request is open.";

/// The panel's URL is not one the browser can open.
pub const PULL_LINK_NOT_WEB: &str = "That pull request link is not a web address.";

/// `#N` for the pull request the panel is showing.
pub fn pull_number_copy(number: u64) -> String {
    format!("#{number}")
}

/// Text for the unbound copy commands. `link` copies the http(s) URL.
/// The panel has to be open on a pull request; a number alone is not enough
/// when `open` is false.
pub fn pull_copy_text(
    open: bool,
    number: Option<u64>,
    url: Option<&str>,
    link: bool,
) -> Result<String, &'static str> {
    if !open {
        return Err(NO_PULL_OPEN);
    }
    let Some(number) = number else {
        return Err(NO_PULL_OPEN);
    };
    if link {
        url.and_then(github_pull_url)
            .map(str::to_string)
            .ok_or(PULL_LINK_NOT_WEB)
    } else {
        Ok(pull_number_copy(number))
    }
}

/// Visible state word. Unknown states stay unlabeled.
pub fn pull_state_label(state: &str) -> &'static str {
    match state {
        "open" => "Open",
        "draft" => "Draft",
        "merged" => "Merged",
        "closed" => "Closed",
        _ => "Pull request",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_dir_walks_toward_the_project_root() {
        assert_eq!(parent_dir(""), None);
        assert_eq!(parent_dir("/"), None);
        assert_eq!(parent_dir("src"), Some(""));
        assert_eq!(parent_dir("src/a"), Some("src"));
        assert_eq!(parent_dir("src/a/"), Some("src"));
    }

    #[test]
    fn a_pull_request_opens_once_per_thread_branch_and_number() {
        let key = branch_pull_key("thread", "feature", 12);
        assert!(should_open_branch_pull(true, Some(12), None, &key));
        assert!(!should_open_branch_pull(true, Some(12), Some(&key), &key));
        assert!(!should_open_branch_pull(false, Some(12), None, &key));
        assert!(!should_open_branch_pull(true, None, None, &key));
        let next = branch_pull_key("thread", "feature", 13);
        assert!(should_open_branch_pull(true, Some(13), Some(&key), &next));
    }

    #[test]
    fn only_http_urls_open_outside_the_app() {
        assert_eq!(
            github_pull_url(" https://github.com/acme/app/pull/3 "),
            Some("https://github.com/acme/app/pull/3")
        );
        assert_eq!(github_pull_url("file:///tmp/pr"), None);
        assert_eq!(pull_state_label("draft"), "Draft");
        assert_eq!(pull_state_label("queued"), "Pull request");
    }

    #[test]
    fn copy_uses_the_open_pull_request_only() {
        assert_eq!(
            pull_copy_text(false, Some(4), Some("https://x"), false),
            Err(NO_PULL_OPEN)
        );
        assert_eq!(
            pull_copy_text(true, None, Some("https://x"), true),
            Err(NO_PULL_OPEN)
        );
        assert_eq!(
            pull_copy_text(true, Some(4), Some("file:///tmp/pr"), true),
            Err(PULL_LINK_NOT_WEB)
        );
        assert_eq!(
            pull_copy_text(
                true,
                Some(4),
                Some(" https://github.com/acme/app/pull/4 "),
                true
            )
            .as_deref(),
            Ok("https://github.com/acme/app/pull/4")
        );
        assert_eq!(
            pull_copy_text(true, Some(4), None, false).as_deref(),
            Ok("#4")
        );
    }
}
