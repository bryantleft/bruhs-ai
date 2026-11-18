use crate::agent::ProviderQueryOptions;
use serde_json::{json, Value};
use std::future::Future;
use std::{collections::HashMap, io};
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};

const MAX_FRAME_BYTES: usize = 64 * 1024;
pub(crate) fn configure(
    options: &mut ProviderQueryOptions,
    server: &str,
    launch: (std::path::PathBuf, Vec<String>),
    names: &[&str],
    env: HashMap<String, String>,
    instructions: &str,
) -> io::Result<()> {
    let (executable, args) = launch;
    let executable = executable
        .to_str()
        .ok_or_else(|| io::Error::other("MCP executable path must be UTF-8."))?;
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
                server.into(),
                json!({"command":executable,"args":args,"env":env}),
            );
            options.mcp_config = Some(config.to_string());
            for name in names {
                let tool = format!("mcp__{server}__{name}");
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
            let vars = env
                .iter()
                .map(|(k, v)| format!("{k} = {}", json!(v)))
                .collect::<Vec<_>>()
                .join(", ");
            let tools = names
                .iter()
                .map(|name| format!("{name} = {{ approval_mode = \"approve\" }}"))
                .collect::<Vec<_>>()
                .join(", ");
            let config = format!("{{ command = {}, args = {}, env = {{ {vars} }}, required = true, tools = {{ {tools} }} }}",json!(executable),json!(args));
            options.config.push(
                codex_sdk_rs::ConfigOverride::new(format!("mcp_servers.{server}"), config)
                    .map_err(io::Error::other)?,
            );
        }
        ProviderQueryOptions::External(options) => {
            options.add_mcp(server, executable, &args, &env)?;
            options.append_instructions(instructions);
        }
    }
    Ok(())
}

pub(crate) async fn serve<
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    F: Future<Output = Result<String, E>>,
    E: std::fmt::Display,
>(
    input: R,
    output: &mut W,
    server: &str,
    instructions: &str,
    tools: fn() -> Value,
    call: impl Fn(Value) -> F,
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
                "Tool request exceeds the 64 KiB transport budget.",
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
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":server,"version":env!("CARGO_PKG_VERSION")},"instructions":instructions})
            }
            Some("ping") => json!({}),
            Some("tools/list") => tools(),
            Some("tools/call") => match call(request["params"].clone()).await {
                Ok(text) => json!({"content":[{"type":"text","text":text}]}),
                Err(error) => {
                    json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]})
                }
            },
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
