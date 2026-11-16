//! bruhs-server's relay between `bruhs-mcp` and the web tab that enabled a
//! session (`docs/local-mcp-workspace.md`, "Web connection").
//!
//! - Every live events stream opens with [`RelayFrame::Hello`], naming that
//!   stream. Only the tab holding the stream sees its ID, so it is the tab's
//!   capability for enabling a session and for answering requests.
//! - `POST` [`SESSION_PATH`] binds a new session to the caller's stream. The
//!   server writes a private connection file holding the session token; the
//!   tab receives only the file's path. One web session exists at a time.
//! - `bruhs-mcp` sends each [`super::BridgeRequest`] to [`CALL_PATH`] with
//!   the token as a bearer credential. The server forwards it to the bound
//!   stream as [`RelayFrame::Request`]; the tab answers at [`REPLY_PATH`].
//! - Disabling, closing or hiding the tab (its stream closes) and stopping
//!   the server revoke the session. A new stream never inherits one.
use super::{BridgeRequest, BridgeResponse};
use serde::{Deserialize, Serialize};

/// Binds (`POST`) or revokes (`DELETE`) the web session.
pub const SESSION_PATH: &str = "/api/workspace-bridge/session";
/// Where the tab answers a [`RelayFrame::Request`].
pub const REPLY_PATH: &str = "/api/workspace-bridge/reply";
/// Where `bruhs-mcp` sends requests, with `Authorization: Bearer <token>`.
/// Requests carrying browser headers (`Origin`, `Sec-Fetch-*`) are refused.
pub const CALL_PATH: &str = "/api/workspace-bridge/call";
/// Requests one session may have waiting on the tab at once.
pub const MAX_PENDING_REQUESTS: usize = 16;
/// How long the server waits for the tab's answer.
pub const REPLY_TIMEOUT_MS: u64 = 15_000;
/// Length of the hex session token and of a stream ID.
pub const SECRET_HEX_LEN: usize = 64;

/// A frame named [`crate::api::live_events::WORKSPACE_BRIDGE`] on the live
/// events stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RelayFrame {
    /// The first frame of every stream. A new stream never carries a session
    /// over, so a tab that sees one treats any session it had as ended.
    Hello { stream_id: String },
    /// A request from the session's client. Answer it once at [`REPLY_PATH`].
    Request {
        session_id: String,
        request_id: String,
        request: BridgeRequest,
    },
    /// The session bound to this stream ended.
    Revoked {
        session_id: String,
        reason: RevokeReason,
    },
}

/// Why a session ended while its tab was still connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevokeReason {
    /// The tab disabled it.
    Disabled,
    /// Another tab enabled a session; only one exists at a time.
    Replaced,
}

impl RevokeReason {
    /// What the tab says about it.
    pub const fn message(self) -> &'static str {
        match self {
            Self::Disabled => "The MCP connection is disabled.",
            Self::Replaced => {
                "Another tab enabled the MCP connection, so this tab's session ended. Enable it here to move it back."
            }
        }
    }
}

/// `POST SESSION_PATH`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnableSession {
    pub stream_id: String,
}

/// The reply to [`EnableSession`]. It never contains the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEnabled {
    pub session_id: String,
    /// The private connection file the copied MCP config names.
    pub connection_path: String,
    /// A session in another tab was revoked to make room for this one.
    pub replaced: bool,
}

/// `DELETE SESSION_PATH`. Revoking a session that already ended succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisableSession {
    pub stream_id: String,
    pub session_id: String,
}

/// `POST REPLY_PATH`: the tab's answer to one request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayReply {
    pub stream_id: String,
    pub session_id: String,
    pub request_id: String,
    pub response: BridgeResponse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentDomain;
    use serde_json::json;

    #[test]
    fn frames_are_tagged_by_kind() {
        let hello = RelayFrame::Hello {
            stream_id: "s".into(),
        };
        assert_eq!(
            serde_json::to_value(&hello).unwrap(),
            json!({"kind":"hello","stream_id":"s"})
        );
        let request = RelayFrame::Request {
            session_id: "a".into(),
            request_id: "r".into(),
            request: BridgeRequest::Read {
                domain: AgentDomain::Tasks,
                query: String::new(),
                offset: 0,
            },
        };
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["request"]["kind"], "read");
        assert_eq!(
            serde_json::from_value::<RelayFrame>(encoded).unwrap(),
            request
        );
        let revoked = RelayFrame::Revoked {
            session_id: "a".into(),
            reason: RevokeReason::Replaced,
        };
        assert_eq!(
            serde_json::to_value(&revoked).unwrap(),
            json!({"kind":"revoked","session_id":"a","reason":"replaced"})
        );
    }

    #[test]
    fn tab_bodies_reject_unknown_fields_so_no_token_or_grant_can_ride_along() {
        assert!(
            serde_json::from_value::<EnableSession>(json!({"stream_id":"s","token":"x"})).is_err()
        );
        assert!(serde_json::from_value::<RelayReply>(json!({
            "stream_id":"s","session_id":"a","request_id":"r",
            "response":{"status":"ok","value":{}},"approved":true
        }))
        .is_err());
        let reply: RelayReply = serde_json::from_value(json!({
            "stream_id":"s","session_id":"a","request_id":"r",
            "response":{"status":"error","value":"Read access is not granted for this domain."}
        }))
        .unwrap();
        assert_eq!(
            reply.response,
            BridgeResponse::Error("Read access is not granted for this domain.".into())
        );
    }
}
