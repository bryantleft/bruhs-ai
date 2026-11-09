//! Preview-pane URL normalization. Moved from bruhs-core `preview.rs` and
//! bruhs-web `state.rs` (identical copies).

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

#[cfg(test)]
mod tests {
    use super::*;

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
