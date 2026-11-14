# CLI usage history and ACP agents

Open **Workbench → Usage** to inspect measured Bruhs turns and privately import
Claude Code or Codex token metadata. API-equivalent prices use your configured
rates; these estimates do not represent subscription bills. Unknown usage or
prices remain unknown.

## Import local history

Choose the source and optionally its CLI data directory. Defaults respect
`CLAUDE_CONFIG_DIR` and `CODEX_HOME`; Bruhs reads their `projects` or `sessions`
subdirectory. The local API confines the selected directory to your home, and
the importer rejects escaping symlinks and special files.

Select a file count and scan-byte budget, then **Preview usage import**. Review
the source, token totals and warnings before **Import reviewed metadata**.
Prompts, tool inputs and outputs are never copied into the imported ledger or
sent to the browser.

Each page includes at most 50 files, matching the host's existing history-page
limit. An incomplete file is excluded and retried from its beginning; increase
the scan budget to the displayed minimum. Later pages carry the first page's
file inventory hash. A changed inventory requires a fresh first page.

Changing any import option invalidates the preview. Only one frozen preview is
retained, and committing consumes it. The last committed preview can be retried
without adding usage twice. Separate companion processes cannot write the ledger
concurrently. Preview and cumulative ledger metadata each have a 50 MiB budget;
an oversized commit preserves the previous ledger and reviewed preview.

Claude streaming rows deduplicate by session/message identity using maximum
reported counters. Codex cumulative rows contribute deltas with cached input
kept separate. Decreased or malformed Codex counters exclude that session file;
conflicting deltas in overlapping copies fail explicitly. No missing value is
inferred from transcript text. Imported CLI history stays separate from measured
Bruhs turns because their overlap cannot be established from older receipts.

## Configure an installed ACP command

In **Workbench → Providers**, enter a literal executable and one argument per
line. Shell expansion is not performed. Supply model IDs and provider-native
mode IDs for Plan, default and Accept edits. Leave a mode blank to make that
access unavailable. Native modes describe the installed agent's behavior; they
are not an OS sandbox.

ACP v1 configuration options are preferred. Optional **Mode selector ID** and
**Model selector ID** default to `mode` and `model`; set them when an adapter
advertises different identifiers. Model selection runs first, then access
selection. The adapter must echo the chosen configuration before Bruhs sends a
prompt. Agents using the older `modes` interface remain supported.

For the installed OpenCode 1.19.5 ACP server, the accepted command is `opencode`
with literal arguments `--pure` and `acp`; its Plan value is `plan` and its
default editing value is `build`. `--pure` disables external plugins. Sign in
using the agent's own CLI when its provider requires authentication. Select
`acp/<instance>/default` in the composer to retain its default model.

The acceptance check negotiates ACP v1, creates an isolated local session,
selects Plan and reloads it without submitting a prompt or starting a provider
turn. Reproduce it with:

```sh
CARGO_INCREMENTAL=0 cargo test -p bruhs-core installed_opencode_negotiates_modes_and_reload_without_a_model_turn --lib -- --ignored
```

The option contract follows the [official ACP v1 configuration specification](https://agentclientprotocol.com/protocol/v1/session-config-options).
