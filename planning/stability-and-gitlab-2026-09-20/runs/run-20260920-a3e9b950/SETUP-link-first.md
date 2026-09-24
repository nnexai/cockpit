# One-click Space setup from a GitLab MR or Jira link (2026-09-24)

## Identity and scope

A user-requested redesign of the New Space dialog plus Jira work-item support. It is not a ledger task; see the 2026-09-24 entry in `DECISIONS.md`.

**What prompted it.** The user reported that choosing an existing repository left the list open and nothing appeared selected, and that the dialog looked bad. The user then chose:
- no separate confirm step;
- a repository field that behaves like the file picker;
- a link-first flow, since most setups start from a Jira ticket or a GitLab MR.

**Cause of the reported defect.** In the old picker, a click saved the choice but never closed the list: the clicked row kept focus inside the picker, which kept the list open. The search field only ever showed the typed query, never the chosen repository.

**Environment.**
- Herdr 0.9.1, jira-cli 1.7.0, glab (the user's keyring login).
- Google Chrome through `playwright-cli`, private in-memory session `csetup`.
- Linux x86_64, viewports 1440×900 and 480×900.

**Fixture.**
- Root `/tmp/csetup-y480ubtr`, Herdr session `csetup00309778` (PID 527786), gateway `127.0.0.1:34047` (PID 527817).
- Herdr and Cockpit state were private to the root.
- glab and jira read the user's existing configuration through `GLAB_CONFIG_DIR`, `JIRA_CONFIG_FILE` and a linked `.netrc`. Cockpit issued read-only CLI calls.
- Local repositories:
  - a fresh clone of the owned `nnex.ai/integration` fixture project;
  - a plain `source` repository.
- Providers configured: `gitlab` (glab) and `jira` (`https://nnexai.atlassian.net`).

**Remote fixtures**, both created on the user's request and recorded in `gitlab-fixtures.json`:
- draft MR `nnex.ai/integration!2`, whose title, branch and description reference `SCRUM-5`;
- Jira work item `SCRUM-5`, with one comment.

## Results (browser, real services)

| Step | Observed |
| --- | --- |
| Open dialog | Link field focused. Four rows: Link, Repository, Branch, Name, plus More. One primary button (`SETUP-link-open.png`). |
| Paste MR !2 link | "✓ MR !2 · Draft: SCRUM-5 Fix login timeout on slow networks".<br>"Also import SCRUM-5 · Cockpit test: fix login timeout on slow networks" (ticked).<br>Repository `integration`; branch and name `cockpit-fixture/SCRUM-5-login-timeout`.<br>Summary "New worktree … from 8345a4664b in …", "Imports nnex.ai/integration!2, SCRUM-5 into Context" (`SETUP-link-mr-resolved.png`).<br>About 9 s from paste to summary. |
| Enter | The dialog closed by itself after 17.4 s. A linked worktree Space on the MR branch is focused, with its terminal (`SETUP-link-mr-created.png`).<br>The companion holds the MR (`review/`) and SCRUM-5 (`issue/`) as Markdown. SCRUM-5 carries its facts, description and one comment. |
| Paste SCRUM-5 link, type `sou`, Enter | "✓ Issue SCRUM-5 · …". The picker overlays the form with the best match highlighted (`SETUP-link-jira-picker.png`).<br>Enter chose `source` and closed the list. The branch default became `cockpit/source/SCRUM-5`. |
| Enter | The dialog closed after 4.1 s. Worktree `cockpit/source/SCRUM-5` exists (`git worktree list`), and the companion holds SCRUM-5. |
| 480×900 | Stacked single-column rows; link field focused after the sidebar drawer closes; Escape closes the dialog (`SETUP-link-narrow.png`). |

**Direct gateway timings.**

| Request | Time |
| --- | --- |
| Defaults for MR !2, including the linked SCRUM-5 lookup | 2.5 s |
| Defaults for the SCRUM-5 link | 0.4 s |
| Plan for MR !2 | 6.3 s |
| Plan for MR !2 with SCRUM-5 linked | 8.4 s |

A single `glab api` MR read takes 1.1 s and `jira issue view --raw` takes 0.4 s. The MR's 17 s end to end is dominated by existing GitLab validation: plan, start preflight and import each read the MR in full. That cost is unchanged by this increment.

**Operation records** (`operations.json` equivalent): two completed setups and four unstarted plans.
- Two unstarted plans were created by the direct timing calls.
- Two were prepared while the form changed.
- Unstarted plans now expire after one hour and are deleted by the periodic prune; startup no longer converts them into resumable partial setups.

## Defects found and repaired during verification

- The new dialog's `space-setup` class collided with the global sidebar button class and rendered the dialog 25 px wide. It was renamed to `task-setup`.
- At 480 px the closing sidebar drawer restored its own focus after the dialog opened, so Escape did not reach the dialog. Focus is now re-applied on the next frame, and Escape is handled at the window level while the dialog is open.

## Checks

- `cargo test --workspace --exclude cockpit-tauri`: 322 passed.
- `cargo check -p cockpit-tauri` passes.
- `rustfmt` is applied to the changed files, which were clean at HEAD. No clippy warning falls on a changed line.
- `bun run typecheck` and `bun run build` pass. `bun run test`: 242 passed, including 14 dialog tests and the new protocol cases.

## Not covered

- The Linux-native (Tauri) dialog was not exercised. The change is shared frontend and core code.
- The existing-folder (Open) path was covered by unit tests only.
- A Space created on a plain folder carries no Git metadata from Herdr, so the dialog cannot preselect its repository; only Spaces Cockpit created as worktrees preselect.

## Cleanup

- The fixture stopped with Herdr exit 0 and gateway exit 0. Port 34047 was released, and no process referenced the root.
- The root was removed.
- The `csetup` browser session was closed. Other browser sessions (`pol`, `review`, `v2`) are not owned by this run and were left open.
- The remote MR and Jira item are retained as fixtures.

# Faster MR setup and pane-folder preselection (2026-09-24, second increment)

**Why.** Follow-up to the limitations above, taken on the user's instruction to work through the remaining items. An MR setup took 17.4 s after Enter because GitLab was asked for the whole MR three times (plan, check before start, import). A Space opened on a plain folder did not preselect its repository.

**Change.**
- Setup may reuse a provider result from the last two minutes; explicit Context import and refresh still always ask the provider.
- When metadata names the canonical URL (GitLab, Jira), the plan uses metadata and reads the full artifact in the background. The check before start waits for that read, so a failing full read still stops setup before anything is created.
- Cockpit keeps each pane's current folder from Herdr's snapshot (`foreground_cwd`, else `cwd`).
- With no Space checkout, the dialog preselects the innermost repository containing the focused pane's folder, re-read from a fresh snapshot each time it opens. It follows the Space only while the repository is still its own guess.

**Fixtures.** Same `setup_fixture.py` as above.
- Preselection run: `/tmp/csetup-z3z31idk`, Herdr `csetup3c0dfdbe` (PIDs 575867/575900), gateway `127.0.0.1:60071`, browser session `cspeed`.
- Timing run on the final build: `/tmp/csetup-xjxsi_un`, Herdr `csetupb3b9a893` (PIDs 665653/665737), gateway `127.0.0.1:41747`, browser session `cspeed2`.

| Step | Observed |
| --- | --- |
| Plain-folder Space; pane in `repos/integration/docs/deep`, then `repos/source`, then `repos/integration`; open the dialog after each | Repository `integration`, `source`, `integration`. The first attempt kept `source` every time; two causes were fixed: the form kept its first guess, and the live snapshot never saw the folder change. |
| Choose `source` in the dialog, close, reopen with the pane still in `integration` | `source` kept. |
| Paste MR !2 link (earlier build: reuse only) | Summary after 8.2 s; Create to closed 1.6 s (was 17.4 s). |
| Paste MR !2 link (final build) | Summary after **4.5 s** with MR !2, "Also import SCRUM-5" and repository `integration` (`SETUP-fast-mr-summary.png`). |
| Enter immediately | Closed after 4.1 s: the start waited for the background read. **9.2 s** from paste to a ready Space, against about 26 s before. Both MR and SCRUM-5 are in the companion (`SETUP-fast-mr-created.png`). |
| Paste SCRUM-5, repository `sou`, Enter; Enter again | Summary 2.3 s; ready 1.8 s later; worktree `cockpit/source/SCRUM-5`. |

**Checks.**
- `cargo test --workspace --exclude cockpit-tauri`: 325 passed. New tests cover setup reuse, prefetch reuse and pane folder parsing.
- `cargo check -p cockpit-tauri` passes. The protocol export `--check` is clean.
- `bun run test`: 245 passed, including 3 new preselection tests.
- rustfmt was applied to my hunks only; `sources.rs` and `cli.rs` were not rustfmt-clean at HEAD, and that unrelated formatting was left alone.

**Not covered.**
- A real full read that fails after metadata succeeds; covered by design (the check before start re-reads) but not reproduced against GitLab.
- Native (Tauri) dialog.

**Cleanup.** Both fixtures stopped with Herdr and gateway exit 0, their PIDs gone, ports 60071 and 41747 released, roots removed, browser sessions `cspeed` and `cspeed2` closed. The other browser sessions (`pol`, `review`, `v2`) are not owned by this run and were left open.
