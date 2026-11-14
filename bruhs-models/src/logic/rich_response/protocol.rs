//! The widget may change only its own state and propose text for review.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const WIDGET_PROTOCOL: u8 = 1;
pub const MAX_WIDGET_STATE_BYTES: usize = super::super::attachments::TEXT_PREVIEW_BYTES;
/// Matches the rich frame extent supported by the reference T3 renderer.
pub const MAX_WIDGET_HEIGHT: f64 = 20_000.0;
pub const MIN_WIDGET_HEIGHT: f64 = 80.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WidgetState {
    pub fields: BTreeMap<String, WidgetField>,
    pub custom: serde_json::Value,
    pub scroll_x: f64,
    pub scroll_y: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WidgetField {
    pub value: String,
    pub checked: bool,
    pub selected: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetAction {
    pub label: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WidgetEvent {
    Ready,
    Resize { height: f64 },
    State { state: WidgetState },
    Error { message: String },
    Action { action: WidgetAction },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidgetMessage {
    pub bruhs: u8,
    pub nonce: String,
    #[serde(flatten)]
    pub event: WidgetEvent,
}

pub fn widget_state_key(scope: &str, html: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(scope.as_bytes());
    digest.update([0]);
    digest.update(html.as_bytes());
    format!("{:x}", digest.finalize())
}

impl WidgetState {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.scroll_x.is_finite() || !self.scroll_y.is_finite() {
            return Err("Widget scroll positions must be finite.");
        }
        if serde_json::to_vec(self)
            .map_err(|_| "Widget state is not valid JSON.")?
            .len()
            > MAX_WIDGET_STATE_BYTES
        {
            return Err("Widget state exceeds the 256 KiB response-state budget.");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn widget_identity_separates_scope_and_changed_source() {
        assert_eq!(
            widget_state_key("thread/a", "html"),
            widget_state_key("thread/a", "html")
        );
        assert_ne!(
            widget_state_key("thread/a", "html"),
            widget_state_key("thread/b", "html")
        );
        assert_ne!(
            widget_state_key("thread/a", "html"),
            widget_state_key("thread/a", "changed")
        );
    }
    #[test]
    fn widgets_can_propose_only_a_reviewable_chat_draft() {
        let packet: WidgetMessage = serde_json::from_value(serde_json::json!({"bruhs":1,"nonce":"instance","type":"action","action":{"label":"Use selection","message":"Selected option A"}})).unwrap();
        assert!(matches!(packet.event, WidgetEvent::Action { .. }));
        assert!(serde_json::from_value::<WidgetMessage>(
            serde_json::json!({"bruhs":1,"nonce":"instance","type":"execute","command":"rm -rf /"})
        )
        .is_err());
    }
}
