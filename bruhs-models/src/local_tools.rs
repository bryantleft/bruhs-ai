//! Consent-scoped browser and device operations shared by both clients.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolScope {
    Browser,
    Devices,
    Dictation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolGrant {
    pub id: String,
    pub principal: String,
    pub scope: ToolScope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrantRequest {
    pub principal: String,
    pub scope: ToolScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserSession {
    pub id: String,
    pub grant_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserAction {
    Navigate { url: String },
    Snapshot,
    Click { selector: String },
    Fill { selector: String, text: String },
    Evaluate { expression: String },
    Screenshot,
    Console,
    Network,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserCommand {
    pub grant_id: String,
    pub session_id: String,
    pub command: BrowserAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserStart {
    pub grant_id: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolImage {
    pub mime_type: String,
    pub base64: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolReply {
    pub value: serde_json::Value,
    pub image: Option<ToolImage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Android,
    Ios,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub platform: DevicePlatform,
    pub state: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceInventory {
    pub devices: Vec<Device>,
    /// Discovery failures stay visible even when another platform works.
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DeviceAction {
    Boot,
    Shutdown,
    Screenshot,
    Tap { x: u32, y: u32 },
    Type { text: String },
    Home,
    Back,
    OpenUrl { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCommand {
    pub grant_id: String,
    pub device_id: String,
    pub platform: DevicePlatform,
    pub command: DeviceAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceVideoStart {
    pub grant_id: String,
    pub device_id: String,
    pub platform: DevicePlatform,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LocalToolsSnapshot {
    pub grants: Vec<ToolGrant>,
    pub browsers: Vec<BrowserSession>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationTranscript {
    /// Draft text only. A transcription never starts an agent turn.
    pub text: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictationOptions {
    #[serde(default)]
    pub model: crate::WhisperModel,
    pub language: Option<String>,
}

/// Reject active content and host-file navigation before opening a browser.
pub fn validate_browser_url(url: &str) -> Result<(), &'static str> {
    if url == "about:blank" {
        return Ok(());
    }
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or("Use an http or https URL")?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty()
        || authority.contains('@')
        || url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
    {
        return Err("The browser URL must have a host and no credentials or whitespace");
    }
    Ok(())
}

/// A transcript replaces the original selection only while that draft is unchanged.
/// A concurrently edited draft keeps its edits and receives the transcript at its end.
pub fn insert_dictation(
    current: &str,
    original: &str,
    selection: std::ops::Range<usize>,
    transcript: &str,
) -> Result<String, &'static str> {
    if transcript.trim().is_empty() {
        return Ok(current.into());
    }
    if current == original {
        if selection.start > selection.end
            || !current.is_char_boundary(selection.start)
            || !current.is_char_boundary(selection.end)
        {
            return Err(
                "The dictation selection is no longer valid. Copy the transcript into the draft.",
            );
        }
        let mut result = current.to_string();
        result.replace_range(selection, transcript);
        Ok(result)
    } else {
        let separator = if current.is_empty() || current.ends_with(char::is_whitespace) {
            ""
        } else {
            " "
        };
        Ok(format!("{current}{separator}{transcript}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_cannot_open_local_files_or_credentials() {
        for invalid in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://u:p@example.com",
            "https://",
            "https://a\\@b",
            "http://a\nb",
        ] {
            assert!(validate_browser_url(invalid).is_err(), "{invalid}");
        }
        for valid in [
            "http://localhost:3000",
            "https://example.com/path?q=x",
            "about:blank",
        ] {
            assert!(validate_browser_url(valid).is_ok(), "{valid}");
        }
    }

    #[test]
    fn dictation_preserves_selection_and_concurrent_edits() {
        assert_eq!(
            insert_dictation("fix bug", "fix bug", 4..7, "tests").unwrap(),
            "fix tests"
        );
        assert_eq!(
            insert_dictation("new draft", "fix bug", 4..7, "tests").unwrap(),
            "new draft tests"
        );
        assert_eq!(
            insert_dictation("keep", "keep", 0..4, "  ").unwrap(),
            "keep"
        );
        assert!(insert_dictation("é", "é", 1..2, "test").is_err());
    }
}
