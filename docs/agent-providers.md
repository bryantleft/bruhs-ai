# Agent runtimes and models

Choose a runtime in the composer model picker on web or desktop. Each runtime
uses its own installed CLI, login and provider configuration. Bruhs does not
copy API keys into browser storage or add them to prompts.

The runtime and the model vendor are independent. For example, a Claude model
selected through Cursor runs in Cursor. An xAI model selected through OpenCode
runs in OpenCode with that provider's API credentials.

## Selecting a model

The built-in choices include each external runtime's default model. They omit
the CLI's model flag, so the runtime's configured default stays in control.
Type an explicit selection into the same picker to choose another model:

| Selection | Runtime model argument |
| --- | --- |
| `cursor/default` | Cursor's configured default |
| `cursor/sonnet-4` | `sonnet-4` through Cursor |
| `antigravity/default` | Antigravity's configured default |
| `grok/default` | Grok's configured default |
| `opencode/default` | OpenCode's configured default |
| `opencode/xai/grok-code-fast-1` | `xai/grok-code-fast-1` through OpenCode |
| `opencode/local/my-model` | Your configured OpenCode provider and model |

Model names in the examples must be available in the installed runtime and
account. Use that runtime's model list for current names.

The Cursor group in the picker lists your account's models beneath
`Cursor default`. Bruhs reads them from `cursor-agent --list-models` each time
the picker opens. The server and desktop cache the answer for ten minutes, or
for 30 seconds after a failure, so a sign-in or update shows up quickly. A
failed refresh keeps the models listed before. When Cursor cannot list them,
the picker footer says why. Typical reasons are a CLI
older than the flag (run `cursor-agent update`) or a signed-out CLI (run
`cursor-agent login`). Typed `cursor/model` selections keep working either
way. On the web the list comes from `GET /api/agent/models/cursor`. Other
runtimes return an empty list. Custom selections
persist in threads, the new-thread default, and roles. Existing unprefixed
Claude and Codex selections keep their previous routing.
To assign a custom model to a role, choose it in the composer first. It then
appears as the current default in the role and project-override model menus.

## Setup

| Runtime | Executable | Sign-in or configuration |
| --- | --- | --- |
| Claude Code | `claude` | `claude auth login` |
| Codex | `codex` | `codex login` |
| Cursor | `cursor-agent` | `cursor-agent login` |
| Antigravity | `agy` | Launch `agy` and complete sign-in |
| Grok | `grok` or `~/.grok/bin/agent` | Use that CLI's `login` command |
| OpenCode | `opencode` | `opencode auth login`, then configure/select a model |

Grok never resolves a bare `agent` executable from PATH: Cursor and Grok both
use that name in some installations. Set `BRUHS_CURSOR_BIN`,
`BRUHS_ANTIGRAVITY_BIN`, `BRUHS_GROK_BIN` or `BRUHS_OPENCODE_BIN` to override
an executable's path.

For API access, gateways and local models, configure the provider in OpenCode,
then select `opencode/provider/model` in bruhs. The adapter preserves provider
configuration while adding its own per-turn access policy. Configuration
schemas differ between OpenCode releases; follow the documentation for your
installed version rather than copying credentials into bruhs settings.

## Capabilities

| Runtime | Default and Plan | Accept edits | Bruhs app tools |
| --- | --- | --- | --- |
| Claude Code | Existing approval and plan behavior | Existing behavior | Available |
| Codex | Read-only sandbox | Workspace-write sandbox | Available |
| Cursor | Deny writes, shell and MCP calls | Allow edits, deny shell and MCP | Unavailable |
| Antigravity | Rejected before execution | Rejected before execution | Unavailable |
| Grok | Restrict tools and deny writes/shell/MCP | Native edit mode | Unavailable |
| OpenCode | Allow read/search tools, deny other tools | Allow edits, deny shell and MCP | Full access only |

Antigravity currently requires an explicit Full access selection. Its `plan`
mode is an instruction prefix, not an enforced read-only boundary. The adapter
does not silently substitute that mode for bruhs's access policy.

Full access is explicit and uses the selected runtime's permissive mode.
Existing CLI deny rules can still prevent an operation. Cursor's per-turn
configuration lives in a private temporary directory; the user's settings
and login files are not rewritten.

Cursor, Grok and Antigravity retain their native tools and configured integrations,
but do not receive bruhs's private per-turn MCP tools. Cursor requires a
persistent approval for each new server configuration, which bruhs does not
write on the user's behalf. The picker and stream report this limit.
External runtimes are not enabled for unattended event
agents, automatic next-message predictions, or meeting summaries. Meeting
summaries continue to use Claude; second opinions use the existing Claude
and Codex reviewer adapters.

Update these additional CLIs using their own installers. Settings only manages
Claude Code and Codex updates. Streaming, tool events, session resumption,
process errors and cancellation use the common bruhs event interface.

## Integration references

- [Cursor headless CLI](https://cursor.com/docs/cli/headless)
- [Antigravity headless CLI](https://antigravity.google/docs/cli/headless/)
- [Antigravity execution modes](https://antigravity.google/docs/cli/modes/)
- [OpenCode CLI](https://opencode.ai/docs/cli/)
- [OpenCode providers](https://opencode.ai/docs/providers/)
