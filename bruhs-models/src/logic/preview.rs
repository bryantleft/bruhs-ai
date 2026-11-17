//! Preview-pane URL normalization. Moved from bruhs-core `preview.rs` and
//! bruhs-web `state.rs` (identical copies). Also the command palette's local
//! servers mode, which both apps show the same way.

/// Turn what the user typed into an absolute URL: `3000` and
/// `localhost:3000` become `http://localhost:3000`; blank → `None`.
pub fn normalize_preview_url(input: &str) -> Option<String> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(format!("http://localhost:{s}"));
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return Some(s.to_string());
    }
    Some(format!("http://{s}"))
}

/// The palette group and mode chip for listening local servers.
pub const LOCAL_SERVERS_GROUP: &str = "Local servers";
/// The mode chip's hint while the scan runs, and once it has.
pub const LOCAL_SERVERS_SCANNING_HINT: &str = "scanning…";
pub const LOCAL_SERVERS_OPEN_HINT: &str = "↵ opens in your browser";
/// The empty list while the scan runs, and when nothing listens.
pub const LOCAL_SERVERS_SCANNING: &str = "scanning localhost…";
pub const LOCAL_SERVERS_NONE: &str = "nothing is listening on localhost";

/// How a server listening on `port` reads in the palette.
pub fn local_server_label(port: u16) -> String {
    format!("localhost:{port}")
}

/// Where picking a server listening on `port` opens.
pub fn local_server_url(port: u16) -> String {
    format!("http://localhost:{port}")
}

/// Whether every word of `query` appears, case folded, in the server's
/// address or command. A blank query matches every server.
pub fn local_server_matches(port: u16, command: &str, query: &str) -> bool {
    let haystack = format!("{} {command}", local_server_label(port)).to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_servers_read_open_and_filter_alike() {
        assert_eq!(local_server_label(5173), "localhost:5173");
        assert_eq!(local_server_url(5173), "http://localhost:5173");
        assert!(local_server_matches(5173, "node vite", ""));
        assert!(local_server_matches(5173, "node vite", "VITE 517"));
        assert!(local_server_matches(5173, "node vite", "localhost:5173"));
        assert!(!local_server_matches(5173, "node vite", "vite 3000"));
    }

    #[test]
    fn normalize_preview_url_cases() {
        assert_eq!(normalize_preview_url("  "), None);
        assert_eq!(
            normalize_preview_url("3000").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(
            normalize_preview_url("localhost:5173").as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            normalize_preview_url("https://app.example/x").as_deref(),
            Some("https://app.example/x")
        );
        assert_eq!(
            normalize_preview_url(" http://127.0.0.1:8080 ").as_deref(),
            Some("http://127.0.0.1:8080")
        );
    }
}
