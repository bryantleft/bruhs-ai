# Changelog

Notable user-facing changes to bruhs are documented here, newest first.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and versions follow [Semantic Versioning](https://semver.org/). Each version
is one signed release on [GitHub](https://github.com/bryantleft/bruhs-ai/releases),
and [bruhs.ai/changelog](https://bruhs.ai/changelog) shows the same entries.

## [Unreleased]

## [0.3.0] - 2026-11-26

### Added

- Desktop Usage shows provider limits, history, and reported costs with accessible charts and live totals.
- Desktop thread cards show project artwork and recorded models. Hover shows the full thread details, including handoffs, and exposes Snooze and Settle on eligible unpinned threads. Drag cards to pin, reorder, settle, or restore them.
- Windows x64 has a signed setup program with a Start menu shortcut and an uninstall entry. Ubuntu 24.04 x64 uses the desktop package with its runtime dependencies and application launcher.
- Linked drafts and prompt links help carry a conversation between the desktop, web, and mobile companion.
- Untrusted project folders can be reviewed before a provider runs inside them.
- Vault: Settings, Vault (formerly Credentials) now creates logins with one-time codes, keys and 1Password references. Grant a login to a bruh and it signs in on that site in its own browser without seeing the password. Keys still ask each time. See `docs/secure-credential-requests.md`.
- Apps: connect GitHub, Notion, Linear, Airtable, Todoist, Trello, ClickUp, HubSpot, Jira, Shopify, Stripe, Cloudflare, Slack and Figma with a token in Settings, Apps, and grant them and your Gmail or Outlook accounts to bruhs. A Claude bruh at Ask first or Full calls an app or sends email as you within its access, and never sees the token. Email, posts and payments ask first unless the bruh is at Full with Send as me on. Routines can start on new GitHub issues, pull requests and email. Each existing bruh starts one fresh session after this update. See `docs/service-connections.md`.
- Backup: make one encrypted file of all your Bruhs data in Settings, Backup, and restore it here or on another computer. Bruhs checks the file and shows what's inside before it replaces anything, moves paths to the new home, keeps your logins in the keychain, and keeps the replaced data aside until you delete it. See `docs/backup.md`.
- Imports: bring your ChatGPT, Claude.ai and Grok chats into Bruhs from their data exports (Grok is best effort), and move an OpenClaw or Hermes Agent persona in as a bruh. Imported memory waits for your approval on the bruh's Memory tab, and Move in asks another assistant what it knows about you. See `docs/imports.md`.

## [0.2.0] - 2026-11-18

### Added

- Windows x64 portable ZIP and Ubuntu 24.04 x64 desktop package. Both include the local server, web app and capture helper. System audio capture remains macOS-only.
- Agents can read public websites from a conversation.

- Always-on agents are now a crew of bruhs. Each has a face you can turn
  and recolour that shows when it is asleep, working, paused, off or needs
  you, a page with Chat, Memory, Routines and Settings, and an access level:
  Read only (the default), Ask first or Full. Ask first and Full pauses
  a run on a risky step and asks you, and an answer after the 15 minute wait
  still resumes it. Full keeps shell commands in the bruh's own folder.
  Can deliver and Send as me are separate switches, off by default. Stop and
  Resume work per bruh or for all, and ⌘. stops everything. ⌘⇧B opens the
  crew rail. An older background service shows a notice with Update service.
- Rooms: group chats with your bruhs. Mention a bruh with @ to hand it work,
  and a bruh's reply that mentions another hands the work on, up to 6 times
  in a row. Approvals show in the room. Agents in your threads can message a
  bruh with message_bruh.
- Computer: a bruh can have its own browser that keeps its logins. Watch it
  live in the bruh's Computer tab, take control, and hand it back; the bruh
  waits while you have it. Deleting a bruh now deletes its private folder
  and browser.
- The desktop app zooms like a browser: ⌘+ and ⌘− (or View → Zoom In and
  Zoom Out) step from 67% to 300%, ⌘0 returns to 100%, and the level is
  remembered. Pages reflow for the larger text at the web's widths.
- Workbench has a Worktrees tab listing every worktree bruhs made, chat
  ones included, with what removing each frees. Remove keeps the branch,
  and the chat's or task's next message checks it out again, on the web and
  in the desktop app. Removal also waits for any program working in the
  folder and for pull-request watches. A worktree whose branch was deleted
  can start again from another branch. Linux measures with allocated
  blocks by default, and a timed-out custom meter is stopped with
  everything it started.
- Finished Workbench tasks can remove their worktree to free disk. The
  task panel shows what removing it frees now and what stays shared with
  your checkout, measured by Pith's scanner on macOS. Removal keeps the
  branch and refuses uncommitted work, and the task's next message checks
  the branch out again. You can swap the meter for your own command or turn
  it off.
- New worktrees start with your checkout's build output on macOS (APFS) and
  on Linux with Btrfs or reflink XFS. Its ignored Cargo `target/debug` and,
  when the lockfiles match, `node_modules` are cloned copy-on-write, so an
  agent's first build or install starts warm at almost no disk cost. On this
  repository, 89 GB of build output arrives in about 7 seconds for under
  70 MiB. Projects can turn it off or add folders in `.bruhs/config.json`.
- The web's Agent workflows page can step back through older workflows, as
  the desktop app's can. Workflow history opens on the newest one; Previous
  workflow and Next workflow move through every workflow you previewed,
  declined, ran or undid, each reading like "Protect focus, Declined,
  workflow 2 of 6", and Review workflow shows the one on screen in full.
  The desktop app's history now names an undone workflow "Rolled back", as
  the web does.
- The web command palette can open a local server, as the desktop app's
  does. Choose Open a local server… to list what is listening on
  localhost, type to filter the list, and press Enter to open one in a new
  tab. Open in system Terminal now works on the web too: it opens Terminal at
  the thread's project, or at your home folder.
- The desktop command palette gains the web's New terminal and Open project
  in editor. New terminal opens another shell in the thread's project, in the
  chat's terminal drawer or on the Terminal page.
- You can give Continue in workbench a key on the web and in the desktop
  app: `thread.continueInWorkbench` is now listed, unbound, under Settings →
  Keybindings.
- You can rename a thread in the desktop app, as on the web: double-click
  its title in the header, or use the pen beside it. Enter saves and Escape
  keeps the old title.
- The desktop app shows the web's Codex sign-in banner when a thread uses a
  Codex model and Codex is signed out. Log in with Codex runs `codex login`
  and waits for it, and the model picker shows whether the Codex CLI is
  signed in.
- The desktop app reopens the page you left when it restarts, as the web
  does after a reload: Workbench, Whiteboards, a document, a source, a
  conversation (with its draft), a meeting or a meeting note. A record you
  deleted opens its list instead.
- The desktop app keeps each contact's unsaved relationship edits until you
  quit, as the web keeps them while its tab is open: opening another
  contact no longer loses them. People lists unsaved drafts with Open,
  Discard and a private copy. Save details changes only the fields you
  edited, and if one of them was also saved elsewhere it keeps your draft
  for review instead of overwriting.
- Workbench Verification's Use draft works in the desktop app on macOS, as
  on the web: the dictation goes into the active thread's composer and
  Chat opens.
- The desktop app opens `bruhs://` links from other apps: a thread link
  (⌘⇧C copies one) selects that thread, and `bruhs://open/` followed by a
  page's web route opens that page, such as `bruhs://open/inbox/<id>` for a
  conversation.
- The desktop app catches up with the web on Today, Calendar and Tasks.
  Today's decisions open their meeting, Now & next links to the calendar,
  and a day plan that could not be saved says so, with Dismiss, Refresh
  saved tasks and calendar, and Retry day-plan recovery. The event dialog
  can record a meeting. Google Calendar events stay readable offline and
  after a restart, and Remove all removes them for good. A task's
  dependencies are picked from a list and removed one at a time, deleting a
  task asks first, and a compact window switches projects from a Projects
  dialog.
- The desktop app does more of what the web does. Search the Knowledge
  library, which now lists the latest change first, and preview a document's
  Markdown. On Finance, import transactions from pasted CSV rows, delete a
  transaction and see your holdings; on Health, see how many sleep and
  recovery logs you have, in the web's cards. Link or unlink a person on an
  imported conversation. On People, view the originals kept after a contact
  merge recovery, and see when contacts are refreshing.
- The desktop app has the web's booking page for a scheduling link. The
  link in the scheduling panel opens it: pick one of the next free times
  inside the link's availability and buffers, add a name, and the booking is
  saved as a local event on your calendar.
- The web app now has the local MCP workspace connection the desktop app
  has. In Agent workflows, choose Enable for this session and Copy MCP config;
  a trusted MCP client running `bruhs-mcp` can then use the same tools against
  the open tab: read chat, tasks, calendar, captures, Knowledge, Inbox, People,
  Meetings and meeting notes, Finance and Health (each only with its Read grant
  and, for Life, its consent), and propose changes. Proposals and undo requests wait for your
  review in Agent workflows, and approved steps are saved like any other
  workflow step. The copied config names a private file; the session token
  never reaches the clipboard. Disabling, closing or reloading the tab, or
  stopping bruhs-server ends the session. One tab holds the connection at a
  time.
- The model picker now lists the models your Cursor account can run, under
  Cursor default, on the web and in the desktop app. Bruhs reads them from
  `cursor-agent --list-models`. When Cursor cannot list them, the picker says
  why and how to fix it, such as updating or signing in to the CLI. Typing
  `cursor/model` still works.
- Added a task board to Workbench Tasks. Agent tasks sit in four lanes,
  Building, Validating, In review and Ready, placed by what the work is doing:
  verification runs, and the branch and pull requests of each task's worktree,
  including checks, reviews and conflicts. Each card shows its branch, pull
  requests, comments, delegates, model and status, and cards that need you
  sort first. Archive files finished tasks away. The same board appears on the
  web, the desktop app and a paired phone.
- Added live meeting suggestions. While a meeting records, the meeting dock
  can offer private action items, open questions, conflicts with your tasks or
  calendar, and related records. Add one as a task, copy it, or dismiss it;
  what you keep tunes how selective it is. Choose Off, When I ask (the
  default), Automatic or Automatic with alerts, and the model, under General.
- Added messages between threads. An agent can list your threads, read one,
  and send one a message or start a new thread; the other thread's agent can
  answer back. A message waits in that thread's queue like one you typed, so
  you can still edit or remove it, and it shows who sent it. An agent can also
  wait for the other thread's reply. Plan turns can list and read threads.
- Added two settings under General: Messages between threads (Read and
  message, Read only, or Off) and Agent message limit, which stops a chain of
  agent messages after 2, 4, 6 or 10 and hands back to you.

- Added roles under General: a named model, effort, access mode and
  instructions that an agent can start a thread with, so one thread can
  orchestrate while others write or run on the models you chose. A project
  can override a role, and a role can narrow your access mode but never widen
  it.
- Added models by task under General: choose the model for meeting summaries
  and for each provider's second opinion.
- Added `/role`. `/role <name>` gives the thread you are in a role, `/role
  none` removes it, and `/role` alone shows it. A thread's role is named in
  its header and on its sidebar card. When a thread's role instructions
  change, its next turn starts a new session that carries the conversation,
  because a running session keeps the instructions it started with.

### Changed

- Agent workflows in the desktop app now reviews like the web's: an MCP
  client's proposal, its undo request and any workflow left waiting each get
  their own card above Local suggestions, with the full payload, the grants
  still missing and their own Start or Undo. A dry run you start is reviewed
  in Preview changes, and Run, Pause, Resume and Undo for the workflow on
  show sit in Workflow history. The MCP panel says what Copy MCP config
  copied, on both apps.
- The workflow notice above every page now has Pause for running background
  workflows on the web too.
- A dry run waiting in Preview changes can be declined there, on both apps,
  so one you decided against no longer holds up an MCP client's proposal.
- The desktop app runs on its UI framework, gpui, without patches. Long
  lines may now break at a `/` and no longer break after a `-`, so a line can
  wrap at a different word than on the web.
- Today in the desktop app now looks like the web's. Agent workflows and
  Life are quiet links in the header, the focus queue numbers its tasks,
  shows each priority in its color and says how many are ready, and
  Overdue, Waiting on, Follow ups and Decisions sit in one row, each with its
  colored dot. The local focus suggestion reads the same in both apps and
  counts only unfinished tasks.
- A conversation in the desktop Inbox now reads like the web's: its source
  and participants sit under the title, its actions are outlined buttons,
  messages sit in a centered column, and the reply box is three lines tall
  with the "⌘↵ log reply, drafts stay local" hint. Command Enter logs the
  reply, as on the web.
- The desktop Knowledge library lays out its records like the web's:
  documents sit two to a row, each with its document icon, and each saved
  source shows a bookmark.
- Finance in the desktop app now has the web's form in each card: Name,
  Balance and Add in Accounts; Merchant, Spend amount, Category and Add
  spend in Transactions; a Budget row and an Income row in Budgets &
  income; and Symbol, Value and Add in Holdings, in place of one Quick add
  form. Both apps check these entries the same way and use the same words,
  and Amounts redacted stays disabled until agent access is on.
- Agent workflows in the desktop app follows the web's order: Refresh under
  the introduction, then Updates, Weekly engineering reports, New automation
  and your automations, with Notifications and Background service side by
  side below them.
  desktop app, instead of only the People page.
- Removed the workflow overview and What's new sections from the landing
  page. The Changelog remains available in the navigation and command menu.
- Chat and coding conversations now share one thread list on web and desktop.
  New threads keep the current conversation’s optional project. Choose a
  project folder within a thread to continue the same conversation with files
  and tools.
- The web app now runs threads side by side, up to three at once. Sending in
  one thread no longer waits for another thread's turn to end, a queued prompt
  starts when its thread is free even if that thread is not open, and each
  thread asks for its own approvals.
- Stopping a turn on the web now pauses only that thread's queued prompts.
  Other threads keep running theirs, and a send in another thread no longer
  resumes the paused one.
- The desktop role editor now matches the web's: the role's fields sit under
  the Roles row, a project override is picked from a list, and roles come
  before the title instructions.
- The dot Bruh in Settings → About now places its eyes and mouth exactly
  where the logo has them, at every size.
- The sidebar fits its controls on one line, on the web and in the desktop
  app: Search, Choose project folder and New thread. The All threads project
  filter is gone; search now matches project names as well as titles, so
  typing a project's name lists its threads.
- Neither app shows a Threads tab above each page any more. To return to a
  conversation, choose its card in the sidebar or use ⌘K.
- Dialogs, such as New task, New event and Update providers, now open in the
  middle of the window on the web and in the desktop app. The command palette
  and the project, import and record pickers stay at the top, so they do not
  move while their lists change.
- Buttons, inputs, menus and icons now share one size scale on the web and in
  the desktop app, based on shadcn's compact sizes: 32px controls and menu
  rows, 28px header toggles, 24px actions on cards and 16px icons. Controls
  that were 40 or 44px tall with a mouse are 32px now, and phones keep 44px
  touch targets. Icons follow the text beside them: 16px in icon buttons and
  beside 13 or 14px labels, 12px beside 12px labels, as shadcn's small
  buttons do. The terminal's tab close and the project list's delete button
  are now 24px targets, and desktop dropdowns show a chevron icon.
- The desktop composer options panel now matches the web's. It has a header
  with its title and a close button, a column of access rows checked at the
  right, a Build and Plan switch, a three-column effort grid, and the web's
  wording. Saved prompts opens in the same kind of panel.
- The sidebar footer's view buttons sit in two rows, so each keeps its full
  32px instead of shrinking to fit one row. The desktop footer gains Agent
  workflows, with its unread count, like the web's.
- Controls that worked differently in the two apps now match. In the desktop
  app, the options panel shows the context ring and "N% used", Scroll to end
  is a labeled pill, a document connects records and cites sources through
  the same search picker as the web, copy buttons show a check for a moment,
  dropdown and custom model checks sit at the right, the update pill's
  loader turns (still with reduced motion), Open in browser shows a globe
  and "↗", the capture dialog's kinds show their icons, and date pickers fit
  their text. Date pickers on the web are 32px tall, like other inputs.
- A turn's copy and metadata row shows on hover or while it has keyboard
  focus in both apps, so hidden rows stay reachable with Tab. During a
  recording, a lost connection says "Live transcript disconnected.
  Reconnecting…" once in both apps and stays until the connection returns.
- The thread menu offers the same snooze times in both apps: In 1 hour,
  Later today, Tomorrow, Next week and Custom…. The desktop app gains
  Custom…, which snoozes a thread until a date and time you pick or for a
  number of minutes, hours or days. The web's Snooze until dialog now uses
  the app's own date picker and unit list instead of the browser's.
- `/model` in the desktop composer opens the model picker, as on the web,
  instead of Settings.
- Importing a conversation into the web Inbox links its sender to the People
  profile with that name or alias, as the desktop app does. A name that two
  profiles share stays unlinked. Deleting a conversation on the web now asks
  first, as on desktop.
- The desktop app now says which editor opened a project or file, and says
  No project to open when there is none, as the web does. Both apps say when
  the system terminal could not open.
- In the desktop app, Project title instructions picks the project from a
  list, and an app event source picks its meeting event from a list, as on
  the web, instead of asking you to type them.

### Fixed

- Buttons on the web show their intended colors: blue text and tinted
  fills (the selected Calendar view, Inbox filter and Agent workflows
  grants, Today's Review) and some card fills were being dropped by the
  stylesheet order.
- The desktop app's Calendar, People and Inbox look like the web's: the
  selected Calendar view is blue, due tasks are italic, Week and Day open
  near the current hour, and a narrow window's week fits all seven days;
  People's form and opened profile have the web's layout, labelled fields
  and a multi-line Notes field; the Inbox list uses the web's spacing and
  type. Italic text, page headers and People and Meetings text match the
  web's sizes.
- People controls are at least 24px with a mouse on both apps, and form
  fields 32px (some were 18 to 20px).
- The web no longer jumps the page 37px when the thread list finishes
  loading: Loading saved threads shows only once a load takes a while, and
  a failure still shows at once.
- The event dialog's Title, Notes and date fields have names for screen
  readers on the web, as on the desktop, and the date fields announce their
  date on both apps.
- On the web, leaving Agent workflows while a press in Workflow history
  (Run, Pause, Resume or Undo, or Previous and Next) was still settling
  could stop the app with an internal error, so pages no longer updated
  until a reload. Leaving the page now just leaves it; the press is still
  saved.
- On the web, the error banner for a refused or failed day plan Apply or
  Undo now goes with the error: when you dismiss it on Today, or when a later
  Apply or Undo works. It used to stay on every page, still saying, for
  example, that a focus block had changed after the Undo had gone through.
- The workflow notice counts one saved step as "1 step saved", in both
  apps.
- Keyboard focus keeps its place on both apps: opening an Inbox
  conversation lands on Back to Inbox and Back returns to its row, and the
  workflow notice's Pause hands focus to Open workflows. The web notice no
  longer drops keyboard focus each time a step is saved.
- On the desktop, primary buttons show a visible focus ring, and page titles
  are headings for screen readers.
- In the desktop app, a dialog opened from the keyboard puts focus on its
  first control, as the web does, instead of on the dialog itself: a delete
  confirmation on Cancel, a month's day list on its first record, Snooze
  until on its date, New bruh on its name, and a read-only Google event on
  Open in Google Calendar. Tasks' Projects starts on its close button.
- In the desktop app, a disabled text field is no longer a Tab stop, as on
  the web.
- In the desktop Inbox, Cancel and Escape close the import form and put
  focus back on Import conversation, as the web's dialog does. Import
  conversation in the command bar now closes the command bar instead of
  leaving it over the form.
- The desktop import form offers each provider as its own button, as the
  web does, instead of one button that cycled through them.
- Screen readers hear the desktop Inbox as they hear the web's: the filters
  and providers are pressed or not, a row reads its sender, date, subject,
  preview and source, the search field is a search field, and the list,
  the messages and the titles are named.
- When the desktop app's saved data cannot be decoded, the Inbox says
  "Conversations could not be loaded", as the web does, instead of "Your
  inbox is clear."
- In the desktop app, a failed save now reads like the web's: "Not saved",
  why, Retry save and Copy recovery data, under the page's top bar instead
  of under the window's close and minimize buttons.
- In the desktop app, the workflow notice (such as "An MCP proposal is
  awaiting your review") no longer runs under the window's close and
  minimize buttons. It sits under the page's top bar, as on the web, and is
  hidden on Agent workflows itself.
- The desktop app's keyboard works from the start: reopened on a page
  other than Chat, it used to ignore Tab and the shortcuts until a click.
- In the desktop app, Tab leaves a reply, note or other multi-line field,
  as it does on the web, instead of indenting the text; the composer still
  indents list items and code.
- A project's Delete button in the desktop Tasks list can be reached with
  Tab, as on the web.
- Desktop Agent workflows shows Local suggestions as the web does: its own
  New dry run, then each suggestion with its evidence, Dismiss and Preview.
- Screen readers hear the same names for Today's task rows and a
  document's fields in both apps.
- On a touch screen, a tap on a month calendar day opens that day's list,
  with full-size rows and New event, instead of hitting tiny dates and
  event pills.
- On the web's Agent workflows page, any workflow in Workflow history can be
  run, paused, resumed or undone, as in the desktop app: the workflow on show
  carries Run next step, Run in background, Pause, Resume and Undo
  transaction, whichever apply. Before, only the eight newest jobs and the
  three newest undoable workflows had them. A workflow you start now shows in
  the history.
- Health in the desktop app now has the web's form in each card: Activity,
  Minutes and Log in Workout; a Sleep row and a Recovery row in Sleep &
  recovery; and a title and Add in Habits, in place of one Quick add form.
  Both apps check these entries the same way and use the same words. A
  value a log would quietly change, such as 0 workout minutes or a 101%
  score, is refused, and the page notice says what to enter; the web used
  to do nothing or save the changed value.
- In a narrow desktop window the sidebar stays docked, as on the web,
  instead of hiding below 960px. The right panel docks from 768px; in a
  narrower window it opens over the chat, and a click beside it closes it.
  Tasks, a task's details, a document's details and booking times also
  change layout at the web's widths, as do a conversation, Agent workflows
  and Life's summary cards, and page headers move their buttons under the
  title when there is no room beside it. As on the web, a docked
  right panel leaves the chat at least 360px, is at most 70% of the
  window, and keeps the width you gave it when the window is resized.
- In a short desktop window the month calendar shows all six weeks, as on
  the web, instead of scrolling them; each day shows the records that fit.
- The desktop sidebar's Working dot pulses as it does on the web, and stays
  still when the system asks for reduced motion.
- The desktop finance CSV import box is several rows tall, as on the web,
  instead of one line.
- On touch screens, Today's Dismiss and Calendar → and the names on People
  are full-size tap targets.
- In the desktop app, a Knowledge document's body field is tall again, as on
  the web, instead of one line. The line under the tags says whether the
  document is saved, and Save waits until something changed.
- After a reload, the web task list keeps tasks in the order they were
  created, as the desktop app does, instead of reshuffling them.
- A month day on the calendar lists all-day events first, then the rest by
  start time, on the web and in the desktop app; a full day keeps the
  earliest.
- Desktop Search moves through results with Up and Down, opens one with
  Enter and clears with Escape, as on the web.
- Smaller desktop differences from the web are fixed: Inbox rows mark unread
  conversations with a dot, People has its subtitle, the Workbench page no
  longer shows the preview's address bar on macOS, and the composer's
  Attach button is a paperclip.
- The web app now saves a reply you stop as stopped. An MCP client reading
  that chat through the local MCP connection sees `stopped: true` for it, as
  it does from the desktop app, instead of `stopped: false`. Replies saved
  before this change still load and read as not stopped.
- In the desktop app, each account's and transaction's × on the Life page
  sits in its row again. It was drawn, and clicked, below the row.
- In the desktop app, Ask agent is on a Knowledge document's page, as on
  the web, and puts the document in the composer. It was in the library,
  where it did nothing.
- The web capture dialog's Contact tab shows its icon instead of a dot, and
  file sizes in the web email composer use the app's small label size
  instead of falling back to 16px.
- Fixed the thread menu in the desktop app opening at the bottom of the
  sidebar. It now opens under the "…" on its card, or above it near the bottom
  of the window. The arrow keys move through it, Escape closes it and returns
  you to the "…" button, and a click outside closes it without also clicking
  what is under it.
- The web thread menu now works the same way. It opens under the "…" on its
  card, or above it near the bottom of the window, and moves with the card.
  The arrow keys, Home and End move through it, and Escape, Tab or choosing
  an item returns you to the "…" button.
- Fixed desktop labels such as message times, model names and Edit from here
  appearing in the system font instead of Monaspace.
- The desktop app now shows one Export reply button under a reply, as the web
  does, instead of three export controls. It opens the media and widget
  choices and Save ZIP. Enter or Space on a button in a reply no longer runs
  its action twice.
- Fixed the desktop New meeting note button reading as a heading. It now sits
  beside the Meeting notes title, as on the web.
- Notices, such as a lost live transcript in the desktop app, now float over
  the page on the web and in the desktop app instead of pushing it up, so the
  composer and the page stay where they are.
- The live transcript notice now appears once, when a recording loses its
  connection, instead of after every reconnect attempt. It no longer appears
  when the server was never running.
- The web app now shows one Lost the connection to bruhs-server notice while
  the connection is down, and removes it when the connection returns, instead
  of adding a new notice after every reconnect attempt.
- The desktop Update providers and Restart to update dialogs show their
  question once, as the dialog's title, instead of twice.
- The sidebar toggle is now a 28px button on the web and in the desktop app,
  the size shadcn uses, instead of 44px. On the desktop, Enter or Space on it
  toggles the sidebar once instead of twice.
- Fixed the landing page's tour progress indicators cutting dots in half.
  Each tab now fills with complete dots, centered under its label.

## [0.1.1] - 2026-11-09

This release fixes the calendar around daylight saving time and tidies the
app's pages and controls.

### Added

- Added a Release notes link to Settings → Updates that opens this changelog
  at the version on offer.

### Fixed

- Fixed the calendar showing a day twice and today in the wrong column in the
  month a daylight saving change happens. Every calendar view now steps by
  calendar date.
- Fixed imported threads showing raw model ids such as
  `claude-haiku-4-5-2025…` in the composer; they now show the model's name.
- Fixed the composer's buttons wrapping onto a second line in narrow windows.
  Attach and Stash drop their labels instead and keep their icons.
- Fixed the Today icon showing as a dot on the web, and Today and Inbox
  showing no icon on desktop.
- Fixed Agent workflows suggesting a focus block when you have no tasks.
- Fixed thread titles showing the backticks from a first message such as
  "Implement `compute_invoice`", in the sidebar, the chat header and the
  import list.

### Changed

- Gave Inbox, Knowledge, People, Meetings and Terminal the same page header
  as Today and Life, and desktop Today the same header as the web.
- Made empty Today cards a quiet line instead of a dashed box.
- Made every Settings menu use the same dropdown, with inputs at the same
  height.
- Made Inbox, Life and meeting notes controls match the rest of the app.
- Rewrote Inbox and Meetings messages that described internals, and fixed
  singular counts such as "1 local record".

## [0.1.0] - 2026-11-09

The first public release: a local-first agent for your whole day, as a
signed Mac app and a web app that both run on your machine.

### Added

- Agent threads with Claude Code and Codex, on your own logins: a live
  timeline that folds finished turns, tool approvals, Steer to redirect
  Claude mid-reply, `/compact`, queued follow-ups, and a prompt stash (⌘S).
- Project threads in the current checkout or a new worktree, with Edit from
  here to rewind a thread and its files to an earlier message.
- A diff panel with a code, tests and docs filter, viewed marks and ⌘F find;
  a terminal drawer (⌘J); a preview panel (⌘⇧J) for local dev servers; and a
  Files tab.
- Thread import, provider session resume commands and Markdown export.
- Today (⌘⇧Y): your schedule, focus tasks, overdue work and meeting
  decisions, with Plan my day to preview focus blocks you can undo.
- Tasks and a calendar with Smart scheduling, local booking links, and
  read-only Google Calendar sync.
- Meetings: on-device recording of your microphone and the other side of the
  call, Whisper transcription with live previews, speaker labels and voice
  identity across meetings, summaries, editable notes, and a compact live
  meeting dock.
- People, Knowledge (Markdown notes and web clips), Inbox, and Life, a
  private finance ledger and health journal hidden from agents until you opt
  in.
- Capture anything (⌘⇧Space) and search across everything (⌘⇧F).
- Agent workflows that preview a multi-step change you can undo, and an MCP
  server that lets other agents propose changes for you to approve.
- Private credential requests: agents ask for a secret through a separate
  form, and the value goes to your keychain, never into the chat.
- Settings → Updates: update checks for the Claude Code and Codex CLIs with
  one-click updates, and for bruhs itself.
- The Mac app (macOS 13 or later, Apple silicon), signed and notarized. It
  starts its own server and meeting-capture daemon, and installs its updates
  when you choose Restart to update.
- Editable keyboard shortcuts for every action, and a command bar (⌘K).

[Unreleased]: https://github.com/bryantleft/bruhs-ai/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/bryantleft/bruhs-ai/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/bryantleft/bruhs-ai/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/bryantleft/bruhs-ai/releases/tag/v0.1.0
