# FLOW-01 — gateway restart during a pending paste, Herdr response deadlines, repository discovery (2026-09-24)

## Identity and scope

A browser FLOW-01 increment. Main owned it and no other writer was active. It covers three things:

- **Criterion 7, gateway restart while a paste receipt is `pending`.** This is the `recover_pending` path left open in `FLOW-01-paste-reconciliation.md`.
- **Two workflow defects found while building the fixture,** both repaired here:
  - Herdr mutations shared a 2 s response deadline, so ordinary slow work was reported as an unknown outcome (OBS-045).
  - Repository discovery hid most of a real project tree from the setup picker (OBS-046).

FLOW-01 stays open.

- **Baseline:** source `34ed50a` (HEAD before this increment). Gateway `target/debug/cockpit` SHA-256 `078398d902453dc4261ae0a6d9ac5deeeaa7712bc8428faacf62d5f0d6018017`. Bundle `dist/assets/index-DI0XfbwR.js` SHA-256 `0e38bb7854b0a95b3f974b6cad98ed2345ec133aa7c44de989b73062b5b4affd`; the frontend is unchanged in this increment.
- **Repaired gateways:**
  - With the deadline repair only: `8284d9dff3344f422ae2a3bb67b55e50307166480fe63031ae11c3c9cba180c1`.
  - With both repairs: `ba69fa54e1cf6edb67d6ba1533ccb3dac70d627760b41e823aa0f0c27168f1a6`.
- **Environment:** Herdr 0.9.1, Pi 0.84.2, Google Chrome 153 through `playwright-cli` (private in-memory profile, session `flowpaste`), Linux x86_64, 1568×1009 viewport.
- **Fixture A:** root `/tmp/cflow-paste-lsz9_z2q`, session `flowpaste6b691bfe`, Herdr PID 330693, proxy PID 330724, gateway `127.0.0.1:32925`. The gateway was restarted on the same port: 330728 → 336162 → 341977 → 343005.
- **Fixture B:** root `/tmp/cflow-paste-7wq8t2y2`, Herdr PID 343629, proxy PID 343661, gateway `127.0.0.1:59825`. Gateway PIDs were 343662 (baseline) → 351033 (deadline repair) → 363393 (both repairs).
- **Setup common to both fixtures:**
  - Each root had a private HOME/XDG/Herdr config and socket, and a copied `herdr-file-viewer` registered only in that root.
  - The gateway reached Herdr's API only through a run-owned Unix-socket proxy.
  - Fixture B also had a generated 120,000-file repository `hugerepo`.
  - The user's Herdr session, profile, installed app and `.audit/` files were not touched.
- **Harness correction:** the gateway derives the terminal-protocol socket `<stem>-client.sock` from the API socket path. A `herdr-proxy-client.sock` symlink to Herdr's real client socket made terminals stream normally through the proxy. The "Terminal WebSocket failed" state recorded in `FLOW-01-paste-reconciliation.md` had exactly this cause, not the proxy's line bound; that file is corrected.

Raw receipts, proxy events, restart records, timings and discovery counts are in `FLOW-01-restart-and-deadlines.json`.

## Gateway killed while a paste is pending (fixture A, baseline gateway)

1. Task Space `flow-restart`, Context right, local snapshot import, and saved line comment `FLOWPASTE-R` on `README.md` line 3. Pi `flow-pi-r` started in `w2:p2`: `agent: "pi"`, idle, interactive ready. Pi's input held no `FLOWPASTE-R`.
2. The proxy's one-shot `HOLD` flag withheld the next `pane.send_text` (`cockpit-851`, 228 bytes, `w2:p2`) and never forwarded it. A watcher saw the hold 5 ms after dispatch and requested a restart. The fixture sent **SIGKILL** to the gateway process group 195 ms later (return code −9). This is well inside the 2 s `pane.send_text` deadline. Receipt `b87cc979-7ff6-420b-8232-a83d9dcb931f` was created at `…291785` ms, before the kill.
3. A new gateway started on the same port. On the first batch read, receipt recovery took the target/batch lease and moved the receipt to `outcome_unknown`: "Cockpit restarted while delivery was pending; inspect the terminal before retrying." This is the only path that writes that message. Nothing was re-sent: the proxy saw no further `pane.send_text`, and Pi's input still held no `FLOWPASTE-R`.
4. After the reconnect, the Context pane returned to its file list. Reopening README comments showed "Paste outcome unknown: Cockpit restarted while delivery was pending…". **Paste to agent** was disabled, the duplicate-risk checkbox was shown, and **Mark pasted · b87cc979** stayed disabled until the box was ticked (`FLOW-01-paste-restart-unknown.png`).
5. After inspecting the empty input, the acknowledged retry produced receipt `fcc6eda5-3d66-47dd-ab83-70e6fc610e62`, `accepted`: "Herdr accepted one raw bracketed-paste write; no Enter was sent." The draft was archived ("No comments yet."), and Pi's input held exactly one `FLOWPASTE-R` (`FLOW-01-paste-restart-retried.png`).

## OBS-045: Herdr mutations reported an unknown outcome for ordinary slow work

Every Herdr socket request, mutations included, shared `FINITE_RESPONSE_TIMEOUT = 2 s`, and the terminal handshake had its own 2 s bound. An expired deadline on a mutation becomes `request_outcome_unknown`, which needs manual recovery.

Direct timings against the fixture B Herdr socket:

| Request | Time |
| --- | --- |
| `pane.split` | 6 ms |
| `tab.create` | 6 ms |
| `workspace.create` | 6 ms |
| `worktree.create`, 40,000-file repository | 1.38 s / 1.22 s |
| `worktree.create`, 120,000-file repository | 3.66 s |

Herdr returns from pane and Space creation before the shell starts, so a slow shell does not delay them. Git worktree work, however, scales with checkout size.

- **Baseline (gateway 343662):** **Set up a task Space** on `hugerepo`, branch `huge-task`, stopped at "herdr requested" after about 5 s with the alert "Herdr mutation response deadline expired; outcome is unknown" (`FLOW-01-setup-large-repo-deadline-red.png`). Herdr had in fact created the worktree and Space. **Recover existing checkout** reached "Workspace setup is partial…" (`FLOW-01-setup-large-repo-recovery.png`), and **Resume failed step** reached "Space and terminal are ready". The durable recovery was correct, but a routine setup cost the user a false error and two recovery clicks.
- **Repair:** `cli/transport.rs::response_deadline(method)` sets the response deadline per request type:

  | Requests | Deadline |
  | --- | --- |
  | `worktree.create/open/remove` (git work, sized to the existing 30 s `operation_timeout_ms` default) | 30 s |
  | `workspace.create/close`, `tab.create/close`, `pane.split/close`, `plugin.pane.open` (process start or stop) | 10 s |
  | Reads, focus, resize, rename, `pane.send_text` and the event-subscription acknowledgement | 2 s (unchanged) |

  The terminal handshake bound is now 10 s. A hung Herdr is still bounded everywhere.
- **Repaired (gateway 351033):** the same setup, with branch `huge-task-2`, completed in one pass, 7.7 s from **Start setup** to "Space and terminal are ready". Terminal `w4:p2` streamed (`FLOW-01-setup-large-repo-green.png`).
- **Tests:**
  - `worktree_create_slower_than_the_read_deadline_still_succeeds`: a fake Herdr answers after 2.5 s. It fails against the old deadline and passes with the repair.
  - `slow_mutations_get_deadlines_sized_for_their_work`
  - The handshake-timeout test now passes an explicit 200 ms deadline to `open_terminal_within`.

## OBS-046: repository discovery hid most repositories from setup

Discovery walked depth-first and read each checkout's contents before visiting that checkout's siblings. The shared `catalog_entries` budget (default 1,024) was therefore spent inside the first large trees.

- **Fixture B, baseline:** even with `catalog_entries = 100000`, only `hugerepo` was listed, and the sibling `source` was absent.
- **The user's configured root** (`/home/nnex/dev/prj`, read-only):
  - Old and new gateways ran under a private config and state directory, pointed at the fixture's Herdr.
  - Discovery ran only `git rev-parse` / `git symbolic-ref`, and no file under the root was newer than the private config afterwards.
  - Results:

    | Build | Repositories listed | Diagnostics | Time |
    | --- | --- | --- | --- |
    | Baseline | 26 | `catalog_entries_bounded` | 263 ms |
    | Repaired | 70 | none | 637 ms |

  - A read-only Python replica of the walk measured 5,786 entries (17 ms) for the whole tree to depth 3.
  - Repository names are withheld from the evidence.
- **Repair:**
  - `RepositoryCatalog::list` walks breadth-first and admits each child checkout as soon as its parent is listed. A budget stop inside one large tree no longer hides repositories nearer the root.
  - Nested checkouts are still found, including the configured root when it is itself a repository, as it is for this user.
  - The default `catalog_entries` is 16,384; the accepted range 1–100,000 is unchanged.
  - `docs/discovery-limits.md` is updated.
- **Repaired (gateway 363393, default budget):** fixture B listed `source`, `hugerepo` and both linked task worktrees, with an honest `catalog_entries_bounded` diagnostic from the 120k-file tree.
- **Test:** `a_large_repository_does_not_hide_its_sibling_from_a_bounded_scan` fails against the old walk (`["a-large"]`) and passes with the repair.

## Checks

- `cargo test --workspace`: all suites pass (315 tests, including 141 cockpit-core unit tests and 56 cockpit-herdr unit tests).
- `rustfmt --check` passes on the changed files, and no clippy warning is attributed to the changed lines.

## Criterion matrix (this increment only)

| # | Result | Observation / limit |
| --- | --- | --- |
| 7 | **PASS for browser gateway-crash recovery** | A `pending` receipt left by a SIGKILLed gateway became durable `outcome_unknown` with no automatic resend. The UI and server required explicit acknowledgement, and the acknowledged retry delivered exactly one copy. Only the undelivered branch was run: Herdr never received the held write. The delivered-then-crash branch shares the same recovery code but was not exercised. |
| 1 | **Repair only** | Setup on large repositories no longer reports a false unknown outcome, and the setup picker lists every repository in a real project tree. Other setup and lifecycle gates remain as recorded in `FLOW-01.md`. |
| Others | Not in this increment | See `FLOW-01.md`, `FLOW-01-resume*.md` and `FLOW-01-paste-reconciliation.md`. |

Native parity was not rerun. The repairs are in the shared `cockpit-core` and `cockpit-herdr` crates that the native host also links.

## Cleanup

- Fixture A shut down when a misconfigured restart made its gateway exit 1 (`catalog_entries = 200000` is outside the accepted range). Herdr exited 0 and the proxy was terminated. The fixture now waits for RESTART/STOP instead of shutting down. Evidence was copied out, and none of its processes remained.
- Fixture B stopped with Herdr exit 0, gateway exit 0 and the proxy terminated. No listed PID remained, the session socket and port 59825 were released, and no process referenced the root.
- The two discovery gateways were terminated, their ports 38869 and 40319 were released, and their private temporary directory was removed.
- Both fixture roots were removed, and the `flowpaste` browser session was closed.

## Remaining FLOW-01 work

Still open:

- Native Review/comment parity and teardown.
- Herdr server/session-process restart with pending-intent recovery.
- The delivered-then-crash paste branch.
- Provider source import.
- The native full-workflow comparison.
