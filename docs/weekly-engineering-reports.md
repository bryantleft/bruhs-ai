# Weekly engineering reports

The weekly report preset saves an editable local report after a successful agent run. It uses the existing reactive runtime for on-demand work and repeating seven-day schedules. Web and GPUI clients share the host's saved reports and run state.

This is local working tree delivery for BNLE-36. It has not been published as a release.

## Use

1. Open Automations, then New report preset.
2. Choose an existing project folder and model. Select local Git commits, GitHub PRs or GitHub deployments. GitHub feeds use the host's `gh` login and the selected project's GitHub origin.
3. Save the preset. Enable Repeat every seven days for background generation while the local server runs. New schedules first run after seven days; a resumed schedule can catch up on a missed window. Choose **Fixed weekday and time** under Schedule to run at, for example, Monday 09:00 in your time zone instead; the reporting window then ends at that slot. A finished report appears under Automations → Updates.
4. Generate a report now. Leave the UTC window end blank for the latest seven days, or enter a historical end such as `2026-11-06T15:00:00Z`. All windows include their start and exclude their end.
5. Open the saved report to edit its title and Markdown, inspect sources, or reopen its project and agent conversation. The conversation receives the selected saved body as an unsent draft.

The report covers completed work, risks and suggested next steps, with source evidence. Missing incident/alert/reliability feeds remain explicit. A missing or partial source does not establish zero activity, perfect uptime or a trend.

## Persistence and permissions

The host records a frozen project/model/window and source snapshot before inference. A successful run and its saved report commit in one SQLite transaction. Failed, cancelled or incomplete provider output cannot produce a completed report artifact. Editing uses revision checks; a conflict retains the local draft.

An HTTP retry reuses its request identity. The runtime's durable scheduled-event identity prevents repeat deliveries from creating extra runs. An intentional new request can regenerate the same historical window with new settings while retaining the earlier report. Retrying a failed run keeps its original evidence and model. That older model's session cannot replace the current model's conversation session.

Report presets own their automation configuration. Generic automation controls cannot independently change their cadence, sources or approved context. Existing approved memory is retained when preset settings change. The schema upgrade preserves prototype report payloads and page identities while allowing explicit regeneration of a window.

Reports stay in the host's private reactive database. No email, chat post, repository write or publishing step is performed. Agent turns use the runtime's existing read-only permission mode; the host owns report persistence.

## Source limits

| Source | Coverage |
| --- | --- |
| Local Git | Commits reachable from the selected repository's HEAD during the reporting window. Commit identities remain visible; GitHub origins add commit links. |
| GitHub PRs | Opened, merged and closed lifecycle events in the window. GitHub's search ceiling and the local evidence budget can make a feed partial or unavailable. |
| GitHub deployments | Requests created in the window, with retained statuses before its end when accessible. Older requests' later transitions are outside this enumeration. |
| Incidents, alerts, reliability and trends | Unavailable until a supporting feed or comparable prior period is connected. |

GitHub source identity is pinned to the selected origin, including canonical SSH and HTTPS forms. An inherited `GH_REPO` cannot redirect it. GitHub.com is supported; enterprise and other hosting adapters are outside this preset.

Command reads use the existing eight-second PR lookup deadline and 32 KiB source response budget. Evidence receives half the runtime's existing 16,000-character prompt budget; discarded rows make truncation explicit. Cancellation stops source collection. No repository text is executed as a shell command.

Deployment requests and successful deployments are distinct in [GitHub's deployment API](https://docs.github.com/en/rest/deployments/deployments). Historical status retention is limited; see [GitHub's status API](https://docs.github.com/en/rest/deployments/statuses). The preset reports retained evidence rather than manufacturing a complete deployment or reliability history.

## Validation

Focused tests cover transactional report/completion saving, retry deduplication, cancellation, storage failure, revision conflicts, reopening, pagination, prototype migration, selected UTC windows, model/session binding, inert repository text and deployment status timing. A real scheduler restart regression verifies delivery at a saved deadline rather than another full interval later.

A live Claude run on a synthetic Git repository saved a report, accepted an edit and reopened that edit from SQLite. A separate live read-only GitHub replay checks the host CLI integration. Their receipts are retained with the delivery evidence.

The native test renders the preset editor, toggles scheduling and retains selected inputs. HTTP tests use the production native client against the server router. Playwright uses mocked API responses with isolated browser storage to check the production WASM at 640×480 and 390×844, including failed saves, duplicate submission, editable reports, reload, stale edits, source links, selected windows, unsent conversation context and viewport bounds.

Delivery receipts are in `bench/weekly-reports/results/2026-11-13/`. The live provider/GitHub replays, SQLite/HTTP tests and mocked-browser checks are separate checks, not one live run through both clients.

For development, build the web assets first:

```sh
cd bruhs-web
env -u NO_COLOR trunk build --dist ../target/weekly-reports-ui-dist
cd ..
```

With no companion server already running, start it from the repository root:

```sh
BRUHS_SERVER_PORT=3001 BRUHS_WEB_DIST=target/weekly-reports-ui-dist cargo run -p bruhs-server --locked
```

Open `http://127.0.0.1:3001/#/automations`, or use the native app's Automations page. A selected model needs its provider CLI installed and signed in. GitHub feeds require `git`, `gh`, a GitHub origin and an account with repository read access; `gh auth status --hostname github.com` checks that account without changing it.

Browser checks require Node, Playwright and Chrome. The runner accepts `BRUHS_CHROME` for another Chrome/Chromium executable and reuses a local Playwright installation or the npm cache. Install Playwright if neither is available.

Run focused checks from the repository root:

```sh
cargo test -p bruhs-core reactive:: --locked
cargo test -p bruhs-server weekly_report_preset_and_run_retry --locked
cargo test -p bruhs-desktop weekly_preset_controls --locked
node bruhs-web/tests/weekly_reports_browser.mjs
```

The browser check requires the fresh Trunk build above. Opt-in live checks are:

```sh
BRUHS_WEEKLY_REPORT_LIVE_OUTPUT=/tmp/bruhs-weekly-report.json cargo test -p bruhs-core live_weekly_report_delivery --locked -- --ignored --nocapture
BRUHS_REPORT_FEED_RECEIPT=/tmp/bruhs-weekly-feeds.json cargo test -p bruhs-core live_github_report_feeds --locked -- --ignored --nocapture
```

The provider replay defaults to `claude-haiku-4-5`; `BRUHS_WEEKLY_REPORT_LIVE_MODEL` selects another configured model. These checks use an installed provider or the host's GitHub account and are excluded from ordinary offline tests.
