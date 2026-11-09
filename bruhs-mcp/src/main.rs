mod args;
mod protocol;
mod serve;
mod stdio_input;
mod tools;
#[cfg(unix)]
mod workspace_bridge;

use bruhs_core::AppState;
use serve::{Backend, ServeError};
use std::process::ExitCode;
use std::sync::Arc;
use stdio_input::StdioInput;
use tokio::io::BufReader;

/// Why the server cannot start or had to stop.
#[derive(Debug, thiserror::Error)]
enum Fatal {
    #[cfg(not(unix))]
    #[error(
        "The workspace connection is not supported on this platform. No fallback was enabled."
    )]
    BridgeUnsupported,
    #[cfg(unix)]
    #[error("The configured workspace connection is unavailable or not private. Enable a new connection in the app and copy its configuration.")]
    BridgeUnavailable,
    #[error(transparent)]
    Serve(#[from] ServeError),
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("bruhs-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Fatal> {
    let backend = backend()?;
    let input = StdioInput::new(BufReader::new(tokio::io::stdin()));
    let mut output = tokio::io::stdout();
    serve::serve(input, &mut output, &backend).await?;
    Ok(())
}

#[cfg(unix)]
fn backend() -> Result<Backend, Fatal> {
    match std::env::var_os("BRUHS_WORKSPACE_BRIDGE") {
        Some(path) => {
            bruhs_core::workspace_bridge::BridgeClient::from_file(std::path::Path::new(&path))
                .map(Backend::Bridge)
                // The cause is deliberately not echoed: it can name the private file.
                .map_err(|_| Fatal::BridgeUnavailable)
        }
        None => Ok(Backend::Standalone(Arc::new(AppState::new()))),
    }
}

#[cfg(not(unix))]
fn backend() -> Result<Backend, Fatal> {
    if std::env::var_os("BRUHS_WORKSPACE_BRIDGE").is_some() {
        return Err(Fatal::BridgeUnsupported);
    }
    Ok(Backend::Standalone(Arc::new(AppState::new())))
}
