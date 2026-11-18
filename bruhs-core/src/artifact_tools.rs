//! Provider-owned artifact tools publish snapshots; only the UI reports previews.

use std::io;

use crate::agent::ProviderQueryOptions;
use crate::artifacts::{ArtifactError, ArtifactStore};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
const HELPER: &str = "--artifact-mcp";
const SERVER: &str = "bruhs_artifacts";
const PROJECT_ENV: &str = "BRUHS_ARTIFACT_PROJECT_ROOT";
const INSTRUCTIONS: &str = "Publish local interactive HTML with publish_artifact(path,title). Local stylesheets, classic scripts, images, fonts and media are bundled into a durable snapshot. Dependencies must remain inside this turn's project or Bruhs attachments. Bundle JavaScript module imports before publishing; network requests, nested frames and submissions are not available. The tool result includes a bruhs-artifact fence that the app can display directly; repeat that fence in your final reply when useful. Publication is not a preview. Use inspect_artifact(id) after the UI displays it to read actual preview errors. If it says not yet previewed, say so; never claim a headless/browser verification or screenshot that did not happen.";

pub fn is_mcp_helper() -> bool {
    std::env::args_os().any(|argument| argument == HELPER)
}

pub(crate) fn configure(options: &mut ProviderQueryOptions, project: &str) -> io::Result<()> {
    let project = if project.trim().is_empty() {
        std::env::current_dir()?.to_string_lossy().into_owned()
    } else {
        project.to_string()
    };
    crate::harness_mcp::configure(
        options,
        SERVER,
        (std::env::current_exe()?, vec![HELPER.into()]),
        &["publish_artifact", "inspect_artifact"],
        std::collections::HashMap::from([(PROJECT_ENV.into(), project)]),
        INSTRUCTIONS,
    )
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
    crate::harness_mcp::serve(input, output, SERVER, INSTRUCTIONS, tools, |params| {
        call(store.clone(), project.clone(), params)
    })
    .await
}

#[cfg(test)]
mod tests;
