use super::ToolError;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};

/// Capture budget fits one 1920x1080 RGBA image plus its PNG metadata and CDP
/// base64 framing (8.3 MiB raw, 11.1 MiB encoded). Larger responses fail loudly.
const OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy)]
pub struct ProcessPolicy {
    pub timeout: Duration,
    pub max_output_bytes: usize,
}

impl Default for ProcessPolicy {
    fn default() -> Self {
        Self {
            timeout: crate::capture_client::REQUEST_TIMEOUT,
            max_output_bytes: OUTPUT_BYTES,
        }
    }
}

pub async fn run(
    program: &str,
    args: &[String],
    policy: ProcessPolicy,
) -> Result<Vec<u8>, ToolError> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command.spawn().map_err(|error| {
        ToolError::Unavailable(format!(
            "Cannot start {program}: {error}. Install the tool and ensure it is on Bruhs' PATH."
        ))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::Unavailable(format!("{program} has no output pipe")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::Unavailable(format!("{program} has no error pipe")))?;
    let read = async move {
        let (out, err) = tokio::try_join!(
            bounded(stdout, policy.max_output_bytes),
            bounded(stderr, policy.max_output_bytes)
        )?;
        Ok::<_, ToolError>((out, err))
    };
    let (status, (out, err)) = tokio::time::timeout(policy.timeout, async {
        tokio::try_join!(async { child.wait().await.map_err(ToolError::Io) }, read)
    }).await.map_err(|_| ToolError::Unavailable(format!("{program} did not finish within {:?}. Check the connected device or browser and retry.", policy.timeout)))??;
    if !status.success() {
        return Err(ToolError::Unavailable(format!(
            "{program} failed ({status}): {}",
            String::from_utf8_lossy(&err)
        )));
    }
    Ok(out)
}

async fn bounded(
    mut read: impl tokio::io::AsyncRead + Unpin,
    max: usize,
) -> Result<Vec<u8>, ToolError> {
    let mut bytes = Vec::new();
    (&mut read)
        .take(u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1))
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > max {
        return Err(ToolError::Unavailable(format!(
            "Local tool output exceeded the configured {max} byte capture budget. Narrow the operation."
        )));
    }
    Ok(bytes)
}

pub fn path_exists(path: &str) -> bool {
    Path::new(path).is_file()
}

pub fn text(bytes: &[u8]) -> Result<String, ToolError> {
    String::from_utf8(bytes.to_vec())
        .map_err(|error| ToolError::Invalid(format!("Tool output was not UTF-8: {error}")))
}
