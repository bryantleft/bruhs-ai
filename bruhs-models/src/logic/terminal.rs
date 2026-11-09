//! Terminal tab naming. Moved from bruhs-core `terminal.rs` and bruhs-web
//! `state.rs` (identical copies).

/// Tab title for a session: the project name, else the folder name, else
/// `Home`.
pub fn terminal_title(cwd: Option<&str>, project_name: Option<&str>) -> String {
    if let Some(name) = project_name.filter(|n| !n.trim().is_empty()) {
        return name.to_string();
    }
    match cwd
        .map(|c| c.trim_end_matches('/'))
        .filter(|c| !c.is_empty())
    {
        Some(c) => c.rsplit('/').next().unwrap_or(c).to_string(),
        None => "Home".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_title_prefers_project_then_folder_then_home() {
        assert_eq!(terminal_title(Some("/a/b/proj"), Some("Proj")), "Proj");
        assert_eq!(terminal_title(Some("/a/b/proj/"), None), "proj");
        assert_eq!(terminal_title(None, Some("  ")), "Home");
        assert_eq!(terminal_title(None, None), "Home");
        assert_eq!(terminal_title(Some("/"), None), "Home");
        assert_eq!(terminal_title(Some("relative"), None), "relative");
    }
}
