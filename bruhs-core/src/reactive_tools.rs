//! Conversational automation setup. A model can propose changes, never approve them.

use crate::agent::ProviderQueryOptions;
use bruhs_models::reactive::{ReactiveChange, ReactiveCommand, MAX_PAYLOAD_BYTES};
use serde_json::{json, Value};
use std::io;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

const HELPER: &str = "--reactive-mcp";
const ENDPOINT_ENV: &str = "BRUHS_REACTIVE_ENDPOINT";
const READ_ONLY_ENV: &str = "BRUHS_REACTIVE_READ_ONLY";
const SERVER: &str = "bruhs_reactive";
const TURN_HINT: &str = "For ongoing automation, reactive_status inspects current setup; reactive_propose prepares a complete save_automation proposal for one owner review. Bundle its agent, source and subscription together, enabled=true and auto_run=true so events run without a per-event review. A proposal is inactive until approved. Writes use separate workflow review. Use reactive_inspect before editing existing setup or memory.";
const INSTRUCTIONS: &str = "Bruhs has persistent reactive agents. For a new automation use reactive_propose with change.type=save_automation and automation={agent,source,subscription}. Choose distinct stable IDs; subscription.source_id and agent_id must match those exact bundled IDs. Set agent/source/subscription enabled=true and subscription.auto_run=true so events run without a per-event review after one approval. The owner reviews the complete purpose, watched input, model, memory and access once in Automations. Until approved nothing is configured or running. Never submit separate setup proposals for each component of a new automation. Individual changes remain advanced settings. Use reactive_status and reactive_inspect to read existing setup before editing; retain saved memory unless the owner requests replacement. Source types: folder(path,pattern,recursive,settle_ms,poll_ms), http(url,poll_ms), command(command,cwd,poll_ms,timeout_ms), schedule(every_ms), calendar(times as HH:MM, weekdays 1-7 with Monday=1, time_zone as an IANA name; use this for wall-clock times like weekdays at 08:00), webhook, app_event(topic), custom. Subscriptions take on_busy=queue (default; events arriving mid-run are handled together next) or restart. Event agents run at their access level: Read only by default (they read and draft), Ask first asks the owner before each change, Full works in its own folder. The owner sets the level on the bruh's page. Writes to app records use existing workflow grants and review. Never put credentials in configuration or promise native banking/MQTT/FTP integrations. Agents use existing local Claude/Codex logins. Source commands run as the server account: explain the exact command in the proposal.";

pub fn is_mcp_helper() -> bool {
    std::env::args_os().any(|arg| arg == HELPER)
}

pub(crate) fn endpoint() -> io::Result<String> {
    let address = std::env::var(ENDPOINT_ENV)
        .or_else(|_| std::env::var("BRUHS_SERVER_URL"))
        .unwrap_or_else(|_| {
            let port = std::env::var("BRUHS_SERVER_PORT").unwrap_or_else(|_| "3001".into());
            format!("http://127.0.0.1:{port}")
        });
    let parsed = url::Url::parse(&address).map_err(io::Error::other)?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host == "localhost"
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !loopback
        || parsed.scheme() != "http"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(io::Error::other(
            "Reactive setup tools require the local companion server over loopback HTTP.",
        ));
    }
    Ok(address.trim_end_matches('/').to_string())
}

pub(crate) fn configure(options: &mut ProviderQueryOptions, read_only: bool) -> io::Result<()> {
    let address = endpoint()?;
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let env = json!({ENDPOINT_ENV: address, READ_ONLY_ENV: if read_only {"1"} else {"0"}});
    match options {
        ProviderQueryOptions::Claude(options) => {
            let mut config = options
                .mcp_config
                .as_deref()
                .map(serde_json::from_str::<Value>)
                .transpose()
                .map_err(io::Error::other)?
                .unwrap_or_else(|| json!({}));
            let servers = config
                .as_object_mut()
                .map(|config| config.entry("mcpServers").or_insert_with(|| json!({})))
                .and_then(Value::as_object_mut)
                .ok_or_else(|| io::Error::other("MCP configuration must be an object."))?;
            servers.insert(
                SERVER.into(),
                json!({"command":executable,"args":[HELPER],"env":env}),
            );
            options.mcp_config = Some(config.to_string());
            if let Some(instructions) = &mut options.append_system_prompt {
                instructions.push_str("\n\n");
                instructions.push_str(TURN_HINT);
            } else {
                options.append_system_prompt = Some(TURN_HINT.into());
            }
            for name in ["reactive_status", "reactive_inspect"] {
                let tool = format!("mcp__{SERVER}__{name}");
                match &mut options.tools {
                    claude_sdk_rs::Tools::CliDefault => {
                        options.tools = claude_sdk_rs::Tools::Allow(vec![tool])
                    }
                    claude_sdk_rs::Tools::Allow(tools) => tools.push(tool),
                    _ => {}
                }
            }
        }
        ProviderQueryOptions::Codex(options) => {
            let settings = format!(
                "{{ command = {}, args = [\"{HELPER}\"], env = {{ {ENDPOINT_ENV} = {}, {READ_ONLY_ENV} = {} }}, tools = {{ reactive_status = {{ approval_mode = \"approve\" }}, reactive_inspect = {{ approval_mode = \"approve\" }}{} }} }}",
                json!(executable),
                json!(address),
                json!(if read_only { "1" } else { "0" }),
                if read_only {
                    ""
                } else {
                    ", reactive_propose = { approval_mode = \"approve\" }"
                }
            );
            options.config.push(
                codex_sdk_rs::ConfigOverride::new(format!("mcp_servers.{SERVER}"), settings)
                    .map_err(io::Error::other)?,
            );
        }
        ProviderQueryOptions::External(options) => {
            options.add_mcp(
                SERVER,
                &executable,
                &[HELPER.into()],
                &std::collections::HashMap::from([
                    (ENDPOINT_ENV.into(), address),
                    (
                        READ_ONLY_ENV.into(),
                        if read_only { "1".into() } else { "0".into() },
                    ),
                ]),
            )?;
            options.append_instructions(TURN_HINT);
        }
    }
    Ok(())
}

pub async fn run_mcp_helper() -> io::Result<()> {
    let address = endpoint()?;
    let read_only = std::env::var(READ_ONLY_ENV).as_deref() != Ok("0");
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(io::Error::other)?;
    serve(
        tokio::io::stdin(),
        &mut tokio::io::stdout(),
        &client,
        &address,
        read_only,
    )
    .await
}

fn tools(read_only: bool) -> Value {
    let mut entries = vec![
        json!({
            "name":"reactive_status", "description":"Inspect server-owned persistent agents, sources, subscriptions, draft runs and pending setup proposals. No mutation. Server must be running.",
            "inputSchema":{"type":"object","additionalProperties":false,"properties":{}}
        }),
        json!({
            "name":"reactive_inspect", "description":"Read the exact configuration, owner-approved memory, event, run or pending proposal by its ID from reactive_status. Read existing agent memory before proposing a replacement. No credentials are returned.",
            "inputSchema":{"type":"object","additionalProperties":false,"required":["kind","id"],"properties":{"kind":{"type":"string","enum":["agents","sources","subscriptions","events","runs","proposals","outbox","follow_ups"]},"id":{"type":"string"}}}
        }),
    ];
    if !read_only {
        entries.push(json!({
            "name":"reactive_propose", "description":"Propose a complete automation with save_automation for one owner review. Bundle matching agent/source/subscription references, enabled=true and auto_run=true so events run without a per-event review after approval. Never applies or approves setup. Individual changes are advanced settings. Command sources run as the server account: explain the exact command before proposing.",
            "inputSchema":{"type":"object","additionalProperties":false,"required":["change"],"properties":{"change":{"oneOf":[
                {"type":"object","additionalProperties":false,"required":["type","agent"],"properties":{"type":{"const":"save_agent"},"agent":{"type":"object","additionalProperties":false,"required":["id","name","instructions","model","project_path"],"properties":{"id":{"type":"string"},"name":{"type":"string"},"instructions":{"type":"string"},"memory":{"type":"string"},"model":{"type":"string"},"project_path":{"type":"string"},"enabled":{"type":"boolean","default":false}}}}},
                {"type":"object","additionalProperties":false,"required":["type","source"],"properties":{"type":{"const":"save_source"},"source":{"type":"object","additionalProperties":false,"required":["id","name","kind"],"properties":{"id":{"type":"string"},"name":{"type":"string"},"enabled":{"type":"boolean","default":false},"kind":{"type":"object","description":"Tagged type: folder{path,pattern,recursive,settle_ms,poll_ms}, http{url,poll_ms}, command{command,cwd,poll_ms,timeout_ms}, schedule{every_ms}, calendar{times:[\"HH:MM\"],weekdays:[1-7, Monday=1, empty=every day],time_zone:IANA name}, webhook, app_event{topic}, custom."}}}}},
                {"type":"object","additionalProperties":false,"required":["type","subscription"],"properties":{"type":{"const":"save_subscription"},"subscription":{"type":"object","additionalProperties":false,"required":["id","source_id","agent_id","instructions"],"properties":{"id":{"type":"string"},"source_id":{"type":"string"},"agent_id":{"type":"string"},"instructions":{"type":"string"},"enabled":{"type":"boolean","default":false},"auto_run":{"type":"boolean","default":false},"debounce_ms":{"type":"integer","minimum":0},"throttle_ms":{"type":"integer","minimum":0},"on_busy":{"type":"string","enum":["queue","restart"],"default":"queue","description":"queue: events that arrive mid-run are handled together next run. restart: stop the running turn and redo it with the new event."}}}}},
                {"type":"object","additionalProperties":false,"required":["type","agent_id","memory"],"properties":{"type":{"const":"set_memory"},"agent_id":{"type":"string"},"memory":{"type":"string"}}},
                {"type":"object","additionalProperties":false,"required":["type","id"],"properties":{"type":{"enum":["delete_agent","delete_source","delete_subscription"]},"id":{"type":"string"}}}
            ]}}}
        }));
        // Compose the complete setup schema from the same component schemas
        // used by advanced changes, so the two forms cannot drift apart.
        if let Some(variants) = entries
            .last_mut()
            .and_then(|tool| tool["inputSchema"]["properties"]["change"]["oneOf"].as_array_mut())
        {
            let mut agent = variants[0]["properties"]["agent"].clone();
            let mut source = variants[1]["properties"]["source"].clone();
            let mut subscription = variants[2]["properties"]["subscription"].clone();
            for component in [&mut agent, &mut source, &mut subscription] {
                component["properties"]["enabled"]["default"] = json!(true);
                if let Some(required) = component["required"].as_array_mut() {
                    required.push(json!("enabled"));
                }
            }
            subscription["properties"]["auto_run"]["default"] = json!(true);
            if let Some(required) = subscription["required"].as_array_mut() {
                required.push(json!("auto_run"));
            }
            variants.insert(0,json!({"type":"object","additionalProperties":false,"required":["type","automation"],"properties":{"type":{"const":"save_automation"},"automation":{"type":"object","additionalProperties":false,"required":["agent","source","subscription"],"properties":{"agent":agent,"source":source,"subscription":subscription}}}}));
        }
    }
    json!({"tools":entries})
}

async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: R,
    output: &mut W,
    client: &reqwest::Client,
    address: &str,
    read_only: bool,
) -> io::Result<()> {
    let mut reader = BufReader::new(input);
    loop {
        let mut frame = Vec::new();
        let read = (&mut reader)
            .take(u64::try_from(MAX_PAYLOAD_BYTES).map_err(io::Error::other)? + 1)
            .read_until(b'\n', &mut frame)
            .await?;
        if read == 0 {
            return Ok(());
        }
        if frame.len() > MAX_PAYLOAD_BYTES {
            return Err(io::Error::other(
                "Automation tool request exceeds the 64 KiB transport budget.",
            ));
        }
        let request: Value = match serde_json::from_slice(&frame) {
            Ok(request) => request,
            Err(_) => {
                write(output, &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})).await?;
                continue;
            }
        };
        if request.get("id").is_none() {
            continue;
        }
        let id = request["id"].clone();
        let result = match request["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":SERVER,"version":env!("CARGO_PKG_VERSION")},"instructions":INSTRUCTIONS})
            }
            Some("ping") => json!({}),
            Some("tools/list") => tools(read_only),
            Some("tools/call") => {
                match call(client, address, read_only, &request["params"]).await {
                    Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}]}),
                    Err(error) => {
                        json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]})
                    }
                }
            }
            _ => {
                write(output, &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}})).await?;
                continue;
            }
        };
        write(output, &json!({"jsonrpc":"2.0","id":id,"result":result})).await?;
    }
}

async fn call(
    client: &reqwest::Client,
    address: &str,
    read_only: bool,
    params: &Value,
) -> io::Result<Value> {
    let response = match params["name"].as_str() {
        Some("reactive_status" | "reactive_inspect") => {
            client.get(format!("{address}/api/reactive")).send().await
        }
        Some("reactive_propose") if !read_only => {
            let change: ReactiveChange =
                serde_json::from_value(params["arguments"]["change"].clone())
                    .map_err(io::Error::other)?;
            client
                .post(format!("{address}/api/reactive/commands"))
                .json(&ReactiveCommand::Propose { change })
                .send()
                .await
        }
        _ => {
            return Err(io::Error::other(
                "Tool is not available in this turn. Plan mode can only inspect automations.",
            ));
        }
    }
    .map_err(|_| {
        io::Error::other(
            "The local automation server is unavailable. Start bruhs-server and retry.",
        )
    })?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response_error(response).await;
        return Err(io::Error::other(format!(
            "Automation request failed ({status}): {}",
            detail.as_deref().unwrap_or(
                "The local server returned an invalid error response. Open Workflows and retry."
            )
        )));
    }
    let snapshot: Value = response.json().await.map_err(io::Error::other)?;
    if params["name"] == "reactive_status" {
        return Ok(status_projection(&snapshot));
    }
    if params["name"] == "reactive_inspect" {
        let kind = params["arguments"]["kind"]
            .as_str()
            .filter(|kind| {
                [
                    "agents",
                    "sources",
                    "subscriptions",
                    "events",
                    "runs",
                    "proposals",
                    "outbox",
                    "follow_ups",
                ]
                .contains(kind)
            })
            .ok_or_else(|| {
                io::Error::other("Choose an automation record kind from the tool schema.")
            })?;
        let id = params["arguments"]["id"]
            .as_str()
            .ok_or_else(|| io::Error::other("A record ID is required."))?;
        let record = snapshot[kind].as_array().and_then(|records| records.iter().find(|record| record["id"].as_str() == Some(id))).cloned().ok_or_else(|| io::Error::other("That record is not in the current automation snapshot. Refresh reactive_status or open Workflows for retained history."))?;
        if record.to_string().len() > MAX_PAYLOAD_BYTES {
            return Err(io::Error::other(
                "This record exceeds the private tool response budget. Review it in Workflows.",
            ));
        }
        return Ok(record);
    }
    let proposal = snapshot["proposals"].as_array().and_then(|proposals| proposals.iter().find(|proposal| proposal["status"] == "pending" && proposal["change"] == params["arguments"]["change"])).ok_or_else(|| io::Error::other("The local server did not acknowledge the exact setup proposal. Refresh reactive_status before retrying."))?;
    Ok(
        json!({"proposal_id":proposal["id"],"status":"pending","notice":"Setup proposal saved for owner review. No configuration was applied. Use reactive_inspect with kind=proposals for the exact change."}),
    )
}

fn short(value: &Value) -> String {
    // This is a navigation preview; reactive_inspect retains exact review data.
    value.as_str().unwrap_or("").chars().take(256).collect()
}

async fn response_error(mut response: reqwest::Response) -> Option<String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len().saturating_add(chunk.len()) > MAX_PAYLOAD_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value["error"]
        .as_str()
        .map(|message| message.chars().take(2_000).collect())
}

fn status_projection(snapshot: &Value) -> Value {
    let project = |kind: &str, fields: &[&str]| {
        snapshot[kind]
            .as_array()
            .into_iter()
            .flatten()
            .map(|record| {
                let mut item = serde_json::Map::new();
                for field in fields {
                    item.insert((*field).into(), record[*field].clone());
                }
                if record.get("name").is_some() {
                    item.insert("name".into(), json!(short(&record["name"])));
                }
                Value::Object(item)
            })
            .collect::<Vec<_>>()
    };
    let runs = snapshot["runs"].as_array().into_iter().flatten().map(|run| json!({"id":run["id"],"agent_id":run["agent_id"],"status":run["status"],"created_at":run["created_at"],"result_preview":short(&run["result"]),"error_preview":short(&run["error"])})).collect::<Vec<_>>();
    let proposals = snapshot["proposals"].as_array().into_iter().flatten().map(|proposal| json!({"id":proposal["id"],"status":proposal["status"],"change_type":proposal["change"]["type"]})).collect::<Vec<_>>();
    json!({
        "agents":project("agents", &["id","model","enabled","project_path"]),
        "sources":project("sources", &["id","enabled"]),
        "subscriptions":project("subscriptions", &["id","source_id","agent_id","enabled","auto_run"]),
        "runs":runs,"proposals":proposals,
        "outbox":snapshot["outbox"].as_array().into_iter().flatten().map(|item| json!({"id":item["id"],"agent_id":item["agent_id"],"kind":item["kind"],"title":short(&item["title"]),"read":!item["read_at"].is_null(),"resolved":!item["resolved_at"].is_null()})).collect::<Vec<_>>(),
        "follow_ups":snapshot["follow_ups"].as_array().into_iter().flatten().filter(|item| item["status"]=="pending").map(|item| json!({"id":item["id"],"agent_id":item["agent_id"],"key":item["key"],"due_at":item["due_at"]})).collect::<Vec<_>>(),
        "source_statuses":snapshot["source_statuses"],
        "notice":"Navigation summary. Use reactive_inspect with kind and id for exact instructions, memory, source configuration, event data or run output before proposing changes. Previews are shortened; no setup is applied by these tools."
    })
}

async fn write<W: AsyncWrite + Unpin>(output: &mut W, value: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    if bytes.len() > MAX_PAYLOAD_BYTES {
        bytes = serde_json::to_vec(&json!({"jsonrpc":"2.0","id":value["id"],"error":{"code":-32603,"message":"The automation response exceeds the 64 KiB private tool budget. Open Workflows to review the full record."}})).map_err(io::Error::other)?;
    }
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_catalog_cannot_propose_or_approve() {
        let catalog = tools(true);
        let entries = catalog["tools"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["name"], "reactive_status");
        assert!(!tools(false).to_string().contains("decide_proposal"));
    }

    #[test]
    fn conversational_setup_exposes_one_complete_automatic_read_only_proposal() {
        let catalog = tools(false);
        let propose = catalog["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "reactive_propose")
            .unwrap();
        let setup = &propose["inputSchema"]["properties"]["change"]["oneOf"][0];
        assert_eq!(setup["properties"]["type"]["const"], "save_automation");
        let properties = &setup["properties"]["automation"]["properties"];
        for component in ["agent", "source", "subscription"] {
            assert_eq!(
                properties[component]["properties"]["enabled"]["default"],
                true
            );
            assert!(properties[component]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("enabled")));
        }
        assert_eq!(
            properties["subscription"]["properties"]["auto_run"]["default"],
            true
        );
        assert!(properties["subscription"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("auto_run")));
        assert!(!propose.to_string().contains("decide_proposal"));
    }

    #[tokio::test]
    async fn no_tool_can_submit_an_apply_command() {
        let response = call(
            &reqwest::Client::new(),
            "http://127.0.0.1:1",
            false,
            &json!({"name":"reactive_apply","arguments":{}}),
        )
        .await;
        assert!(response.unwrap_err().to_string().contains("not available"));
    }

    #[test]
    fn status_does_not_repeat_event_payloads_memory_or_full_outputs() {
        let snapshot = json!({"agents":[{"id":"a","name":"Agent","memory":"private memory","instructions":"long instructions"}],"events":[{"payload":"large scan"}],"runs":[{"id":"r","result":"x".repeat(16_000)}]});
        let status = status_projection(&snapshot);
        let text = status.to_string();
        assert!(!text.contains("private memory"));
        assert!(!text.contains("large scan"));
        assert!(text.len() < 2_000);
    }

    #[tokio::test]
    async fn validation_messages_return_to_the_agent_without_an_opaque_status() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(400)
                    .set_body_json(json!({"error":"Choose an existing absolute folder path."})),
            )
            .mount(&server)
            .await;
        let error=call(&reqwest::Client::new(),&server.uri(),false,&json!({"name":"reactive_propose","arguments":{"change":{"type":"save_agent","agent":{"id":"a","name":"Agent","instructions":"Draft","memory":"","model":"sonnet","project_path":"missing","enabled":false}}}})).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("Choose an existing absolute folder path"));
    }

    #[tokio::test]
    async fn proposal_acknowledgment_does_not_dump_other_pending_changes() {
        let server = wiremock::MockServer::start().await;
        let change =
            json!({"type":"set_memory","agent_id":"a","memory":"Owner-approved preference"});
        wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({"proposals":[{"id":"new","status":"pending","change":change.clone()},{"id":"old","status":"pending","change":{"type":"set_memory","agent_id":"b","memory":"x".repeat(16_000)}}]}))).mount(&server).await;
        let result = call(
            &reqwest::Client::new(),
            &server.uri(),
            false,
            &json!({"name":"reactive_propose","arguments":{"change":change}}),
        )
        .await
        .unwrap();
        assert_eq!(result["proposal_id"], "new");
        assert!(result.to_string().len() < 512);
    }

    #[tokio::test]
    async fn oversized_model_responses_fail_with_the_original_request_id() {
        let mut output = Vec::new();
        write(
            &mut output,
            &json!({"jsonrpc":"2.0","id":7,"result":{"text":"x".repeat(MAX_PAYLOAD_BYTES)}}),
        )
        .await
        .unwrap();
        let response: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["error"]["code"], -32603);
        assert!(output.len() < MAX_PAYLOAD_BYTES);
    }
}
