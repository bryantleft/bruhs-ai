use super::*;
pub(super) const READS: [&str; 3] = [
    "list_whiteboards",
    "read_whiteboard",
    "whiteboard_proposal_status",
];
pub(super) const WRITE: &str = "propose_whiteboard";
pub(super) fn tools(read_only: bool) -> Vec<Value> {
    let board_id =
        json!({"type":"string","description":"Complete board UUID from list_whiteboards."});
    let mut tools = vec![
        json!({"name":READS[0],"description":"List local boards explicitly shared with agents in this thread's working folder. Native and web have independent local stores.","annotations":{"readOnlyHint":true,"openWorldHint":false},"inputSchema":{"type":"object","additionalProperties":false,"properties":{}}}),
        json!({"name":READS[1],"description":"Read one page of editable shape records and source fingerprints from a shared local board. Follow next_cursor until null before planning a whole-board edit. Image payloads stay local.","annotations":{"readOnlyHint":true,"openWorldHint":false},"inputSchema":{"type":"object","additionalProperties":false,"required":["board_id"],"properties":{"board_id":board_id,"cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100}}}}),
        json!({"name":READS[2],"description":"Read the status of this turn's immutable proposal. Do not poll while awaiting the owner: tell them to review it in Whiteboards.","annotations":{"readOnlyHint":true,"openWorldHint":false},"inputSchema":{"type":"object","additionalProperties":false,"required":["proposal_id"],"properties":{"proposal_id":{"type":"string"}}}}),
    ];
    if !read_only {
        tools.push(json!({"name":WRITE,"description":"Create an immutable preview against the current board revision. No edit is applied until its owner accepts in the UI. edits_json is JSON: {operations:[{op:upsert,shape:...},{op:delete,id:...}]} using records from read_whiteboard, or {mermaid:'flowchart LR\\n A-->B',source_path:'src/file.rs'} to append an editable diagram with a living source fingerprint. Preserve manual shapes when refreshing. Cannot enable sharing or accept proposals.","inputSchema":{"type":"object","additionalProperties":false,"required":["board_id","base_revision","description","edits_json"],"properties":{"board_id":board_id,"base_revision":{"type":"integer","minimum":0},"description":{"type":"string"},"edits_json":{"type":"string"}}}}));
    }
    tools
}
pub(super) fn operation(
    name: &str,
    mut arguments: serde_json::Map<String, Value>,
    read_only: bool,
) -> Result<ThreadOperation, String> {
    let action = match name {
        name if name == READS[0] => "list",
        name if name == READS[1] => "read",
        name if name == READS[2] => "status",
        WRITE if !read_only => "propose",
        _ => return Err("Unknown or read-only whiteboard tool.".into()),
    };
    arguments.insert("action".into(), json!(action));
    let request = serde_json::from_value(Value::Object(arguments))
        .map_err(|e| format!("Invalid whiteboard arguments: {e}"))?;
    Ok(ThreadOperation::Whiteboard {
        turn_id: String::new(),
        request,
    })
}
