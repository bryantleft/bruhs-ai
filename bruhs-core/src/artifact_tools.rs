//! Provider-owned artifact tools publish snapshots; only the UI reports previews.

use std::io;

use crate::agent::ProviderQueryOptions;
use crate::artifacts::{ArtifactError, ArtifactStore};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};

const HELPER: &str = "--artifact-mcp";
const SERVER: &str = "bruhs_artifacts";
const PROJECT_ENV: &str = "BRUHS_ARTIFACT_PROJECT_ROOT";
// Matches other app-owned path/ID-only MCP helpers. HTML bytes stay on disk.
const MAX_FRAME_BYTES: usize = 64 * 1024;
const INSTRUCTIONS: &str = "Publish local interactive HTML with publish_artifact(path,title). Local stylesheets, classic scripts, images, fonts and media are bundled into a durable snapshot. Dependencies must remain inside this turn's project or Bruhs attachments. Bundle JavaScript module imports before publishing; network requests, nested frames and submissions are not available. The tool result includes a bruhs-artifact fence that the app can display directly; repeat that fence in your final reply when useful. Publication is not a preview. Use inspect_artifact(id) after the UI displays it to read actual preview errors. If it says not yet previewed, say so; never claim a headless/browser verification or screenshot that did not happen.";

pub fn is_mcp_helper() -> bool {
    std::env::args_os().any(|argument| argument == HELPER)
}

pub(crate) fn configure(options: &mut ProviderQueryOptions, project: &str) -> io::Result<()> {
    let executable = std::env::current_exe()?.to_string_lossy().into_owned();
    let project = if project.trim().is_empty() {
        std::env::current_dir()?.to_string_lossy().into_owned()
    } else {
        project.to_string()
    };
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
                json!({"command":executable,"args":[HELPER],"env":{PROJECT_ENV:project}}),
            );
            options.mcp_config = Some(config.to_string());
            for name in ["publish_artifact", "inspect_artifact"] {
                let tool = format!("mcp__{SERVER}__{name}");
                match &mut options.tools {
                    claude_sdk_rs::Tools::CliDefault => {
                        options.tools = claude_sdk_rs::Tools::Allow(vec![tool])
                    }
                    claude_sdk_rs::Tools::Allow(tools) if !tools.contains(&tool) => {
                        tools.push(tool)
                    }
                    _ => {}
                }
            }
        }
        ProviderQueryOptions::Codex(options) => {
            let config = format!("{{ command = {}, args = [\"{HELPER}\"], env = {{ {PROJECT_ENV} = {} }}, required = true, tools = {{ publish_artifact = {{ approval_mode = \"approve\" }}, inspect_artifact = {{ approval_mode = \"approve\" }} }} }}", json!(executable), json!(project));
            options.config.push(
                codex_sdk_rs::ConfigOverride::new(format!("mcp_servers.{SERVER}"), config)
                    .map_err(io::Error::other)?,
            );
        }
        ProviderQueryOptions::External(options) => {
            options.add_mcp(
                SERVER,
                &executable,
                &[HELPER.into()],
                &std::collections::HashMap::from([(PROJECT_ENV.into(), project)]),
            )?;
            options.append_instructions(INSTRUCTIONS);
        }
    }
    Ok(())
}

pub async fn run_mcp_helper() -> io::Result<()> {
    let project = std::env::var(PROJECT_ENV).map_err(|_| {
        io::Error::other("Artifact tools require the current turn's project scope.")
    })?;
    let store = ArtifactStore::open_default().map_err(io::Error::other)?;
    serve(tokio::io::stdin(), &mut tokio::io::stdout(), store, project).await
}

fn tools() -> Value {
    json!({"tools":[
        {"name":"publish_artifact","description":"Bundle a local HTML file and its local CSS, classic JavaScript, image, font and media dependencies into a durable inline response. No remote fetching. Returns a bruhs-artifact display fence. Publication does not mean previewed; inspect_artifact reads actual UI feedback.","inputSchema":{"type":"object","additionalProperties":false,"required":["path"],"properties":{"path":{"type":"string"},"title":{"type":"string"}}}},
        {"name":"inspect_artifact","description":"Read actual client preview state and runtime errors for a published artifact. Until a client reports, returns not yet previewed. Does not run a browser or create a screenshot.","inputSchema":{"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"type":"string","format":"uuid"}}}}
    ]})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Publish {
    path: String,
    title: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inspect {
    id: String,
}

async fn call(
    store: ArtifactStore,
    project: String,
    params: Value,
) -> Result<String, ArtifactError> {
    tokio::task::spawn_blocking(move || match params["name"].as_str() {
        Some("publish_artifact") => {
            let input: Publish = serde_json::from_value(params.get("arguments").cloned().unwrap_or_else(|| json!({})))?;
            let artifact = store.publish(&input.path, input.title.as_deref(), Some(&project))?;
            let fence = serde_json::to_string(&json!({"id":artifact.id,"title":artifact.title}))?;
            Ok(format!("Published a durable self-contained artifact. Preview: not yet previewed. The app can render the block below; use inspect_artifact after it reports actual runtime diagnostics.\n\n```bruhs-artifact\n{fence}\n```"))
        }
        Some("inspect_artifact") => {
            let input: Inspect = serde_json::from_value(params.get("arguments").cloned().unwrap_or_else(|| json!({})))?;
            let artifact = store.load(&input.id)?;
            let diagnostics = store.load_diagnostics(&input.id)?;
            Ok(serde_json::to_string(&json!({"id":artifact.id,"title":artifact.title,"preview":if diagnostics.previewed_at_ms.is_none() {"not yet previewed"} else {"client reported"},"diagnostics":diagnostics}))?)
        }
        _ => Err(ArtifactError::Invalid("Unknown artifact tool.".into())),
    }).await.map_err(|error| ArtifactError::Invalid(format!("Artifact operation could not finish: {error}")))?
}

async fn serve<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: R,
    output: &mut W,
    store: ArtifactStore,
    project: String,
) -> io::Result<()> {
    let mut reader = BufReader::new(input);
    loop {
        let mut frame = Vec::new();
        let read = (&mut reader)
            .take(u64::try_from(MAX_FRAME_BYTES).map_err(io::Error::other)? + 1)
            .read_until(b'\n', &mut frame)
            .await?;
        if read == 0 {
            return Ok(());
        }
        if frame.len() > MAX_FRAME_BYTES {
            return Err(io::Error::other(
                "Artifact tool request exceeds the 64 KiB path/ID transport budget.",
            ));
        }
        let request: Value = match serde_json::from_slice(&frame) {
            Ok(request) => request,
            Err(_) => {
                write(output, &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Invalid JSON"}})).await?;
                continue;
            }
        };
        let Some(id) = request.get("id") else {
            continue;
        };
        let result = match request["method"].as_str() {
            Some("initialize") => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":SERVER,"version":env!("CARGO_PKG_VERSION")},"instructions":INSTRUCTIONS})
            }
            Some("ping") => json!({}),
            Some("tools/list") => tools(),
            Some("tools/call") => {
                match call(store.clone(), project.clone(), request["params"].clone()).await {
                    Ok(text) => json!({"content":[{"type":"text","text":text}]}),
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

async fn write<W: AsyncWrite + Unpin>(output: &mut W, value: &Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    bytes.push(b'\n');
    output.write_all(&bytes).await?;
    output.flush().await
}

#[cfg(test)]
mod tests;
