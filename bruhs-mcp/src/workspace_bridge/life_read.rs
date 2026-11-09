use super::*;
use crate::args::{MAX_OFFSET, MAX_QUERY_BYTES};

pub(super) fn translate(
    tool: &str,
    domain: AgentDomain,
    args: &Value,
) -> Result<BridgeRequest, ArgumentError> {
    let mut object = match args {
        Value::Null => serde_json::Map::new(),
        Value::Object(object) if !object.contains_key("kind") => object.clone(),
        _ => {
            return Err(ArgumentError::Rule(
                "Supply optional section, query and nonnegative offset only.",
            ))
        }
    };
    object.insert(
        "kind".into(),
        if domain == AgentDomain::Finance {
            "read_finance"
        } else {
            "read_health"
        }
        .into(),
    );
    let request: BridgeRequest = args::parse(tool, &Value::Object(object))?;
    match &request {
        BridgeRequest::ReadFinance { query, offset, .. }
        | BridgeRequest::ReadHealth { query, offset, .. }
            if query.len() <= MAX_QUERY_BYTES && *offset <= MAX_OFFSET =>
        {
            Ok(request)
        }
        _ => Err(ArgumentError::Rule("Query or offset is too large.")),
    }
}
