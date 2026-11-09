//! Validated tool-argument types shared by the standalone and bridge routes,
//! so a bound is written once and cannot drift between tools.
#[cfg(unix)]
use bruhs_core::workspace_bridge::ProposalError;
use bruhs_models::{AgentDomain, WorkflowOperation};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;

/// Longest identifier a tool accepts, as the published schemas advertise.
pub const MAX_ID_BYTES: usize = 128;
/// Longest `query` a read tool accepts, in UTF-8 bytes.
pub const MAX_QUERY_BYTES: usize = 512;
/// Largest `offset` a read tool accepts.
pub const MAX_OFFSET: usize = 100_000;

/// Why a tool call's arguments are rejected. Reported to the caller as a tool
/// result with `isError: true` (see [`crate::protocol`]).
#[derive(Debug, thiserror::Error)]
pub enum ArgumentError {
    #[error("Invalid arguments for {tool}: {source}")]
    Schema {
        tool: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("{0}")]
    Rule(&'static str),
    #[error("Operation belongs to {operation}, not {tool}")]
    WrongDomain {
        operation: &'static str,
        tool: &'static str,
    },
    #[error("Invalid operation content, size, duration, or timestamp. No workflow was created.")]
    InvalidOperation,
    #[cfg(unix)]
    #[error("Submitting requires a request_id of 1–128 bytes.")]
    MissingRequestId,
    #[cfg(unix)]
    #[error(transparent)]
    Proposal(#[from] ProposalError),
    #[cfg(unix)]
    #[error("This tool is not available through the workspace connection.")]
    NotBridged,
}

/// Parses a tool's `arguments`. A missing or `null` value is an empty object,
/// so tools whose arguments are all optional accept both.
pub fn parse<T: DeserializeOwned>(tool: &str, arguments: &Value) -> Result<T, ArgumentError> {
    let arguments = if arguments.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        arguments.clone()
    };
    serde_json::from_value(arguments).map_err(|source| ArgumentError::Schema {
        tool: tool.to_owned(),
        source,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("must be non-blank and at most {max} bytes")]
pub struct BoundedIdError {
    max: usize,
}

/// A non-blank string of at most `N` bytes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct BoundedId<const N: usize>(String);

impl<const N: usize> BoundedId<N> {
    pub fn into_string(self) -> String {
        self.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> TryFrom<String> for BoundedId<N> {
    type Error = BoundedIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() || value.len() > N {
            return Err(BoundedIdError { max: N });
        }
        Ok(Self(value))
    }
}

/// An identifier as the schemas bound it (1–128 bytes).
pub type ToolId = BoundedId<MAX_ID_BYTES>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("must be at most {MAX_QUERY_BYTES} bytes")]
pub struct QueryError;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Query(String);

impl Query {
    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for Query {
    type Error = QueryError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() > MAX_QUERY_BYTES {
            return Err(QueryError);
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("must be at most {MAX_OFFSET}")]
pub struct OffsetError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "usize")]
pub struct Offset(usize);

impl Offset {
    pub fn get(self) -> usize {
        self.0
    }
}

impl TryFrom<usize> for Offset {
    type Error = OffsetError;

    fn try_from(value: usize) -> Result<Self, Self::Error> {
        if value > MAX_OFFSET {
            return Err(OffsetError);
        }
        Ok(Self(value))
    }
}

/// Optional `query` and `offset`, ignoring any other field (the schema of the
/// generic domain reads does not forbid extras).
#[derive(Debug, Default, Deserialize)]
pub struct ReadPage {
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub offset: Offset,
}

/// Like [`ReadPage`] for tools whose schema says `additionalProperties: false`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrictReadPage {
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub offset: Offset,
}

#[derive(Debug, Deserialize)]
struct RawWrite {
    operation: WorkflowOperation,
    #[serde(default = "preview_by_default")]
    dry_run: bool,
    #[serde(default)]
    request_id: Option<ToolId>,
}

pub fn preview_by_default() -> bool {
    true
}

/// A `bruhs_write_<domain>` call whose operation is typed, belongs to the
/// tool's domain, and is valid.
#[derive(Debug)]
pub struct WriteRequest {
    pub operation: WorkflowOperation,
    pub dry_run: bool,
    pub request_id: Option<ToolId>,
}

impl WriteRequest {
    pub fn parse(
        tool: &str,
        domain: AgentDomain,
        arguments: &Value,
    ) -> Result<Self, ArgumentError> {
        let raw: RawWrite = parse(tool, arguments)?;
        if raw.operation.domain() != domain {
            return Err(ArgumentError::WrongDomain {
                operation: raw.operation.domain().label(),
                tool: domain.label(),
            });
        }
        if !raw.operation.is_valid() {
            return Err(ArgumentError::InvalidOperation);
        }
        Ok(Self {
            operation: raw.operation,
            dry_run: raw.dry_run,
            request_id: raw.request_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WithId {
        id: BoundedId<4>,
    }

    #[test]
    fn bounded_ids_are_non_blank_and_capped_in_bytes() {
        let ok: WithId = serde_json::from_value(json!({"id":"abcd"})).unwrap();
        assert_eq!(ok.id.as_str(), "abcd");
        for bad in [
            json!(""),
            json!("  "),
            json!("abcde"),
            json!("éé€"),
            json!(1),
        ] {
            assert!(
                serde_json::from_value::<WithId>(json!({"id": bad})).is_err(),
                "{bad}"
            );
        }
        assert!(serde_json::from_value::<WithId>(json!({})).is_err());
    }

    #[test]
    fn the_tool_id_bound_is_the_advertised_128_bytes() {
        assert!(ToolId::try_from("x".repeat(128)).is_ok());
        assert!(ToolId::try_from("x".repeat(129)).is_err());
    }

    #[test]
    fn query_and_offset_caps_match_the_specific_readers() {
        let page: ReadPage = parse("t", &Value::Null).unwrap();
        assert_eq!(
            (page.query.into_string(), page.offset.get()),
            (String::new(), 0)
        );
        assert!(parse::<ReadPage>("t", &json!({"query":"x".repeat(512),"offset":100_000})).is_ok());
        for bad in [
            json!({"query":"x".repeat(513)}),
            json!({"offset":100_001}),
            json!({"offset":-1}),
            json!({"offset":1.5}),
            json!({"query":1}),
        ] {
            assert!(parse::<ReadPage>("t", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn lenient_pages_ignore_extras_and_strict_pages_reject_them() {
        let extra = json!({"offset":1,"limit":5});
        assert!(parse::<ReadPage>("t", &extra).is_ok());
        assert!(parse::<StrictReadPage>("t", &extra).is_err());
    }

    #[test]
    fn schema_errors_name_the_tool_and_the_field() {
        let error = parse::<StrictReadPage>("bruhs_read_people", &json!({"person_id":"p"}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("bruhs_read_people"), "{error}");
        assert!(error.contains("person_id"), "{error}");
    }
}
