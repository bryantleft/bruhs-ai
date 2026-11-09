//! Ask the local `codex` CLI a question in a read-only sandbox.
use codex_sdk_rs::{query, ApprovalPolicy, CodexAccess, QueryEvent, QueryOptions, SandboxMode};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), codex_sdk_rs::Error> {
    let options = QueryOptions::new("What files are in the current directory?")
        .access(CodexAccess::Sandboxed {
            sandbox: SandboxMode::ReadOnly,
            approval: ApprovalPolicy::Never,
        })
        .skip_git_repo_check(true);

    let mut stream = query(options)?;
    while let Some(event) = stream.next().await {
        match event {
            QueryEvent::Init { thread_id } => println!("[Thread: {thread_id}]"),
            QueryEvent::Text(text) => print!("{text}"),
            QueryEvent::ToolCall { tool, .. } => println!("\n[Tool: {tool}]"),
            QueryEvent::Usage(usage) => println!(
                "\n[Tokens: {} in, {} out]",
                usage.input_tokens, usage.output_tokens
            ),
            QueryEvent::ProcessFailed(failure) => eprintln!("[codex failed: {failure}]"),
            QueryEvent::Error(error) => eprintln!("[Error: {error}]"),
            _ => {}
        }
    }
    Ok(())
}
