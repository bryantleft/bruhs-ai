use crate::protocol::ToolInfo;
use bruhs_models::AgentDomain;
use serde_json::{json, Value};

pub(super) fn operation_schema(domain: AgentDomain) -> Option<Value> {
    let title = json!({"type":"string","minLength":1,"maxLength":4096});
    let body = json!({"type":"string","minLength":1,"maxLength":65536});
    Some(match domain {
        AgentDomain::Chat => {
            json!({"type":"object","additionalProperties":false,"required":["type","title"],"properties":{"type":{"const":"create_chat_thread"},"title":title},"description":"Create an empty local chat thread with the reviewed title. Does not send messages, request model generation, or grant Chat Read."})
        }
        AgentDomain::Tasks => {
            json!({"type":"object","additionalProperties":false,"required":["type","title"],"properties":{"type":{"const":"create_task"},"title":title}})
        }
        AgentDomain::Calendar => {
            json!({"type":"object","additionalProperties":false,"required":["type","title","start_at_ms","end_at_ms"],"properties":{"type":{"const":"create_calendar_event"},"title":title,"start_at_ms":{"type":"integer","description":"Epoch milliseconds"},"end_at_ms":{"type":"integer","description":"Epoch milliseconds, after start"}}})
        }
        AgentDomain::Capture => {
            json!({"type":"object","additionalProperties":false,"required":["type","content"],"properties":{"type":{"const":"add_capture"},"content":body}})
        }
        AgentDomain::Knowledge => {
            json!({"type":"object","additionalProperties":false,"required":["type","title","body"],"properties":{"type":{"const":"create_knowledge_note"},"title":title,"body":body}})
        }
        AgentDomain::Inbox => {
            json!({"type":"object","additionalProperties":false,"required":["type","recipient","body"],"properties":{"type":{"const":"draft_inbox_message"},"recipient":title,"body":body}})
        }
        AgentDomain::People => {
            json!({"type":"object","additionalProperties":false,"required":["type","person_id","person","reminder"],"properties":{"type":{"const":"create_person_reminder"},"person_id":{"type":"string","minLength":1,"maxLength":128},"person":title,"reminder":body},"description":"Append an undated local reminder to the exact person ID and current reviewed display name. Does not notify, modify the person, contact anyone, or grant People Read. A missing or renamed person requires a new review."})
        }
        AgentDomain::Finance => {
            json!({"type":"object","additionalProperties":false,"required":["type","merchant","category","amount_minor"],"properties":{"type":{"const":"add_finance_transaction"},"merchant":title,"category":title,"amount_minor":{"type":"string","pattern":"^-?[0-9]+$","description":"Exact signed USD cents, range -9223372036854775807 to 9223372036854775807. Negative is spend. Local manual record only, not a payment; dated when executed."}}})
        }
        AgentDomain::Health => {
            json!({"type":"object","additionalProperties":false,"required":["type","activity","minutes"],"properties":{"type":{"const":"log_workout"},"activity":title,"minutes":{"type":"integer","minimum":1,"maximum":1440,"description":"Manual local log dated when executed, initial perceived exertion 5 of 10."}}})
        }
        AgentDomain::Meetings => {
            json!({"type":"object","additionalProperties":false,"required":["type","title"],"properties":{"type":{"const":"create_meeting_note"},"title":title,"body":{"type":"string","maxLength":65536,"default":""}},"description":"Create a local text meeting note with the exact reviewed title and body. Separate from recordings and generated summaries; does not capture audio, request generation, contact providers, or grant Meetings Read."})
        }
    })
}

pub(super) fn tool() -> ToolInfo {
    let title = json!({"type":"string","minLength":1,"maxLength":4096});
    let operations: Vec<_> = AgentDomain::ALL
        .into_iter()
        .filter_map(operation_schema)
        .collect();
    ToolInfo {
        name: "bruhs_propose_workflow".into(),
        description: "Preview an ordered cross-domain workflow, or submit it for one in-app user review with dry_run=false and a stable request_id. Never approves or executes. Every domain needs its own Write grant to submit. Connected native writes: empty local Chat threads, Tasks, local Calendar, Capture, Knowledge documents, Inbox drafts, manual USD ledger entries, workout logs, undated People reminders by exact ID and reviewed name, and local text Meeting notes. Chat writes do not send messages or request model generation. Meeting notes do not capture audio or generate summaries. Writes do not enable reads or connect providers. Whole encoded IPC request must fit 64 KiB. Check bruhs_workflow_status for saved progress; user controls execution and undo.".into(),
        input_schema: json!({
            "type":"object", "additionalProperties":false,
            "required":["title","operations"],
            "properties": {
                "title":title,
                "dry_run":{"type":"boolean","default":true},
                "request_id":{"type":"string","minLength":1,"maxLength":128},
                "operations":{"type":"array","minItems":1,"maxItems":128,"items":{"oneOf":operations}}
            }
        }),
    }
}
