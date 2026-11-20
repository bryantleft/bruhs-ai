# Personal suite

Product decisions recorded November 19, 2026. The milestones below are open. Verify each milestone on web and desktop, including accessibility, persistence, and privacy.

## Ideas for recalling work

The goal is for bruhs to be your digital brain. Today already brings meeting decisions and follow-ups into view. These concepts explore how you might find the same context weeks later.

- **3D memory viewer.** Start at a meeting and explore the people who attended, the decisions recorded there, and the tasks linked to it. Select a connection to see why it exists and open its source. A keyboard-accessible list could show the same relationships.
- **Decision history.** From a project, follow a decision from the original meeting through later notes that revised it. Each entry would open the passage that explains what changed.
- **Catch me up.** Before meeting someone again, pull up the last conversation, outstanding promises and related notes. Each reminder would link back to the record it came from.

## Where each workflow belongs

| User intent | Owner and experience |
| --- | --- |
| Organize my day, work, relationships, and knowledge | Bruhs owns Today, Inbox, Calendar, Tasks, Meetings, People, Knowledge, capture, and the agent workbench. Today is the personal entry point, including quick capture on a phone. |
| Understand my money | Bruhs Finance owns accounts, transactions, spending, income, budgets, holdings, and savings goals. Ditz owns trip and household budgets, shared expenses, and settlement. |
| Manage my health | Bruhs Health owns workouts, sleep, recovery, habits, nutrition, mental wellbeing, symptoms, medications, appointments, screenings, and health records. Ditz owns meal planning, recipes, groceries, and cooking coordination. A person can choose to share a dietary constraint for a specific plan. |
| Keep my paperwork and obligations in order | Bruhs Life Admin owns tax years, IDs, insurance, leases, registrations, deadlines, and document renewals. Knowledge can hold the underlying files. A lab result belongs in Health, a claim dispute in Life Admin, and a medical payment in Finance. Link related records. |
| Decide what to change next week | Personal goals, milestones, private reflection, and Weekly Review belong in Today. Tasks and Calendar supply the initial evidence. Finance and Health participate by choice. |
| Plan a trip, find a place, shop, or coordinate a household | Ditz owns discovery, saved places, maps, itineraries, reservations, shopping comparisons, purchase follow-up and warranties, meals, shared lists, chores, votes, and receipt splits. This includes solo shopping and travel. Bruhs surfaces selected commitments and links to the original plan. [Ditz product brief](https://github.com/bryantleft/dump/blob/main/ditz.md) |

## Career workspace (deferred)

Career belongs in bruhs-ai as an optional workspace for a job search. The long-term plan is to bring the existing TypeScript pipeline and CLI into this repository as a dedicated module. Implementation and repository migration are deferred, with no scheduled milestone. [Existing Careers repository](https://github.com/bryantleft/careers)

The Career module would own job discovery and scoring, résumé versions, application review, submission status, and receipts. People would hold recruiter contacts, Calendar would hold interviews, and Notes and Today would hold interview notes and follow-ups. Completing a follow-up in Today would leave the application’s submission status under Career’s control.

The proposed starting point is an opportunity page. Returning to a role before an interview, you could open the posting, check the exact résumé you sent, see who you’re meeting, and pick up the follow-up you promised last time.

## Build milestones

Suggested slices can proceed independently except where a dependency is stated. The existing [web Life page](bruhs-web/src/components/life_page.rs) and [native Life view](bruhs-desktop/src/views/workspace/life_ui.rs) provide the manual ledger and health journal foundation.

| Workstream | First user-visible slice | Acceptance example |
| --- | --- | --- |
| Finance overview | Give Finance a direct entry, period selector, spending total, category breakdown, and transaction detail. Keep manual entry and CSV import accessible. Use Origin’s published spending designs as the visual reference. | With dated sample purchases in two periods, switch months, identify the largest category, and open its exact purchases in one click. Compare populated web and native renders at the same viewport; also inspect empty and narrow layouts. [Origin spending designs](https://useorigin.com/products/spending) |
| Life Admin | Start with tax season and document renewals. Group supporting files, missing items, deadlines, and completion evidence around an obligation. Surface the next action in Today. | Create a tax-year checklist, attach a received form, see what is missing, and save a filing confirmation. Separately, renew a registration and verify that Today shows the new due date after reopening. |
| Health | Give Health a direct entry and a dated history. Extend the existing journal first with appointments and linked health records, then deliver the other health domains listed above. | From a scheduled visit, open the appointment, attach a visit summary, and find it later by date. Existing journal entries survive the change. Agent access remains opt-in independently of connecting Ditz or enabling Weekly Review. |
| Today and Weekly Review | Make Finance, Health, Life Admin, and quick capture reachable from the personal home screen. Add a weekly review of goals, planned work, completed work, and outstanding commitments. | From a week containing a missed goal, inspect the underlying task and calendar block, record a private reflection, and choose a next-week action. Review works with Finance and Health access turned off. |
| Ditz connection | Once Ditz exposes stable plan and assignment identifiers plus authorized update and completion operations, add an optional connection in Settings. Let users choose households or trips, display selected events in Calendar and their assignments in Today, and open a plan or saved place in Ditz. | Reschedule a dinner in Ditz and see the existing event move. Complete “bring tortillas” in Today and see it completed once in Ditz, including after a retry. Cancellation removes the active commitment. Disconnecting revokes access and clears the connected views. |

## Cross-product examples

- **Shared dinner.** A $120 card charge belongs in Bruhs Finance. A $30 personal share and $90 owed back come from the Ditz split. A later connection can annotate the matched charge and distinguish cash outflow from personal cost. Counting the split as another purchase would inflate spending.
- **Passport and trip.** With permission, Bruhs can compare a connected departure date with a passport expiry held in Life Admin. The reminder opens the renewal in Bruhs and the itinerary in Ditz. The passport document stays in Bruhs.
- **Privacy.** Preserve the separate Finance exact-amount gate and Health agent opt-in. A Ditz connection grants access only to the selected Ditz information and actions. Sharing a Bruhs record or constraint requires a separate user choice.

## Customize pillars and features

Recommended design, pending implementation. Obsidian lets people enable built-in features individually and customize quick access. Apple Health lets people pin categories to Summary and inspect data access separately. Bruhs should adopt both patterns so someone can simplify Today, stop an optional feature, or manage sensitive access independently. [Obsidian settings](https://obsidian.md/help/settings) · [Apple Health Summary](https://support.apple.com/guide/iphone/view-your-health-data-iphe3d379c32/26/ios/26)

- **Settings → Features** lists available pillars in plain language, with one workspace-wide enable switch per optional pillar. Expand a pillar to configure independently useful features such as Weekly Review or medication reminders. Keep Settings and the feature directory reachable at all times. A disabled parent suspends its children and remembers their individual choices.
- **Customize Today** lets someone pin, remove, or reorder cards. Sidebar shortcuts can be customized separately. Removing a Finance card changes that surface only. Finance stays available in search and its scheduled work continues.
- **Turning a pillar off** retains its records and stops its commands, search results, Today cards, reminders, background jobs, and agent reads and writes. Before applying a change that interrupts an active reminder or workflow, identify what will pause. A saved link offers to enable the pillar without exposing record details. Re-enabling restores the records and preferences. Missed jobs and external writes require review before replay.
- **Permissions and connections** keep their own controls. Enabling Health grants no agent access. Enabling Finance grants no access to exact amounts. Enabling travel features never connects Ditz automatically. Disconnect and delete remain explicit actions.
- **First build slice:** use Finance as the first end-to-end toggle and give its Today card a separate pin control. Define effective availability once in the shared model and enforce it at execution and read boundaries as well as navigation. Existing workspaces retain current availability on upgrade. Newly introduced optional pillars can be discovered in Settings and enabled when wanted.
- **Acceptance:** unpin Finance and find a purchase through search. Disable Finance and verify that search, deep links, scheduled actions, and agent requests respect the switch. Restart, re-enable, and recover the same records, pin choice, and privacy settings. Repeat on web and native desktop, including keyboard and screen-reader operation.
