# Measurement recipes (S0 / S7)

Throwaway recipes, run for real on 2026-09-28 (results: [`baseline-2026-09-28.md`](baseline-2026-09-28.md)). Put the scripts under `/tmp/cockpit-perf-*`, never in the repository.

Rules:
- Use a disposable fixture only. Never touch the user's Herdr session or gateway.
- Before and after, `stat -c %Y ~/.config/herdr/plugins.json` must print the same value.
- Note the hardware and filesystem: `/tmp` is tmpfs on the reference machine, so fixture numbers are warm and diskless.
- Use the **release** binary (`target/release/cockpit`); `ui_polish_runtime.py start` launches the debug one. Check that no `.rs` file is newer than it.

## What was wrong in the first draft (fixed below)

- The 70 empty repos took 0 ms to scan per file, so they under-represented real checkouts. Give the small repos ~150 files.
- `big` (50k tracked files) and any checkout with a > 1 MiB `git diff --binary` fail Review with `bounded_output`, so Review timings need a separate mid-size repo (`mid`, ~0.5 MB diff).
- `plugin.action.invoke` opens Files at the plugin directory; the Review pane needs `POST …/review/open` from the terminal pane.
- The gateway port changes on every restart; re-read it from `gateway.log`.
- Per-operation git counts are polluted by the browser's polls unless the tab is on `about:blank`.
- A picker-status check for `/Index/i` also matches "index incomplete"; use `/Indexing/`.
- `Runtime.callFunctionOn` times out after ~180 s; start long picker runs without awaiting them and poll `window.__r4`.
- Closures are not captured by `tab.run`; pass data through `args`.
- `git init -b main` avoids the default-branch hint noise.

## R0 — Fixture

1. `R=$(python3 scripts/verify/ui_polish_runtime.py start --with-plugins | tail -1)`.
2. Generate data (`/tmp/cockpit-perf-build.py` is the script that was used; re-create it from this description):
   - 69 small repos `r00..r68` (150 `.ts` files each, one commit);
   - `big`: `src/sNN/dNNNN/f0..f9.ts` for 5,000 dirs (50,000 tracked files), `target/debug/o0..o4999.o` ignored via `.gitignore`, then one appended line to `src/s00/d0000/f0.ts`;
   - `bigdiff`: 6,000 tracked files of 60 lines, 1,500 modified, 300 untracked (diff ≈ 11 MB);
   - `mid`: `git clone --local --no-hardlinks <cockpit checkout>`, append 38 lines to 160 tracked `.ts/.tsx/.rs` files (diff ≈ 507 KB), add 25 untracked files;
   - `/tmp/cockpit-perf-plain`: 5,000 dirs × 10 files, not a git repo.
3. Configure `$R/cockpit.toml`: `repository_roots = ["/tmp/cockpit-perf-repos"]`, `library_root = "$R/library"`, the fake Confluence provider block and `[limits] library_space_pages = 2000`. The fake CLI and the 1,200-page follow are in `skill://cockpit-large-library-perf-fixture`; an already-followed `library/` directory can be copied into the new root (index paths are relative).
4. Restart the gateway with `/tmp/cockpit-perf-launch.py`: kill the pgid in `$R/gateway.pid`, then `ui_polish_runtime.launch(root, "gateway", [target/release/cockpit, serve, --herdr-session S, --herdr-socket SOCK, --config cfg, --static-dir dist], ui_polish_runtime.environment(root))`. Add the shim directory to `env["PATH"]` only for count runs. Save the port from `gateway.log`.
5. Workspaces and panes (all through `ui_polish_runtime.py rpc` and the HTTP API, never the `herdr` CLI):
   - `workspace.create {"cwd": "/tmp/cockpit-perf-repos/mid", "label": "mid"}` → terminal pane `wN:p1`;
   - Review: `POST /api/v1/sessions/S/review/open` with `{"pane_id", "binding_id", "repository_id", "direction": "right"}` (values from `GET …/presentation` of the terminal pane);
   - Files: `plugin.action.invoke {"plugin_id": "herdr-file-viewer", "action_id": "open-file-viewer", "workspace_id": "wN"}`.
   - Repeat for `big` (Review will 503) and `bigdiff`.

## R1 — Count git spawns

Shim first on the **fixture gateway's** `PATH` only, not Herdr's:

```sh
mkdir -p /tmp/cockpit-perf-shim
cat > /tmp/cockpit-perf-shim/git <<'EOF'
#!/bin/sh
printf '%s %s\n' "$(date +%s.%N)" "$*" >> /tmp/cockpit-perf-git.log
exec /usr/bin/git "$@"
EOF
chmod +x /tmp/cockpit-perf-shim/git
```

- Per operation: put the browser tab on `about:blank`, `: > /tmp/cockpit-perf-git.log`, issue exactly one request, `wc -l`. Expect 4R + 5 for any Context request today.
- Idle: open the browser on the `mid` workspace (terminal + Review + Files visible), wait 5 s, truncate the log, `sleep 60`, `wc -l`, and `cut -d' ' -f2- log | sed 's/-c core.hooksPath=\/dev\/null -c core.fsmonitor=false //' | awk '{print $1,$2}' | sort | uniq -c | sort -rn`.
- Also run the same window with **no** browser to prove the load is polling.
- Do **not** take latency numbers with the shim on.

## R1b — Idle CPU

Shim off. `/tmp/cockpit-perf-cpu.py <gateway pid> 30` prints deltas of `utime+stime` (gateway) and `cutime+cstime` (waited children, i.e. git) from `/proc/<pid>/stat` as a percentage of one core.

## R2 — HTTP latency probe

Only non-GET requests need `Origin`, which must be the exact bound origin. Use `/tmp/cockpit_perf_lib.py` (Python `urllib`, `time.perf_counter`, p50/p95 helper) rather than `curl` loops.

```
GET  /api/v1/sessions/S/panes/P/presentation
POST /api/v1/sessions/S/panes/P/context/directory  {"binding_id","root_id","path":"src/s00"}
POST /api/v1/sessions/S/panes/P/context/document   {"binding_id","root_id","path"}
POST /api/v1/sessions/S/panes/R/review/snapshot    {"binding_id","repository_id","comparison":"all_local","base_ref":null}
POST /api/v1/sessions/S/panes/R/review/file        {"binding_id","review_id","generation","file_id"}
GET  /api/v1/project/repositories                  (the bare scan)
GET  /api/v1/sessions/S/space-git
```

- Take `binding_id` and `root_id` from `GET …/presentation`, `repository_id` from the terminal pane's repository root.
- Sequential, n=20 (10 for the slow ones). Report min, p50, p95.
- For throughput: `ThreadPoolExecutor(8)` over 16-32 different directories.
- Review size limits: append lines until `git diff --binary | wc -c` crosses 1,048,576 and record the last size that returns 200.

## R3 — Library

```
GET  /api/v1/library?offset=N          (follow next_offset until null; time the whole loop, n=10)
POST /api/v1/library/directory         {"path": ""} and {"path": "confluence"}   (n=20)
```

- Serialisation check: time 160 different directories at concurrency 1 and 8; equal per-request time means the flock serialises them.
- Record `ls -l $R/library/.cockpit/index.json` and the item/file/dir counts.

## R4 — Picker open → full list (browser)

`browser.open` a tab at the gateway, click the workspace button, then run in the page (start it **without** awaiting the whole run; poll):

```js
window.__r4 = {done: false, seen: []};
const btn = [...document.querySelectorAll('.viewer-file-picker-trigger')].find(b => /Context file/.test(b.title));
const t0 = performance.now(); btn.click();
// every 250 ms: read .file-picker-status; done when it no longer matches /Indexing/
```

- Record `performance.now() - t0` and the final status text (`N files`, `· index incomplete`).
- Reopen: Esc, then repeat.
- Library picker: in a Files pane, `page.select('select', 'library')` on the Context-root combobox, then the same click.
- After S2, also record the time to the first rendered result row and the "Refreshing" phase.
- Only one `__r4` loop may exist per page; reload the page between runs.

## R5 — Keystroke cost

With the picker open and the list loaded, in the page's CDP session:

1. Clear the input (triple click + Backspace) and wait 0.5-0.8 s.
2. Read `Performance.getMetrics` (`ScriptDuration`, `TaskDuration`, `LayoutDuration`, `RecalcStyleDuration`).
3. Type the query one character at a time with `page.keyboard.type`, waiting two rAFs per character.
4. Read the metrics again and divide the deltas by the number of characters.
5. Use several queries, including one with zero matches (ranking cost is independent of match count) and one with two tokens (`f3 d09`).
6. Idle control: the same deltas over 5 s with nothing typed (was 2 ms).
- Pure ranking: import `src/app/input/fileNavigation.ts` into a throwaway page (`bun build … --target=browser` to an IIFE) and time `rankFileMatches` over 3k / 10k / 50k synthetic paths in a **blank** Chromium page (V8), not in bun (JSC gives ≈ 5× slower numbers).

## R6 — Enumeration and persisted-list primitives

- `/tmp/cockpit-perf-git.py`: p50 of 7 for `git ls-files -z --cached --others --exclude-standard`, `--cached` only, `--others`, `--stage -z`, `rev-parse HEAD`, `status --porcelain` on each fixture repo and on this repo (read-only).
- `/tmp/cockpit-perf-rs/walk.rs` (`rustc -O`): `read_dir` + `file_type()` walk that skips `.git`, sorts, and honours an optional cap.
- `/tmp/cockpit-perf-rs/cachebench` (offline cargo, `serde` 1.0.228, `serde_json` 1.0.149): serialise, write+rename, read and parse a 50k-path list, an `lstat` of every directory, and a parse of the Library `index.json`.
- **Cold measurement (S6 acceptance):** needs a cold page/dentry cache. Either `echo 3 | sudo tee /proc/sys/vm/drop_caches` (needs root), or the first run after a reboot. Record both the first and second run of the enumeration and of the persisted-list load.

## Cleanup

- `python3 scripts/verify/ui_polish_runtime.py stop <ROOT>`.
- Close the browser tab. Confirm `ps aux | grep <root>` is empty.
- Keep `/tmp/cockpit-perf-repos` and `/tmp/cockpit-perf-plain` if a later slice will re-measure; otherwise `rm -rf /tmp/cockpit-perf-*` and old `/tmp/cpol-*` roots you created.
- Re-check the `plugins.json` mtime.
