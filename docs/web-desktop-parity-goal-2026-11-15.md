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
| PARITY-06: thread card menu | Both menus show one list of rows ([`thread_menu`](../bruhs-models/src/logic/sidebar.rs)): Pin, Settle, Wake thread while snoozed, the snooze times In 1 hour, Later today, Tomorrow, Next week and Custom…, then Regenerate title and Discard draft. The [desktop menu](../bruhs-desktop/src/views/workspace/sidebar_ui.rs) and the [web menu](../bruhs-web/src/components/sidebar.rs) open under the card's "…" by one placement rule ([`menu_origin`](../bruhs-models/src/logic/menu.rs)), flip above it near the window's bottom and follow the card. The arrow keys, Home and End move through the rows (`menu_step`); Escape, Tab and every action close the menu and return focus to "…"; a click outside closes it without reaching what is under it, and so does focus moving elsewhere. Custom… opens Snooze until on both apps (desktop [`snooze_ui.rs`](../bruhs-desktop/src/views/workspace/snooze_ui.rs)), with the app's date picker and unit list. Verified 2026-11-16 by the desktop `desktop_thread_menu_*` tests, a 22-check headless Chrome run at 1280×800 and 1280×420, and real-window screenshots. One visual difference remains in the shared date and time picker: its trigger spans the field on desktop and fits its text on web, here and in the event dialog. | Both surfaces share one item set and one keyboard and dismissal model. | Done: same rows, keyboard and dismissal |
| PARITY-07: transcript turn actions | Web shows a turn's copy and metadata row on hover or focus within. Desktop always shows it, with Cite in composer, so the keyboard can reach it; GPUI could hide it with opacity and track focus within. | Agree on one visibility rule that keeps the actions reachable from the keyboard on both surfaces. | Open: differs |
| PARITY-08: notice placement | [Desktop notices](../bruhs-desktop/src/views/workspace/meetings_ui.rs) and the web [BannerStack](../bruhs-web/src/components/banner.rs) both float over the active view below its header row, so nothing moves when one appears or expires. Only the notices take pointer input, and a tall stack scrolls past 30% of the window. Desktop announces a dropped live transcript once; web shows no banner for it. | One placement and one announcement rule on both surfaces. | Open: placement matches; the transcript notice is desktop only |
| PARITY-09: sidebar controls | The [desktop sidebar](../bruhs-desktop/src/views/workspace/sidebar_ui.rs) and the [web sidebar](../bruhs-web/src/components/sidebar.rs) put Search, Choose project folder and New thread on one line, have no project filter, and search project names as well as titles (`thread_matches`, `visible_threads`). | One control layout and one search rule on both surfaces. | Done: same layout and search |
| PARITY-10: Threads tab | The [desktop top bar](../bruhs-desktop/src/views/workspace/navigation_ui.rs) and the [web navigation](../bruhs-web/src/components/workspace_navigation.rs) hold only the 28px sidebar toggle; a thread's card or ⌘K returns to the conversation. | The same top bar on both surfaces. | Done: same top bar |
| PARITY-11: dialog placement | [Desktop dialogs](../bruhs-desktop/src/views/workspace/modal_ui.rs) and the [web Dialog](../bruhs-web/src/ui/dialog.rs) are centered in the window with 32px around them. The command palette and the project, import and record pickers stay top-anchored (web `anchor_top`), so they do not move while their lists change. | One placement rule on both surfaces. | Done: same placement |
| PARITY-12: shared size scale | Both apps define the [shared size scale](ui-standards.md) once ([desktop theme](../bruhs-desktop/src/theme.rs), web [tailwind.css](../bruhs-web/tailwind.css) and [tailwind.config.js](../bruhs-web/tailwind.config.js)) and use it for shared buttons, inputs and menus, the sidebar, notices, the chat header, composer actions and page controls. Web controls rely on `bruhs-control` for 44px touch targets instead of hard-coded 44px classes. Bars and headers keep their heights, and the meeting dock keeps its 44px buttons. The composer chips are 24px with 12px icons beside their 12px labels (`icon-xs`, shadcn's `xs` Button), and its queue, attachment and saved prompt buttons are 24px with 16px icons. Desktop page constants that equal a scale value now use its name. | One size scale on both surfaces; new controls use its names. | Open: icons inside page controls. 29 web icons at 12 or 14px sit directly inside buttons (the diff, terminal, right and preview panels, updates, the settings close, the stepper, the project and model pickers, file attachments), and desktop has matching constants such as `SETTINGS_PAGE_CLOSE_ICON_PX`, `STEPPER_ICON_PX`, the diff and terminal panel icons and the dialog close icon. Each needs a call between `icon` (16px) and `icon-xs` (12px beside 12px text). |
| PARITY-13: composer options panel | The [web panel](../bruhs-web/src/components/composer/options.rs) has a header with its title, Composer options, and a 32px close button, on the floating tone (`bg-longan-700`). The [desktop panel](../bruhs-desktop/src/views/chat/composer_ui.rs) has no header or close button and uses `longan_900`. Found in the code on 2026-11-16; not yet compared in screenshots. | One panel: the same header, close button and tone. | Open |

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
