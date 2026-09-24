# Sidebar rows aligned with Herdr's (2026-09-24)

## Identity and scope

A user-requested polish, not a ledger task. The user compared Cockpit's Spaces and Agents lists with Herdr's sidebar and asked for three things:
- agent states on the Spaces;
- the branch under each Space that is not a worktree;
- Herdr's cleaner indenting and alignment.

**What Herdr does**, read from the user's default Herdr session (read-only `herdr api snapshot`, with the user's permission):
- Herdr's API reports a Git checkout only for Spaces it knows as worktrees, and even then often without a branch.
- A plain Space whose panes sit in a repository has no `worktree` object, yet Herdr's sidebar shows its branch. Herdr reads the branch from the pane's folder itself.
- Linked worktrees show no branch line.

## Change

- **Branch for plain Spaces.** `space_git::read` now also covers a Space without a checkout, using its first pane's folder (`foreground_cwd`, else `cwd`). Such a Space is reported only when Git finds a branch.
  - The client's poll key includes that folder, so a `cd` into another repository is read at once instead of on the next 15 s poll.
  - Clients still never name a path; the folder comes from Herdr's snapshot.
- **Rows.**
  - One status column, with the name beside it and the branch as a second line under the name (no icon, as in Herdr).
  - The upstream position (`↑2 ↓1`) stays after the branch.
  - Linked worktrees get no branch line.
- **Repository rows** show their own agent state instead of a grid icon, with the collapse chevron at the right as in Herdr. A collapsed repository shows the most urgent state among its hidden worktrees, so a blocked worktree stays visible.
- **Worktrees** hang from a trunk under the repository's name; their state column sits right after the connector.
- **Agents** use the same columns: the Space name in bold and the tab muted (`sidebar-tidy · 1`), with the agent on the second line.

## Proof (browser)

Fixture: `ui_polish_runtime.py`, root `/tmp/cpol-vdh8d5ee`, Herdr session `polish-vdh8d5ee`, gateway `127.0.0.1:35961`, playwright session `cside`, 1440×900 and 480×900.

The fixture contained:
- a repository Space on `main`;
- two worktrees opened with `herdr worktree open`;
- a plain folder;
- a plain Space in another repository on `develop`.

Agent states were set with `herdr pane report-agent`: working, blocked, idle, idle.

| Step | Observed |
| --- | --- |
| Before | Flat list, no branches: Herdr gave no Git data for the plain Spaces, and the worktrees had not been opened through Herdr (`SIDEBAR-before.png`). |
| After | The repository shows `◐` with `main` under it. The worktrees show `×` and `○` on a trunk. `notes` shows `·` with no branch; `other` shows `develop` (`SIDEBAR-after.png`). `GET …/space-git` returned `develop` for the plain Space. |
| Collapse the repository | Its state becomes `×`, taken from the blocked worktree (`SIDEBAR-collapsed.png`). |
| Select a worktree, 480×900 drawer | Same alignment, and the selection bar stays within the row (`SIDEBAR-narrow.png`). |

## Checks

- `cargo test --workspace --exclude cockpit-tauri`: 329 passed, including the new `a_plain_space_shows_the_branch_of_its_first_panes_folder`.
- `space_git.rs` was rustfmt-clean at HEAD and is formatted.
- `bunx tsc --noEmit` passes. `bun run test`: 247 passed, including the new collapsed-state and poll-key tests.

## Not covered

- Native (Tauri) window: the change is shared frontend code and shared core code behind the existing `cockpit_space_git_status` command.

## Cleanup

- Herdr, gateway and fixture server are gone (PIDs 844519, 849130, 844518).
- Port 35961 is released and no process references the root.
- The root and the browser session `cside` are removed.
