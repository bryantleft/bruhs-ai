# Pith storage connection

bruhs-ai can ask the local Pith scanner what takes space, which cleanup candidates are worth reviewing, and what deleting a proposed set would actually free. This works with Claude and Codex on both app surfaces, including project-less lean turns. No provider-wide MCP settings, API keys, listening ports, or file-content access are needed for the connection.

## Connect

Build Pith in the sibling bruhs checkout (`pnpm build:pith` there), then from this checkout:

```sh
cargo run -p bruhs-core --example connect_pith -- \
  --command /absolute/path/to/Pith.app/Contents/MacOS/storage-scanner \
  --root /Users/your-name
```

Choose the scope explicitly. A project folder or Downloads grants less access than the whole home. The example prints the dedicated `pith-mcp.json` file it saved in bruhs-ai's platform data directory. Alternatively, save the JSON copied by **Pith > Settings > Connections > Copy MCP Configuration** to that file. The app does not load arbitrary server commands from chat requests or project files.

The connection file is read at the next turn, so a running app need not restart. Start a fresh conversation after changing or removing the connection, to keep earlier storage evidence out of the new conversation. Remove this file to stop adding Pith to later turns. An active turn retains its launched server until that turn ends; stop the turn as well when disconnecting. This does not change MCP servers separately configured in the provider itself. `BRUHS_PITH_CONFIG=/absolute/path/to/pith-mcp.json` selects an explicit file for a development server; an explicit missing or invalid file fails the turn with an actionable error. An absent default file leaves Pith off.

The configuration format is exactly Pith's copied JSON:

```json
{
  "mcpServers": {
    "pith": {
      "command": "/Applications/Pith.app/Contents/MacOS/storage-scanner",
      "args": ["mcp", "--root", "/Users/your-name/Downloads"]
    }
  }
}
```

Only the five Pith read tools are pre-approved. Existing private credential and thread servers, user permissions, and provider instructions are preserved. Transcript-only Claude runs remain tool-free. The connection file is written atomically with private permissions. It must contain an absolute scanner command and exactly `mcp --root /absolute/folder`; extra server, environment, or command fields are rejected. Keep the app at the configured location or reconnect after moving it.

## Ask about storage

Ask **“Use Pith to scan my project folder and recommend what to clean up. Inspect the rebuildable items, explain their risks, and measure what the proposed set would free. Leave everything in place.”**

The assistant uses `pith_status`, `pith_scan`, `pith_cleanup_candidates`, `pith_inspect_path`, and `pith_estimate_cleanup`. Pith returns point-in-time metadata, coverage gaps, reasons, risks, and APFS-aware estimates. Every candidate page requires the exact `scan_id` returned by the intended scan. Folder inspection returns its own ID and never changes earlier evidence. IDs survive reconnects, are bound to the exact configured folder scope, and expire after 24 hours or earlier cache eviction. Unknown or unavailable IDs require a fresh scan; there is no implicit last scan. Its configured folder is the boundary for its tools; it does not expand bruhs-ai's normal permissions. A Pith failure is reported rather than replaced with shell-based guesses.

This connection cannot move or delete files. Review the proposed paths in Pith's Disk tab and use its existing confirmation and Undo. Moving to Trash alone does not free space, and shared blocks or snapshots can retain bytes. Connecting shares names, paths, sizes and cleanup reasons with the chosen assistant provider. No user file contents are returned by Pith. Pith retains scan metadata privately in `~/Library/Caches/app.bruhs.pith/mcp-reports` (0700 directory, 0600 files), with a 64 MiB budget for retained report blocks and an 8 MiB serialized limit per report. Directory/lock metadata and one report being written are outside the retained-report budget. Expired reports are removed on later scans or when requested. Removing the cache invalidates its IDs. Oversized reports fail explicitly and request a narrower scan. macOS may require privacy access for the provider CLI's host separately from the Pith app; unreadable locations remain explicit in the results. Provider tool deadlines still apply, so prefer a narrow scan when a large one cannot finish in time.

## Live acceptance test

Start a local bruhs-ai server with the connection enabled. This uses the signed-in local provider and spends real tokens:

```sh
cargo run -p bruhs-server
python3 scripts/test-pith-agent.py --server-url http://127.0.0.1:3001
# Optionally exercise the other provider using a model from bruhs-ai's model catalog:
python3 scripts/test-pith-agent.py --server-url http://127.0.0.1:3001 --model your-codex-model
```

The connection scope must be exactly the home directory for this test, since it makes a disposable fixture there and uses `/` as an outside-scope probe. The test checks `pith_status` first and refuses a broader or narrower scope before further filesystem probes. The model is asked to scan only that fixture. The test goes through the real `/api/agent` stream, provider CLI, and Pith MCP server. It requires successful results from all five Pith tools, the fixture scan’s exact ID on candidate requests and in the answer, rebuildable output ranked ahead of a larger personal file, exact allocated and reclaimable numbers copied into the final answer, an overlapping selection counted once, an actual refusal to inspect `/`, and every fixture file unchanged. It fails on non-Pith tool use or unexpected approval requests. No scan of the person's home or file contents is requested. The fixture is removed afterwards; the receipt and streamed evidence are saved under the system temporary directory (override with `--out`). The default model is the app's current default, not one hardcoded by the test.

`cargo test -p bruhs-core pith::tests` verifies configuration validation, private atomic saves, removal, lean Claude configuration, preservation of both private brokers, Codex configuration, and tool-free transcript runs without spending tokens.
