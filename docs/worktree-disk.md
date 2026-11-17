# Worktree disk use

Agents work in their own Git worktrees. Most of a worktree's disk use is not
the checkout itself; it's the first build or install, which writes a fresh
`target/` or `node_modules`. On filesystems that support copy-on-write
clones, bruhs clones the project checkout's existing build folders into each
new worktree. The agent starts warm, and the clone costs almost nothing
until either side changes it. Each side's writes stay private.

| Platform | Clones | Otherwise |
| --- | --- | --- |
| macOS, APFS | `clonefile(2)` | |
| Linux, Btrfs or XFS with reflink | `FICLONE` | |
| Linux ext4, other volumes, Windows | | No build folders are reused |

The worktree and the project checkout must be on the same volume. Worktrees
live in the app's data folder (`~/Library/Application Support/dev.bruhs.bruhs-ai/worktrees`
on macOS), so a project on an external drive falls back. Nothing is ever
copied byte for byte as a fallback, so a failed clone never costs extra disk.

Git always checks the worktree out itself, so committed files are exactly
what `git worktree add` writes. Every new worktree gets this: New worktree in
chat on the web and in the desktop app, Workbench tasks started with New
worktree, and Start issue in a worktree, right after its reviewed
`git worktree add`.

## What is cloned

- **Cargo:** `target/debug` beside each tracked `Cargo.toml`, plus
  `.rustc_info.json`.
  - `incremental/` is left out: it is keyed by the checkout's path and
    cannot be reused. Other profiles stay behind.
  - Cargo reuses registry dependencies and rebuilds the workspace's own
    crates, whose build hashes include their path.
  - While a build holds `debug/.cargo-lock`, `target` is skipped, so a
    half-written artifact is never cloned.
- **Node:** `node_modules` beside each tracked `package.json`, only when that
  `package.json` and every lockfile beside it or at the repository root match
  byte for byte between the checkout and the new worktree. A branch with
  other dependencies starts without `node_modules`, as before.
- **Configured folders:** see below.

A folder is cloned only when all of these hold:
- Git ignores it in the new worktree. A folder Git would show as untracked
  is never cloned, so an agent cannot commit it by accident.
- It is a real directory in the checkout.
- The worktree does not have it yet.
- No symlinked folder leads to it on either side.

Measured on this repository (macOS, APFS, 2026-11-16, two rounds):

| | Time | Additional disk |
| --- | --- | --- |
| `git worktree add` | 0.6–0.7 s | 58 MiB |
| Build folders (89 GB `target/debug`, 2 × `node_modules`) | 6.5–7 s | 46–65 MiB |

In the cloned target, Cargo rebuilt about 20 workspace and path crates and
reused 1,879 registry builds. Most of the clone time goes to the metadata of
a very large `target/debug`; a typical project's target clones in well under
a second. For Start issue in a worktree, that time counts against the
action's reviewed time budget.

### What sharing costs later

Shared blocks stay alive as long as any clone uses them. When the checkout
later rebuilds or runs `cargo clean`, a worktree still holds the old
artifacts it was given, and those blocks are no longer shared with the
checkout. That is never more than the folder's size at the moment of
cloning, and every worktree cloned at that same moment shares it. Removing a
finished worktree releases it.

`du` and Finder report a clone's full logical size, so they overstate what a
worktree really adds. The task panel measures what removing it really frees.

## Removing a finished worktree

Open a task on the Workbench board. Its **Worktree** section says what
removing the worktree frees now, what stays shared with other folders (such
as the checkout's build output), and what local snapshots still hold. Choose
**Remove worktree…**, read what it deletes, then confirm.

- **What is deleted:** the folder, including ignored files such as build
  output and local `.env` files.
- **What stays:** the branch and every commit on it, and the task's record
  and transcript.
- **Coming back:** the task's next message checks the branch out again at
  the same path, with warm build folders as above. A task that shares the
  worktree (a fork) brings it back the same way.

Remove is refused when:
- the worktree has uncommitted changes or untracked files that are not
  ignored;
- no branch is checked out (detached HEAD);
- this task, or one sharing its worktree, is running, queued or still
  stopping;
- another checkout is inside it, which `git status` cannot see: a worktree an
  agent added under an ignored folder such as `.claude/`, or a nested clone;
- this task has an enabled pull-request wake, which reads its worktree;
- the worktree has submodules;
- the folder is outside bruhs's worktrees folder;
- Git refuses, for example for a locked worktree.

A folder that vanished outside bruhs can still be removed. That clears Git's
record of it and keeps the branch.

The removal is recorded on the task first. Then the folder is renamed aside,
Git drops its record, and the renamed folder is deleted in the background.
A large build folder never holds up the request, and a Git failure moves
the folder back and clears the record.
- **Interrupted removal:** a crash or full disk partway through still leaves
  the task able to come back. Restore drops Git's stale record before
  checking the branch out again.
- **Interrupted delete:** a server that stops mid-delete finishes the job at
  the next removal in that project.

The board keeps the pull requests it last read for a removed worktree,
without refreshing them, as it does for archived tasks.

Remove is the `remove_worktree` task command, so it works from web, desktop
and a paired phone (owners only). Agents cannot send it. Sizes are shown in
the web and desktop apps; the phone shows Remove without them.

## Measuring

The size comes from a replaceable meter, set in `worktree-meter.json` in
bruhs-ai's data directory (`~/Library/Application Support/dev.bruhs.bruhs-ai/`
on macOS). `BRUHS_WORKTREE_METER` names another file.

| `meter` | Measures with | Default on |
| --- | --- | --- |
| `pith` | Pith's scanner, built in: APFS private sizes and clone counts | macOS |
| `command` | Your program | |
| `off` | Nothing; no size is shown | Other platforms |

```json
{ "meter": "command", "command": "/Applications/Pith.app/Contents/MacOS/storage-scanner", "args": ["frees"] }
```

A `command` meter is run as `command args… <worktree folder>`, with a
two-minute limit.
- Its last output line must be JSON with `bytes`, and optionally `shared`,
  `snapshots` and `unreadable`, as Pith's `frees` prints.
- Use it to measure with a newer Pith than the built-in one, or to wrap
  another tool.
- On a filesystem without clones or snapshots, a script that sums allocated
  blocks once per inode is accurate.

A file that cannot be read or parsed shows its error in the task panel
instead of a size. The built-in meter is Pith's `storage-scanner` crate
(MIT), pinned to a commit of `bryantleft/bruhs`.

## Configuring it

`.bruhs/config.json` in the project checkout, beside the delivery scripts:

```json
{
  "worktree": {
    "copy_on_write": false,
    "build_dirs": ["vendor/bundle", ".turbo"]
  }
}
```

- `copy_on_write: false` reuses no build folders.
- `build_dirs` adds ignored folders, relative to the repository root, to the
  detected ones. These are cloned whole, without the lockfile check.
  - Paths that leave the repository or enter `.git` are ignored, and at most
    32 are read.
  - Do not list folders that store absolute paths, such as Python virtual
    environments: they break once cloned to another place.

A config file that bruhs cannot read or parse, or that is larger than 64 KiB,
turns cloning off, and the host log says why. A typo should never turn on
what you meant to turn off. Builds of bruhs older than this feature reject a
`config.json` that contains `worktree`, including its delivery scripts.

## Code

- `bruhs_core::git::add_worktree` creates every worktree and calls
  `seed_worktree_build_dirs`, which delivery also calls after its reviewed
  command.
- `bruhs_core::git::cow` picks the folders and checks the lock and the
  lockfiles.
- `bruhs_core::git::clone_fs` is the platform clone. It reports
  `ErrorKind::Unsupported` rather than copying, and removes a partial clone.
- `bruhs_models::delivery::WorktreeConfig` is the project setting.
- `bruhs_core::coordination` (`worktrees.rs`) inspects, removes and restores
  task worktrees; `GET /api/coordination/tasks/{id}/worktree?measure=true`
  serves the panel.
- `bruhs_core::worktree_meter` loads the meter and measures.

The host logs which folders each worktree shares at `info` level.

## Not done

Cloning committed files (as `git-twig` does) was built and then removed on
2026-11-16. It saved about 40 MiB per worktree on this repository, but making
the result identical to Git's own checkout meant re-implementing it:
- modes under `core.fileMode=false`;
- extended attributes such as quarantine;
- line-ending and filter attributes;
- submodule recursion;
- partial-clone fetches;
- a half-built worktree after a failure.

Repositories with large committed assets usually use Git LFS, whose
`post-checkout` hook would have turned it off anyway.
