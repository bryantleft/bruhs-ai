//! JSON-RPC 2.0 framing types for the MCP stdio transport.
//!
//! Error rule, applied uniformly by [`crate::serve`]:
//!
//! * A frame that is not JSON is a parse error (-32700); JSON that is not a
//!   JSON-RPC 2.0 request object (wrong or missing `jsonrpc`, no `method`,
//!   batch arrays) is an invalid request (-32600); an unknown method is
//!   -32601.
//! * `tools/call` whose `params` are malformed (no `name`) or that names an
//!   unknown tool is invalid params (-32602): the request itself is wrong.
//! * Everything about the *arguments* of a known tool -- missing, malformed,
//!   oversized, unknown fields -- and every execution failure is a tool result
//!   with `isError: true`, so the model that made the call can read the reason
//!   and retry.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const JSONRPC_VERSION: &str = "2.0";

/// The JSON-RPC 2.0 error codes this server emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum ErrorCode {
    ParseError = -32700,
    InvalidRequest = -32600,
    MethodNotFound = -32601,
    InvalidParams = -32602,
    InternalError = -32603,
}

impl ErrorCode {
    pub const fn code(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

/// Why a frame is not a request this server can dispatch.
#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("Parse error: {0}")]
    Parse(#[source] serde_json::Error),
    #[error("Invalid request: {0}")]
    Shape(#[source] serde_json::Error),
    #[error("Invalid request: jsonrpc must be \"2.0\", not {0:?}")]
    Version(String),
}

impl JsonRpcRequest {
    /// Parses one frame. On failure the error response (with the request's
    /// `id` when it could be read) is [`RequestError::into_response`].
    pub fn parse(frame: &[u8]) -> Result<Self, (Option<Value>, RequestError)> {
        let value: Value =
            serde_json::from_slice(frame).map_err(|error| (None, RequestError::Parse(error)))?;
        let id = value
            .get("id")
            .filter(|id| id.is_string() || id.is_number())
            .cloned();
        let request: Self = serde_json::from_value(value)
            .map_err(|error| (id.clone(), RequestError::Shape(error)))?;
        if request.jsonrpc != JSONRPC_VERSION {
            return Err((id, RequestError::Version(request.jsonrpc)));
        }
        Ok(request)
    }
}

impl RequestError {
    pub fn into_response(self, id: Option<Value>) -> JsonRpcResponse {
        let code = match self {
            Self::Parse(_) => ErrorCode::ParseError,
            Self::Shape(_) | Self::Version(_) => ErrorCode::InvalidRequest,
        };
        JsonRpcResponse::error(id, code, self.to_string())
    }
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: ErrorCode, message: String) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(JsonRpcError {
                code: code.code(),
                message,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Serialize)]
pub struct ToolResult {
    pub content: Vec<ToolContent>,
    #[serde(rename = "isError", skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

/// The only content kind this server returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentType {
    Text,
}

#[derive(Debug, Serialize)]
pub struct ToolContent {
    #[serde(rename = "type")]
    pub content_type: ContentType,
    pub text: String,
}

impl ToolResult {
    pub fn ok(text: impl Into<String>) -> Self {
        Self::text(text, None)
    }

    pub fn err(text: impl Into<String>) -> Self {
        Self::text(text, Some(true))
    }

    fn text(text: impl Into<String>, is_error: Option<bool>) -> Self {
        Self {
            content: vec![ToolContent {
                content_type: ContentType::Text,
                text: text.into(),
            }],
            is_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_error(frame: &str) -> (Option<Value>, i32) {
        let (id, error) = JsonRpcRequest::parse(frame.as_bytes()).expect_err("must be rejected");
        let response = error.into_response(id.clone());
        (id, response.error.expect("error response").code)
    }

    #[test]
    fn error_codes_are_the_json_rpc_numbers() {
        assert_eq!(ErrorCode::ParseError.code(), -32700);
        assert_eq!(ErrorCode::InvalidRequest.code(), -32600);
        assert_eq!(ErrorCode::MethodNotFound.code(), -32601);
        assert_eq!(ErrorCode::InvalidParams.code(), -32602);
        assert_eq!(ErrorCode::InternalError.code(), -32603);
    }

    #[test]
    fn a_valid_request_parses_with_defaulted_params() {
        let request =
            JsonRpcRequest::parse(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).unwrap();
        assert_eq!(request.method, "ping");
        assert_eq!(request.id, Some(json!(1)));
        assert!(request.params.is_null());
    }

    #[test]
    fn a_wrong_or_missing_jsonrpc_version_is_an_invalid_request_that_keeps_its_id() {
        for frame in [
            r#"{"jsonrpc":"1.0","id":7,"method":"ping"}"#,
            r#"{"jsonrpc":2,"id":7,"method":"ping"}"#,
            r#"{"id":7,"method":"ping"}"#,
        ] {
            assert_eq!(
                parse_error(frame),
                (Some(json!(7)), ErrorCode::InvalidRequest.code()),
                "{frame}"
            );
        }
    }

    #[test]
    fn json_that_is_not_a_request_object_is_invalid_not_a_parse_error() {
        for frame in [
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            "42",
        ] {
            assert_eq!(
                parse_error(frame).1,
                ErrorCode::InvalidRequest.code(),
                "{frame}"
            );
        }
    }

    #[test]
    fn text_that_is_not_json_is_a_parse_error() {
        assert_eq!(
            parse_error("{nope"),
            (None, ErrorCode::ParseError.code()),
            "truncated JSON"
        );
    }

    #[test]
    fn tool_content_serializes_the_mcp_text_shape() {
        assert_eq!(
            serde_json::to_value(ToolResult::err("boom")).unwrap(),
            json!({"content":[{"type":"text","text":"boom"}],"isError":true})
        );
        assert_eq!(
            serde_json::to_value(ToolResult::ok("fine")).unwrap(),
            json!({"content":[{"type":"text","text":"fine"}]})
        );
    }
}
