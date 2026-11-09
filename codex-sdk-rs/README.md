# codex-sdk-rs

A small async Rust wrapper around the Codex CLI. It spawns the local `codex`
binary with `codex exec --json`, parses the event stream, and exposes it as a
`Stream` of typed events. Authentication is whatever the local Codex CLI
already uses.

This crate is internal to the bruhs-ai workspace and is not published to
crates.io (`publish = false`).

See `examples/codex_basic.rs` for usage.

## License

MIT. See [LICENSE](LICENSE).
