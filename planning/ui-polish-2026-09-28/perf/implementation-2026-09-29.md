# Performance implementation and smoke evidence — 2026-09-29

Fixture: disposable Herdr session `polish-wnsuufa_`, fixture root `/tmp/cpol-wnsuufa_`, served by a release `cockpit` gateway. Large checkout `big` contains 50,000 candidate paths. Timing is warm `/tmp`/tmpfs and includes HTTP transfer + JSON decoding; it is not a cold-disk or isolated server benchmark. Owner `~/.config/herdr/plugins.json` mtime was `1790518335` before and after the run.

## Implemented paths

- Repository-catalog read cache with a mutation generation, bounded stale-while-refill, single-flight refresh and startup prewarm. Fresh authorization paths remain fresh.
- Context file-index API (`cached` / `fresh`) over server-side Git enumeration or bounded no-follow folder walk. The list is an authorization-independent hint; open/read revalidates against the authorized root.
- Persisted cache under `cache_root/file-index/v1`, with canonical root + device/inode identity, bounded entry/list sizes, expiry, 0700 directories, 0600 files, atomic replacement and corruption-as-miss behavior. Library file listings remain in-memory only.
- Library index identity cache and deferred/limited picker ranking; review snapshot reuse guarded by current Git tokens and streamed revision hashing.

## Review round (2026-09-29) — high-severity findings fixed

An independent review reproduced three high-severity problems, all fixed and covered by regression tests: (1) per-directory discovery canonicalized the runtime cwd before validation, so a symlinked cwd was accepted; (2) a cached per-directory discovery kept authorizing the parent checkout after a nested `git init`; (3) snapshot reuse returned a stale untracked-file diff after a same-size, mtime-preserving rewrite. Fixes: per-directory discovery is no longer cached (about 10 ms of git work; the catalog scan, the 574 ms part, stays cached), and snapshots containing untracked files are rebuilt instead of reused. The concurrency and bounded-output/streamed-hash categories had no high-severity findings. Also fixed: the Tauri ACL (`src-tauri/build.rs`, `capabilities/default.json`) lacked `cockpit_context_file_index` and `cockpit_library_file_index`, so the native picker failed with "Could not load files" while the browser build worked; confirmed native afterwards (picker lists `README.md`).

## Observed verification

- 20 sequential `fresh` POSTs for the 50,000-path Git index: all HTTP 200, all 50,000 paths, truncated at the cap; p50 **608.5 ms**, p95/max **753.1 ms**; response **2,100,257 B**.
- 20 sequential `cached` POSTs after restart: all HTTP 200, all 50,000 paths; p50 **19.1 ms**, p95/max **22.1 ms**; response **2,200,259 B**. These include transfer of the ~2.2 MB JSON response.
- The final release-gateway restart retained and served the persisted list (HTTP 200, 70.6 ms end-to-end for one cached response). Cache files were observed at 0600 under 0700 `file-index` directories.
- Replacing the disposable JSON cache entry with malformed data, restarting, then requesting `cached` returned `state: miss` and zero files. A following `fresh` request rebuilt and returned 50,000 paths (`state: fresh`, `source: git`).
- Browser proof on `Performance big`: the picker returned and opened `src/s49/d4999/f8.ts`; its source row was `export const v4999_8 = 8;`. A disposable Library fixture with 1,200 Markdown documents returned `Page-1199.md` in `Find in Library`.
- `bun run test`: 47 files, 390 tests passed. `bun run build`: TypeScript check and production Vite build passed (existing chunk-size warning). `cargo test -p cockpit-core`: 258 unit + 22 integration tests passed. `cargo test -p cockpit-host --test server`: 15 passed. `cargo test -p cockpit-providers`: 60 unit + 5 CLI integration + 5 source-contract tests passed. `cargo build -p cockpit-host --bin cockpit --release` passed.

- Cache regression coverage verifies a missing disk entry is rewritten despite a memory hit, expired entries refresh, and failed persistence retries after the cache directory becomes available.

## Limits and unverified paths

- The Review pane command was disabled in this fixture because it had no authorized repository/companion association. Review reuse is covered by Rust tests but was not exercised against a live Review pane here. A native Tauri runtime smoke was not run.
- The persisted index is a path hint and survives gateway restart, but does not implement the full D10 validator payload (`HEAD`, index/ignore/root mtimes), a validator-based probably-current label, hourly LRU touch, or asynchronous post-response persistence. Cached candidates remain labeled potentially stale by the UI. Persistence currently serializes the protocol file records (cached `bytes` are null) rather than the exact proposed `files: string[]` JSON. Retention cleanup is lazy on cache access rather than a gateway-start background task. Cross-process eviction coordination is not implemented.
- The measured fresh/cached numbers are warm local fixture HTTP wall times, not the proposed 20-run multi-scenario S7 suite, isolated server CPU, a cold-disk run, or proof of sub-second UI latency on a different host.
