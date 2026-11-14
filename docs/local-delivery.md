# Local code delivery

The Workbench’s Delivery tab reviews GitHub PRs and GitLab MRs, reads Linear/Jira issues, and runs local project actions. Forge operations use the host’s installed `gh`/`glab` and `git` with their existing authentication. Tracker tokens stay in the OS keychain. Managed cloud execution is not involved. The integrated forge adapters support GitHub and GitLab; other forge CLIs remain available in the terminal.

## Review, then run

Choose a repository folder and load its open pull requests, an individual PR, its diff or line comments. The list shows stack relationships when a PR's base matches another in-repository PR's head branch.

Prepare an action to create a PR; commit selected files, push and create a PR; comment; submit a review with line comments; request reviewers; or merge. Read PR loads the head commit used by review and merge actions. Merging uses GitHub's rules and `--match-head-commit`; the integration never bypasses branch protections. Auto-merge waits for required checks.

**Preview action** freezes the repository, branch, head, commands, multiline input and timeout. **Run reviewed action** starts that proposal once. For ordinary actions, a changed checkout or reviewed PR head is rejected. Native-stack current-head authority has its separate owner review described below. Commit actions also fingerprint selected file content and the tracked diff. Arguments are passed directly to processes, and review text goes through stdin; text containing shell syntax is not interpreted by the integration.

The host saves completed-step counts, bounded output and errors. Closing the UI does not stop an action. Stop action cancels the active process group on Unix. After a host restart, running actions become Interrupted and never replay automatically. A failed or interrupted network command may already have changed GitHub; inspect the repository and PR before preparing another action.

## Whole-stack actions

Select a PR and prepare **Rebase whole stack locally** or **Merge whole branch stack**. These actions load every page of open PRs and derive the complete base/head chain containing that PR, including its ancestors and descendants. Forks are scoped by repository identity and are refused rather than joined by branch name. Cycles, multiple owners of one head, branching children, dirty checkouts and stack branches checked out in any worktree are refused. The preview binds every branch name, head SHA and base SHA, plus the root SHA.

Local rebase also freezes every local branch SHA and its proven replay boundary. It supports linear histories whose previous parent tip can be established from the frozen local or remote parent; it refuses guessed boundaries and merge commits in the replay range. Local branches must already exist, and the local root must descend from the reviewed remote root. Git hooks, automatic stashing, rerere and automatic ref updates are disabled. Repositories with external Git filters or merge drivers require terminal review instead of silently executing those tools.

Every layer is replayed in one private detached scratch worktree. After all replays succeed, the graph, source checkout and local leases are checked again. A single `git update-ref --stdin` transaction verifies the root and compare-and-swaps all local branches together. A conflict leaves every source branch ref unchanged. No remote push runs. The scratch worktree is removed after success, failure, cancellation or timeout; cleanup failure identifies its path. A host crash can leave that owned detached worktree for inspection, and the interrupted receipt is never replayed.

Branch-stack merge checks all layers' heads, bases, passing checks, approval decisions and mergeability before any server write. It merges bottom-up with `gh pr merge --match-head-commit`, confirms each merged state, and retargets the next child to the root. It rechecks CI and reviews after each retarget; newly required checks or reviews stop the action. Branch protection remains enforced by GitHub. Queued merges, changed roots, failed commands and unexpected state stop later layers. Receipts retain confirmed merge counts and explicit retarget output, so partial delivery can be inspected before preparing a new action. The integration checks base leases immediately before mutations; GitHub's merge API provides an atomic head lease but no corresponding atomic base-branch lease.

GitHub native stacks use a different asynchronous merge API. The [official gh-stack implementation](https://github.com/github/gh-stack) accepts a merge method/action but no caller-provided expected-head leases. Choose **Merge native GitHub stack** for a separate owner review. Its default-off acknowledgement explicitly authorizes server-current heads: a push after the fresh full-stack preflight may be merged. The normal execute API and MCP cannot grant this acknowledgement; they refuse native merge proposals. Membership or destination changes require a new preview. The host submits the official async API once through the top open member, saves its operation UUID and observed per-PR completion, and distinguishes pending, enqueued, merged, failed and unknown outcomes. Queue acceptance is not completion. Refresh the saved operation to observe progress after a disconnect/restart; an uncertain submission is never replayed. Cancellation stops local observation and does not cancel GitHub’s remote operation. Local isolated rebase remains available when the branch graph satisfies its usual safety checks.

## GitLab delivery

The **Forge** selector detects `github.com` and `gitlab.com` from the origin, or explicitly selects GitLab for a self-hosted host. Supported forges are GitHub and GitLab. GitLab needs the official `glab` executable on the delivery host: run `glab auth login` for that host, or provide the CLI’s documented `GITLAB_TOKEN`/`OAUTH_TOKEN` through the host environment. A missing CLI or failed authentication produces an actionable error. The app never reads browser credentials. Origin URLs containing embedded credentials are refused.

The GitLab adapter uses [documented `glab api` JSON input](https://docs.gitlab.com/cli/api/) with the frozen host and encoded project path. It supports MR discovery/details, raw diffs, discussion line comments, comments, approval/change-request review, reviewer selection by numeric GitLab user IDs, MR creation, selected-file commit/push/create, CI watches, issues and worktrees. Slash-prefixed note/description lines are refused because GitLab executes them as quick actions. Request-changes uses [the dedicated GraphQL mutation](https://docs.gitlab.com/api/graphql/reference/mutations/#mergerequestrequestchanges), so unrelated pending drafts are not published. GitLab GraphQL application errors are failures even with HTTP 200. A review can partially publish its earlier comments before a later decision fails; receipts retain the exact completed steps.

[GitLab merge](https://docs.gitlab.com/api/merge_requests/#merge-a-merge-request) and [approval](https://docs.gitlab.com/api/merge_request_approvals/#approve-merge-request) requests include the reviewed SHA. Squash is explicit. The Merge option requires a project configured for merge commits; Rebase is refused because GitLab chooses that strategy through project settings. Ordinary branch stacks use the same complete graph and local rebase safety checks as GitHub. Stack merging sends a SHA lease per MR, confirms each merged state, retargets the next MR to the root, and stops when refreshed checks, reviews, heads or roots change. GitLab does not expose an atomic base lease for retargeting, so the host rechecks the observed base immediately around each step.

Watches freeze forge and origin identity. An origin change stops observation until the owner reviews a new watch. Replacing a watch with a different origin creates a new watch ID, so existing task subscriptions cannot silently follow it. Legacy watches without frozen origin identity require a fresh review after upgrade. Task subscriptions remain scoped to the approved watch ID and canonical repository.

## Issues and project actions

Read an issue, choose **Start issue in a worktree**, and use a branch beginning with `bruhs/`. The preview includes the actual worktree creation command. Setup is included only when explicitly selected. A completed receipt offers the new workspace and the issue's source-linked context.

For Linear or Jira, open **Issue tracker**, select the provider and save the connection for this repository. The token is accepted by the private form and stored in the OS keychain; only nonsecret connection settings live in the private delivery directory. Changing the origin or account creates a separate credential binding. Leave the token blank to retain the credential for that exact account. Requests use a shared bounded client without redirects; issue links never choose the credential destination.

Linear uses `https://api.linear.app` and its [GraphQL API](https://linear.app/developers/graphql). Personal API keys are sent directly in `Authorization`; select OAuth for a bearer access token. Enter a key such as `ENG-123` or a `https://linear.app/.../issue/ENG-123/...` link. GraphQL errors reject partial data.

Jira Cloud uses the configured HTTPS site and the [REST v3 issue endpoint](https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/#api-rest-api-3-issue-issueidorkey-get). Save account email plus API token, or select OAuth for a bearer access token. For OAuth, the host performs [Atlassian site discovery](https://developer.atlassian.com/cloud/jira/platform/oauth-2-3lo-apps/) and calls the fixed `api.atlassian.com/ex/jira/{cloudid}` gateway only for the configured site. Enter `PROJECT-123` or a `/browse/PROJECT-123` link from that same site. Atlassian Document Format descriptions are converted into plain task context. These integrations only read issues. **Start tracker issue in a worktree** freezes the source-linked issue context and actual Git/setup commands for review; opening the completed workspace passes that context into the existing task flow.

Project commands live in `.bruhs/config.json`:

```json
{
  "setup": [{ "program": "npm", "args": ["ci"] }],
  "run": [{ "program": "npm", "args": ["run", "dev"] }],
  "teardown": []
}
```

Each phase runs commands sequentially in the selected workspace. Run setup, Run project and Run teardown each require their own preview and explicit execution. A long-running server remains Running until it exits, is stopped or reaches the execution timeout chosen for that action. Discovery does not execute repository scripts. Shell scripts can be used deliberately by naming a shell executable and literal arguments; the review then shows that shell invocation.

## Follow CI and reviews

Choose a poll interval to watch a PR. The server keeps polling while its UI is closed. Changes to check failures, completed checks, review/comments, head commits, merge conflicts and terminal PR state create durable notifications. Comments from the signed-in owner do not wake a watch. A closed or merged PR stops its watch. Failed reads remain visible and are retried on the chosen cadence without replacing the previous successful snapshot.

For an agent wake-up, configure a Custom event source and reviewed agent subscriptions in Automations, then choose **Load agent event sources** and select that source by name. PR events enter a persistent outbox. Publishing uses a stable event identity so a reconnect does not create a second automation event. The subscription's existing read-only and review rules still apply. Without a source, the watch saves notifications without starting inference.

The active watch list and unresolved review queue use the existing 50-record review-page budget. When unread events fill that page, the affected watch pauses visibly instead of losing notifications. Finish or dismiss pending work before adding more. The service retains up to the background registry's 200 finished actions and 200 acknowledged/delivered events. Pending actions and unread events are not evicted.

The private journal is `delivery/journal.json` under the Bruhs local data directory. `BRUHS_DELIVERY_DIR` selects an isolated directory for tests. A process ownership lock prevents two servers from executing against the same journal.

## Verification

`cargo test -p bruhs-core delivery::tests --lib` covers literal multiline input, preview without execution, stale checkout checks, selected-path confinement and tracked deletions, script discovery, frozen script execution, single-use actions, restart interruption and watch change detection. `cargo test -p bruhs-models delivery::tests --lib` covers check-state interpretation.

The offline fixture in `bruhs-web/tests/delivery_browser.mjs --serve` can be opened in the shared preview browser. It substitutes synthetic GitHub responses and exercises review forms, explicit execution, line comments, watches and issue intake without reading credentials or writing a real repository. Live forge mutation is intentionally separate from those acceptance tests. `delivery::gitlab::tests` uses real synthetic Git repositories and a fake glab to verify literal stdin, frozen origin, CI mapping, reviewer/merge contracts, watches and leased bottom-up stack delivery. `delivery::native_stack::tests` uses fake gh responses to verify separate owner authority, pending/queued/unknown results, UUID refresh without replay, membership refusal and partial counts. `secrets::trackers::tests` uses local HTTP fixtures for real Linear GraphQL/Jira REST request and authentication contracts, credential reflection redaction, project isolation and ADF parsing. No test sends a real PR, review, merge or tracker write.
