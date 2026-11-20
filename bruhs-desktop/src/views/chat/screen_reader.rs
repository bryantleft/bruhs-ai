//! Announce complete changes in the open conversation without reading each token.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Announcements {
    #[default]
    Status,
    Messages,
    Everything,
}

impl Announcements {
    pub fn key(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Messages => "messages",
            Self::Everything => "everything",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ReaderPreferences {
    pub announcements: Announcements,
    pub window_title: bool,
}

#[derive(Default)]
pub(super) struct Readout {
    pub(super) mode: Announcements,
    latest: SharedString,
    offset: usize,
}

impl Readout {
    pub fn latest_status(&mut self, text: &'static str) {
        self.latest = text.into();
    }

    pub fn status(&mut self, text: &'static str) {
        self.latest = text.into();
        self.offset = 0;
    }

    pub fn tool_start(&mut self, content: &str, tool: &str) {
        if self.mode == Announcements::Status {
            return;
        }
        let message = content.get(self.offset..).unwrap_or(content).trim();
        self.offset = content.len();
        let step = if self.mode == Announcements::Everything {
            format!("Running {tool}.")
        } else {
            String::new()
        };
        if !message.is_empty() || !step.is_empty() {
            self.latest = [message, step.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .into();
        }
    }

    pub fn element(&self) -> impl IntoElement {
        div()
            .id("chat-reader-live-region")
            .absolute()
            .role(Role::Status)
            .aria_label(self.latest.clone())
            .a11y_synthetic_children(|tree| {
                tree.parent_node().set_live(gpui::accesskit::Live::Polite)
            })
    }
}

impl ChatView {
    pub(crate) fn set_announcements(&mut self, mode: Announcements) {
        if self.readout.mode != mode {
            self.readout = Readout {
                mode,
                ..Default::default()
            };
        }
    }

    pub(crate) fn reader_title(&self) -> String {
        let status = if self.has_pending_permission() {
            "Needs approval"
        } else if self.pending_secret.is_some() {
            "Needs input"
        } else if self.is_streaming {
            "Working"
        } else {
            match self
                .messages
                .last()
                .filter(|m| m.role == MessageRole::Assistant)
                .map(|m| &m.state)
            {
                Some(TurnState::Failed { .. }) => "Failed",
                Some(TurnState::Stopped) => "Stopped",
                Some(TurnState::Finished) => "Completed",
                _ => "Ready",
            }
        };
        format!(
            "{status} · {} · {}",
            self.thread_title,
            crate::DESKTOP_WINDOW_TITLE
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{Announcements, Readout};
    #[test]
    fn readout_levels_announce_each_message_once_before_steps() {
        let mut r = Readout::default();
        r.status("Working");
        r.tool_start("Reviewing the changes.", "Read");
        assert_eq!(r.latest.as_ref(), "Working");
        r.mode = Announcements::Messages;
        r.tool_start("Reviewing the changes.", "Read");
        assert_eq!(r.latest.as_ref(), "Reviewing the changes.");
        r.mode = Announcements::Everything;
        r.tool_start("Reviewing the changes. Found the file.", "Edit");
        assert_eq!(r.latest.as_ref(), "Found the file. Running Edit.");
    }
}
