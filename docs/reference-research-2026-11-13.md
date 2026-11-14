# Reference research for BNLE-38 and BNLE-39

Bruhs can borrow reusable context and durable execution patterns while retaining local ownership of project data and approvals. This research completes the two reference tasks; feature delivery remains covered by the implementation tickets.

Reviewed 2026-11-13. Recommendations below are design inferences for these tickets. They are not claims about features already shipped in Bruhs. This document and the reactive runtime are currently uncommitted working-tree content.

## Ramp Glass (BNLE-38)

Glass combines shared company context, reusable skills, collaborative sessions and model selection. Its launch announcement also describes project cost visibility and model routing. [Ramp announcement, 2026-10-13](https://ramp.com/blog/introducing-glass).

Ramp's onboarding starts with a real task and approved context, then source review. Personal connections and shared-space access have distinct permissions. Prompt instructions alone do not limit connected-account permissions. Repeating work uses an explicitly configured automation. [Ramp onboarding](https://support.ramp.com/get-started-with-glass/).

**Application to Bruhs:**

| Pattern | Recommendation and rationale |
| --- | --- |
| Reuse context | Keep source-linked project context and editable local reports. A generated report should reopen with its source window and agent thread. |
| Repeat useful work | Extend the reactive runtime in the current working tree for BNLE-36. Its persisted run identity can support retries. Save a report with a host-owned receipt so an agent response cannot be mistaken for persistence. |
| Enforce access | A database connection's agent-data permission belongs in the query boundary (BNLE-65). Pairing a phone must preserve existing data permissions (BNLE-68). |
| Show model choice | Retain explicit provider/model settings. Cost estimates should distinguish measured usage from unavailable pricing or usage data. |

The original reference posts remain below. X did not return readable source content during review, so their specific claims are unverified. The findings above use Ramp's primary sources instead.

- [Original Geoff Charles post](https://x.com/geoffintech/status/2042002590758572377?s=20)
- [Original Buchan post](https://x.com/buchan_sm/status/2044524727679566156?s=46)

## Vercel Open Agents (BNLE-39)

Open Agents separates its web interface, durable agent workflow and execution sandbox. The agent runs outside the sandbox; workflow and VM lifecycles can change independently. Its README describes persisted runs with reconnectable streams, cancellation and snapshot resume. Commit, push and PR creation are optional preferences. [Open Agents primary repository](https://github.com/vercel-labs/open-agents).

**Application to Bruhs:**

| Pattern | Recommendation and rationale |
| --- | --- |
| Separate control from execution | Keep job identity, approvals and completion receipts with the authoritative host. Treat a provider process or future cloud sandbox as replaceable execution. |
| Resume existing work | A reconnecting phone should subscribe to the desktop's existing turn and pending approval, preserving their identities (BNLE-68). |
| Persist results | Save weekly reports through the host's store. An agent instruction to write a file does not establish successful persistence (BNLE-36). |
| Make remote execution explicit | A future cloud worker needs scoped repository credentials and its own execution choice. LAN/Tailscale mobile access can control an awake desktop without a cloud worker. |

**Boundary:** These recommendations complete reference analysis. They do not add hosted execution, sandbox infrastructure or automatic repository publishing. A future cloud execution implementation requires its own acceptance criteria; the current mobile ticket still requires real desktop pairing and control.
