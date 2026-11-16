# Web and desktop capability matrix

Recorded 2026-11-16 for the [parity goal](web-desktop-parity-goal-2026-11-15.md)
(PARITY-01). The inventory reads the committed source at **59674f6d**. Rows
closed since then name the commit that closed them. Later changes reopen the
rows they touch.

## Method

Five read-only audits split the union of both apps: routes, commands and
shortcuts; settings and connections; agent tools, permissions and automations;
Inbox, Today, Calendar, Tasks and People; and chat, Knowledge, Meetings,
Finance, Health and the shared surfaces. Each traced controls to their handlers
and saved state in `bruhs-web/src` and `bruhs-desktop/src` rather than trusting
labels or registered routes. Nothing was run, so **implemented** means a
complete code path exists, not that it passed acceptance.

Implementation state per surface: **implemented**, **partial** (limited,
placeholder, or missing a step), **missing**, or **n/a** (cannot apply on that
surface). Acceptance per case and surface: **open**, **in progress**,
**awaiting external verification**, or **verified**, as the
[pillar records](product-completion-goal-2026-11-13.md) hold it. A pillar
record's case ID (such as TODAY-07) links its evidence.

## Gaps between the apps

Each row is a capability one app has and the other lacks or only partly has.
Close a row by bringing the behavior to both apps and recording its checks;
then move it to the matching list below.

| ID | Capability | Pillar | Web | Desktop | Missing on | Evidence (web; desktop) | Acceptance | Status |
|---|---|---|---|---|---|---|---|---|
| PM-01 | Booking page for a scheduling link | Calendar | implemented | implemented | — | `components/booking_page.rs`; `workspace/booking_ui.rs` | Desktop test `booking_page_books_a_local_event_like_web` | Closed in 6abbbbf0 (PARITY-02) |
| PM-02 | Workspace agent tools over the local MCP connection, with session enable/revoke, grants, proposals, review, decline and undo review | Workflows and automations | missing | implemented (Unix) | web | none; `workspace/mcp_bridge/` | PEOPLE-12 verified on desktop | In progress (PARITY-03) |
| PM-03 | Decline a workflow proposal; browse older workflows; a previewed workflow survives reload | Workflows and automations | partial | implemented | web | `components/automations_page.rs:17,89-125,228-262`; `automations_ui.rs:889-895`, `automations_ui/history_ui.rs:28-92` | open | Open; part of PARITY-03 review |
| PM-04 | Rename a thread | Chat and code | implemented | missing | desktop | `components/chat_header.rs:57-83`; `chat/composer_ui.rs` (title is text) | open | Open |
| PM-05 | `/model` opens the model picker | Chat and code | implemented | partial (opens Settings) | desktop | `mention_editor.rs:563-565`; `chat/hints_ui.rs:296-299` | open | Open |
| PM-06 | Codex sign-in banner and sign-in status in the model picker | Chat and code | implemented | missing | desktop | `components/auth_banner.rs:30,168`, `model_picker.rs:65-75`; `chat/auth.rs:67`, `model_picker_ui.rs:681` | open | Open |
| PM-07 | Palette: New terminal; Open project in editor | Chat and code | implemented | partial (other controls) | desktop | `command_bar.rs:326,350`; `terminal_panel.rs:510`, `keybindings_ui.rs:735` | open | Open |
| PM-08 | Palette: Open a local server… | Chat and code | partial (preview panel only) | implemented | web | `components/preview_panel.rs:129`; `palette_ui.rs:253,628-668` | open | Open |
| PM-09 | Open in system Terminal | Chat and code | missing | implemented | web | none; `palette_ui.rs:247,523` | open | Open: needs a local server route |
| PM-10 | Return to the last view after restart (Workbench, Whiteboards, detail views) | Shared shell | implemented | partial | desktop | `app.rs:113-121`; `workspace/persisted_enum.rs:60-79` | open | Open |
| PM-11 | Addressable links to views and records | Shared shell | implemented (hash routes) | partial (`bruhs://thread/` in ⌘K only) | desktop | `app.rs:12-83,142-176`; `palette_ui.rs:334-341` | open | Open: needs an OS URL handler |
| PM-12 | Knowledge library search | Knowledge and capture | implemented | missing | desktop | `knowledge_page.rs:140`; `knowledge_ui.rs` | open | Open |
| PM-13 | Markdown preview of a document | Knowledge and capture | implemented | missing | desktop | `knowledge_page.rs:597`; `knowledge_ui.rs:1086` | open | Open |
| PM-14 | Finance CSV import | Finance | implemented | missing | desktop | `life_page.rs:221,706`; `life_ui.rs` | open | Open |
| PM-15 | Delete a transaction | Finance | implemented | missing | desktop | `life_page.rs:609-627`; `life_ui.rs:549` (accounts only) | open | Open |
| PM-16 | Holdings list | Finance | implemented | partial (net worth only) | desktop | `life_page.rs:679`; `life_ui.rs:896` | open | Open |
| PM-17 | Sleep and recovery history | Health | implemented | partial (latest card) | desktop | `life_page.rs:823`; `life_ui.rs:921` | open | Open |
| PM-18 | Link or unlink a person on an imported conversation | Inbox | implemented | missing | desktop | `inbox_page.rs:316-324`; `inbox_ui.rs:566-835` | INBOX-08 open | Open |
| PM-19 | Auto-link an imported sender to the one matching profile | Inbox | missing | implemented | web | `inbox_page.rs:97`; `inbox_ui.rs:62` | People record, desktop test | Open |
| PM-20 | Confirm before deleting an imported conversation | Inbox | missing | implemented | web | `inbox_page.rs:303-312`; `modal_ui.rs:294-344` | INBOX-09 open | Open |
| PM-21 | Open a recent decision's source meeting from Today | Today | implemented | missing | desktop | `today_page.rs:475`; `today_ui.rs:815-830` | TODAY-09 open | Open |
| PM-22 | Open a follow-up's source person from Today | Today | partial (People page) | missing | both | `today_page.rs:456`; `today_ui.rs:792-810` | TODAY-09 open | Open |
| PM-23 | Recover from a day-plan save failure (Dismiss, Refresh, Retry) | Today | implemented | partial (text only) | desktop | `today_page.rs:318-335`; `today_ui.rs:706-714` | TODAY-07 verified on web | Open |
| PM-24 | "Calendar →" link from Today's schedule | Today | implemented | missing | desktop | `today_page.rs:380`; `today_ui.rs` | open | Open |
| PM-25 | Record a meeting from the event dialog | Calendar | implemented | missing | desktop | `event_create_dialog.rs:425-436`; `event_dialog_ui.rs` | open | Open |
| PM-26 | Google events readable offline and after restart | Calendar | implemented | partial (in memory) | desktop | IndexedDB `events`; `meetings_hub.rs:359` | CAL-11 partial | Open |
| PM-27 | Remove all imported Google events persists | Calendar | implemented | partial (memory only) | desktop | `google_calendar_settings.rs:124-145`; `meetings_hub.rs:1594-1597` | open | Open |
| PM-28 | Pick and remove task dependencies | Tasks and projects | implemented | partial (next candidate, clear all) | desktop | `task_detail.rs:225-455`; `tasks_ui.rs:651-678` | open | Open |
| PM-29 | Confirm before deleting a task | Tasks and projects | implemented | partial (deletes at once) | desktop | `task_detail.rs:477-514`; `tasks_ui.rs:788-795` | open | Open |
| PM-30 | Switch project with the sidebar hidden or in a compact window | Tasks and projects | implemented | partial | desktop | `tasks_page.rs:82,149-152`; `tasks_ui.rs:847-848` | open | Open |
| PM-31 | People recovery views: merge recovery data, unsubmitted relationship drafts, contact cache states | People | implemented | partial | desktop | `people_page/merge_status.rs`, `draft_status.rs`, `people_page.rs:174-190`; `people_view.rs:916,1180-1229` | PEOPLE-18 partial | Open: confirm which apply to desktop |
| PM-32 | Project title instructions pick a project from a list | Chat and code | implemented | partial (typed name) | desktop | `settings_page.rs:389-433`; `settings_page_ui.rs:947-969` | open | Open |
| PM-33 | Send a dictation draft from Workbench Verification into the composer | Workbench | implemented | missing | desktop | `components/workbench.rs:21-56`; `views/workbench.rs:10-31` | open | Open |
| PM-34 | App-event topic picker in the advanced source editor | Workflows and automations | implemented | partial (free text) | desktop | `reactive_automations/setup.rs:231`; `views/reactive_automations.rs:978` | open | Open |
| PM-35 | Local app context during a meeting | Meetings | missing | implemented | web | `meeting_live.rs:237`; `meetings_hub/local_focus.rs` | open | Open: OS-bound; needs an authorized equivalent or a recorded constraint |
| PM-36 | `thread.continueInWorkbench` shortcut | Workbench | partial (saved rule only) | missing | both | `command_bar.rs:523`; `keybindings_ui.rs` | open | Open: add to the shared shortcut list |

## Product gaps on both surfaces

These are missing from both apps, so they are not parity gaps; they belong to
the [product completion goal](product-completion-goal-2026-11-13.md) and must
reach both apps together: archive, read or snooze connected mail at the
provider; edit, disable or delete a scheduling link; local recurring events;
rename a project; undo task edits or deletion; delete a whole meeting; export
documents and clips; edit finance and health records; finance CSV export;
data retention controls; a text size setting; links that open across the two
apps.

## Platform differences

These differ by platform and are not gaps: app self-update with restart
(desktop only; a browser cannot install the app); the floating meeting dock
window (desktop) versus the in-page dock (web); Settings as a page (web) versus
a dialog (desktop); the embedded preview browser (desktop on macOS, opens the
system browser elsewhere); attachments opened in the system app (desktop)
versus downloaded (web); phone pairing (desktop; native mobile keeps its own
scope).

## Implemented on both

Each pillar's capabilities below are implemented on both surfaces at the
inventory revision. Acceptance stays with the pillar records and with PARITY-04
(behavior) and PARITY-05 (visual and interaction); a capability is verified only
when its record says so for both surfaces.

| Pillar | Capabilities | Main evidence (web; desktop) | Acceptance |
|---|---|---|---|
| Shared shell | Routes for every view and detail record listed in the goal; back and forward; the sidebar footer; the command palette's navigation, thread, task, scheduling, document, capture, meeting note and panel actions; every shared shortcut from `bruhs_models::logic::keybindings`; global search with the same result kinds; notices and banners; the agent update badge; compact and touch layouts | `app.rs`, `shell.rs`, `command_bar.rs`, `search_page.rs`; `workspace.rs`, `palette_ui.rs`, `keybindings_ui.rs` | PARITY-05 open |
| Chat and code | Start, resume, switch, pin, settle, snooze, wake, regenerate title, delete; runtime, model, access, mode and effort; streaming, stop, steer, queue, retry; attachments; rich replies and export; cite in composer; edit from here; second opinion, `/handoff`, `/role`, `/goal`, `/compact`, `/clear`, `/help`, `/settings`; approvals and secrets; thread import and export; diff, terminal, files, right panel, preview and find; projects, branches and worktrees; pull requests; saved prompts; compaction settings; `bruhs_threads` tools; the tool servers attached to each turn | `chat.rs`, `composer/`, `timeline_view.rs`, `sidebar.rs`; `chat/`, `threads_ui.rs`, `panels_ui.rs` | Chat pillar audit required |
| Inbox | Import and search conversations; filters and paging; read, snooze, important, archive; reply drafts and local log; ask agent; Gmail and Outlook connections; synced mail view, search, thread reading, attachments; compose, reply, reply all, review and send, drafts | `inbox_page.rs`, `connected_inbox.rs`, `mail_compose.rs`; `inbox_ui.rs`, `connected_inbox.rs`, `mail_compose/` | [Inbox record](inbox-completion-2026-11-13.md): INBOX-02/03 verified; INBOX-04, -06, -07, -16 open |
| Today | Now and next; focus queue; overdue and waiting; plan, review, apply and undo; rollover and time-zone invalidation; the DST-safe workday; briefing and suggestion | `today_page.rs`; `today_ui.rs` | [Today record](today-completion-2026-11-14.md): TODAY-01/02/03/05/07/08/11 verified on web; desktop restart and mounted checks open |
| Calendar | Month, week and day views; local event create, edit, delete; time grid drag, resize, create; day list; due-task markers and focus blocks; read-only Google events; smart scheduling; create a scheduling link; remote-change conflicts; Google connect and calendar choice; instant-based times; provider recurring instances | `calendar_page.rs`, `event_create_dialog.rs`, `smart_schedule_panel.rs`; `calendar_ui.rs`, `event_dialog_ui.rs`, `scheduling_ui.rs` | [Calendar record](calendar-completion-2026-11-14.md): CAL-01/02/03/06/14 verified on web; desktop runs open |
| Tasks and projects | Create and edit every field; estimate, recurrence, cycle and waiting on; recurring completion; list and board; board drag; paging; projects with cascade delete; global search; focus-block counts | `task_detail.rs`, `tasks_page.rs`, `task_board.rs`; `tasks_ui.rs`, `task_forms_ui.rs` | Tasks pillar audit required |
| People | Create, search, rename, aliases; observations; linked meetings and conversations; relationship items; follow-ups; merge; delete; session undo; tracking opt-out; voice enrollment; sync retry | `people_page.rs`, `people_page/`; `meetings_ui/people_view.rs` | [People record](people-completion-2026-11-14.md): PEOPLE-01/02/05/12 and others; desktop rename and opt-out by source review |
| Knowledge and capture | Capture kinds with the same icons; global capture; documents with history; source clips; links, backlinks and citations through the same search picker; attachments; ask agent; import conversation | `knowledge_page.rs`, `capture_dialog.rs`; `knowledge_ui.rs`, `capture_ui.rs` | Knowledge pillar audit required |
| Meetings | Record and stop; auto-record prompt; pause and resume; live transcript and meter; segment delete; speakers and enrollment; summaries and their model; observations; live suggestions; meeting notes; chat with a meeting; recovery; recording, transcription and speaker settings | `meeting_live.rs`, `meeting_detail.rs`, `meeting_notes.rs`; `meetings_hub.rs`, `meetings_ui.rs`, `meetings_ui/` | Meetings goal acceptance open |
| Finance and Health | Accounts, spending, budgets, income, holdings entry; workouts, sleep, recovery, habits; habit toggles; workouts list; privacy opt-ins; insights; ask agent; record detail | `life_page.rs`; `life_ui.rs` | Finance and Health audits required |
| Workflows and automations | Grants; recheck and grant for blocked domains; start, run, pause, resume; undo of unchanged records; receipts and audit; status notice; local suggestion; event sources, webhooks, enable, approvals, proposals, custom events, chat; updates, memory, follow-ups; notification settings; background service; weekly reports; reactive drafts | `automations_page.rs`, `reactive_automations/`; `automations_ui.rs`, `views/reactive_automations/` | Workflows audit required |
| Workbench | The Workbench page; continue a chat as a task; `bruhs_workbench` tools | `components/workbench.rs`; `views/workbench.rs` | Workbench audit required |
| Settings | The same eleven sections: general defaults, roles and project overrides, prompt suggestions, second opinions, meeting models; updates; beta; connected accounts and credentials; Google Calendar; recording; transcription; speakers; people; keybindings; about; notification and background service settings | `settings_page.rs` and its panels; `settings_page_ui.rs`, `settings_ui.rs`, `role_settings_ui.rs`, `credential_settings.rs`, `service_connections.rs` | PARITY-05 open |
