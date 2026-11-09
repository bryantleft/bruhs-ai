# claude-sdk-rs

A small async Rust wrapper around the [Claude Code](https://docs.claude.com/en/docs/claude-code)
CLI. It spawns the local `claude` binary, parses its streaming JSON output, and
exposes it as a `Stream` of typed events. Authentication is whatever the local
Claude Code installation already uses (OAuth); no API key is handled here.

## Requirements

- The `claude` CLI installed and logged in, on `PATH`.
- A Tokio runtime.

## Example

```rust,no_run
use claude_sdk_rs::{query, PermissionMode, QueryEvent, QueryOptions};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() -> Result<(), claude_sdk_rs::Error> {
    let options = QueryOptions::new("What files are in the current directory?")
        .permission_mode(PermissionMode::Plan)
        .max_turns(3)
        // Keep the CLI on its OAuth login even if the environment has a key.
        .env_remove("ANTHROPIC_API_KEY");

    let mut stream = query(options)?;
    while let Some(event) = stream.next().await {
        match event {
            QueryEvent::Text(text) => print!("{text}"),
            QueryEvent::ProcessFailed(failure) => eprintln!("[claude failed: {failure}]"),
            _ => {}
        }
    }
    Ok(())
}
```

See `examples/basic.rs` for a fuller version.

## Behavior

- The prompt is passed after `--` (or on stdin with permission prompts), so a
  prompt such as `--help` or `-` is always text.
- Dropping the stream kills and reaps the CLI.
- A CLI that exits unsuccessfully ends the stream with
  `QueryEvent::ProcessFailed`, carrying the exit code and the tail of its
  stderr.
- Every `result` subtype (`success`, `error_max_turns`,
  `error_during_execution`, ...) arrives as `QueryEvent::Result`.
- With `QueryOptions::permission_prompts`, tool-permission prompts arrive as
  `QueryEvent::PermissionRequest` and are answered through
  `QueryStream::responder`.

## Migrating from 0.2

0.3.0 replaces `QueryOptions::allowed_tools: Option<Vec<String>>` with the
`Tools` enum (set with `QueryOptions::tools`):

| 0.2 | 0.3 |
| --- | --- |
| `allowed_tools: None` | `Tools::CliDefault` (the default) |
| `allowed_tools: Some(vec!["Read".into()])` | `Tools::Allow(vec!["Read".into()])` |
| `allowed_tools: Some(vec![])` (no tools) | `Tools::None` |

`Tools::Allow(vec![])` approves nothing extra and leaves the CLI's default
tool set in place; it is **not** tool-free. Use `Tools::None` for
transcript-only generation (no tools, `--restricted`, no inherited MCP
servers).

## License

MIT. See the `LICENSE` file.
