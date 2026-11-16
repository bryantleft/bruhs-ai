# Web and desktop parity goal

Recorded 2026-11-15. **Status: active.** Every capability available in either
bruhs-web or bruhs-desktop must be available in both. Bring missing features,
controls, agent tools, and recovery paths to the other surface while preserving
the working behavior already present. This is a standing delivery goal; full
parity is not yet verified.

Parent: [product completion goal](product-completion-goal-2026-11-13.md).

## Scope

The feature baseline is the union of both apps. Audit in both directions:
web capabilities missing from desktop and desktop capabilities missing from
web. Include detail routes and commands, settings, provider connections, agent
read/write tools, import/export, attachments, permissions, persistence,
offline behavior, cancellation, error recovery, and undo. Cover every pillar
and the shared surfaces listed in the parent goal.

Web remains the visual reference under the Bruhs design system; that does not
make it the limit of the feature set. Desktop-only capabilities must also reach
web. Match layout, hierarchy, typography, tokens, and primary actions at the
same viewport, with platform-appropriate keyboard and pointer interactions.

Keep browser-incompatible OS operations in the existing local server or native
companion and expose an equivalent authorized flow to the browser. This goal
uses the existing local-host architecture: web may depend on the running local
server and capture daemon. Show required components and their unavailable
states, including any active native-host requirement for workspace tools.
Document real browser, OS, provider, or device constraints and their proposed
equivalent. An unresolved constraint stays an open gap. A disabled control, unrelated-page
fallback, or local-only imitation of a provider action does not close it.

Preserve local-first storage, domain grants, opt-outs, credential isolation,
recording consent, and provider boundaries. In particular, read-only Google
Calendar stays read only. Parity must retain existing capabilities rather than
remove them from the more capable surface. Native mobile keeps its separately
documented scope.

## Initial gaps and verification work

This is a starting list from the current source and pillar records, not a
complete capability audit. Recheck each item before implementation. Existing
pillar records own their detailed acceptance cases and evidence.

| Case | Current evidence | Required outcome | Status |
| --- | --- | --- | --- |
| PARITY-01: inventory | The parent goal lists all pillars and shared surfaces, but no complete bidirectional capability matrix is recorded here. | Inventory the union of routes, commands, UI actions, settings, and agent tools. For each capability, record web and desktop behavior, the missing direction, owning pillar, acceptance case, and evidence. Distinguish missing behavior from behavior awaiting verification. | Open |
| PARITY-02: booking | [Web booking](../bruhs-web/src/components/booking_page.rs) offers scheduling slots and local booking. The desktop `AppView::Booking` route in [workspace.rs](../bruhs-desktop/src/views/workspace.rs) renders the regular calendar. | Desktop can open the saved scheduling link, inspect equivalent availability, book a slot, and restore the result after restart. Keep the existing local-booking boundary. | Open: missing on desktop |
| PARITY-03: workspace agent tools | The [local MCP connection](local-mcp-workspace.md) supports the native Unix app; the web connection is unimplemented. The [People record](people-completion-2026-11-14.md) also identifies missing web People read tools. | Bring existing desktop workspace tools to web through an authorized local bridge, with equivalent per-domain grants, opt-out handling, proposal review, revocation, and the same durability guarantees as native execution. Audit all tools; People is one known example. | Open: missing on web |
| PARITY-04: behavioral acceptance | [Calendar](calendar-completion-2026-11-14.md), [Today](today-completion-2026-11-14.md), [People](people-completion-2026-11-14.md), and [Inbox](inbox-completion-2026-11-13.md) retain open cross-surface acceptance work. | Exercise the same user flows and meaningful failures on both surfaces, including reload/restart, offline/reconnect, stale edits, permissions, and failed saves. | Open: verification incomplete |
| PARITY-05: visual and interaction acceptance | Same-size comparisons and real native-window checks remain open in the pillar records. | Compare both apps with identical synthetic records at matching viewports; verify navigation, focus, keyboard access, text scaling, reduced motion, and the shared design tokens. | Open: verification incomplete |

## Delivery and acceptance

1. Build the bidirectional capability matrix alongside the parent goal's pillar
   audits. Inspect real implementations; a registered route or control alone
   does not prove an outcome works.
2. Close known missing capabilities in reviewable slices, preserving the parent
   goal's Inbox-first priority and handling shared dependencies when needed.
   Reuse shared models and logic so subsequent fixes reach both surfaces.
3. Verify each slice on web and in a real desktop window. Use the same synthetic
   records at 1280×800 and 640×480, plus the parent's 390×844 touch-web checks.
   Record the revision, environment, steps, expected and actual outcomes, and
   evidence. Automated model or GPUI frame tests supplement actual user-flow
   checks; they do not replace native-window or live-provider acceptance.
4. Close each capability only when both surfaces pass its behavioral,
   persistence/privacy, accessibility, visual, and applicable provider/device
   checks. Record each check's applicability and reason in the capability
   matrix; an unavailable provider or device remains awaiting external
   verification. Run the repository gate before merging implementation changes.

Use **open**, **in progress**, **awaiting external verification**, or **verified**
per acceptance case and surface. Keep implementation and verification states
distinct: the matrix records **missing**, **partial**, or **implemented** for
each surface separately from its acceptance status. Link existing evidence
instead of duplicating or resetting pillar progress. Assess closure against an
explicit source revision and its complete inventory. Overall parity stays open
until every required capability at that revision has passing evidence on both
surfaces. Later changes reopen the affected cases.

## Keeping parity

New features, settings, agent tools, and behavior fixes must be assessed on both
surfaces. Deliver the equivalent behavior together where possible; otherwise
record the missing counterpart and its acceptance case before calling the
feature complete. Update the matrix and owning pillar record as gaps are found
or closed. A new one-sided feature expands the shared baseline.
