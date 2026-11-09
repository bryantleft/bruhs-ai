//! Live check of per-turn usage against the installed `codex` (spends tokens):
//! `cargo run -p codex-sdk-rs --example live_usage -- <dir>`.
use codex_sdk_rs::{query, QueryEvent, QueryOptions};
use tokio_stream::StreamExt;

#[tokio::main]
async fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let mut thread = None;
    for prompt in ["Reply with only: ok", "Reply with only: ok2"] {
        let mut options = QueryOptions::new(prompt)
            .cwd(&dir)
            .skip_git_repo_check(true);
        if let Some(id) = &thread {
            options = options.resume(id);
        }
        let mut stream = query(options).expect("spawn codex");
        while let Some(event) = stream.next().await {
            match event {
                QueryEvent::Init { thread_id } => thread = Some(thread_id),
                QueryEvent::Usage(usage) => println!("{prompt:?}: {usage:?}"),
                _ => {}
            }
        }
    }
}
