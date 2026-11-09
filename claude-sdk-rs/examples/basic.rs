//! Ask the local `claude` CLI a question and print the streamed answer.
use claude_sdk_rs::{query, PermissionMode, QueryEvent, QueryOptions, Tools};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), claude_sdk_rs::Error> {
    let options = QueryOptions::new("What files are in the current directory?")
        .tools(Tools::Allow(vec![
            "Bash".into(),
            "Read".into(),
            "Glob".into(),
        ]))
        .permission_mode(PermissionMode::Plan)
        .max_turns(3);

    let mut stream = query(options)?;
    while let Some(event) = stream.next().await {
        match event {
            QueryEvent::Text(text) => print!("{text}"),
            QueryEvent::ToolCall { tool, .. } => println!("\n[Tool: {tool}]"),
            QueryEvent::Result { duration_ms, .. } => println!("\n[Done in {duration_ms}ms]"),
            QueryEvent::ProcessFailed(failure) => eprintln!("[claude failed: {failure}]"),
            QueryEvent::Error(error) => eprintln!("[Error: {error}]"),
            _ => {}
        }
    }
    Ok(())
}
