# 04 — Performance and caching (file search, Library, Review diffs)

Status: S0 baseline and implementation evidence are recorded; see [`perf/baseline-2026-09-28.md`](perf/baseline-2026-09-28.md) and [`perf/implementation-2026-09-29.md`](perf/implementation-2026-09-29.md). Recipes: [`perf/measurement-recipes.md`](perf/measurement-recipes.md). The evidence file names unverified and not-yet-implemented portions of the plan.

> **Number provenance.** **measured** = taken on 2026-09-28/29 with the recipes (hardware and fixture in the baseline file: Ryzen 7 7840U, `/tmp` on tmpfs, so fixture numbers are warm and diskless). **counted** = an exact operation count read from the code or a git-shim log. **[INFERENCE]** / **[extrapolated]** = not measured. **No cold-disk number exists**: dropping caches needs root. S7 re-measures against §7.

## 1. Outcome

- Opening the file picker (`Ctrl+P` / search button) on a large repository or Library shows candidates immediately, and typing stays smooth.
- Opening, switching back to, or refreshing Context/Files/Review panes no longer re-walks the repository roots or re-spawns hundreds of `git` processes, and an open UI no longer burns ≈ 55 % of a core when idle.
- Review opens on any checkout, including ones whose diff or index exceeds 1 MiB.
- Reopening an unchanged Review reuses its snapshot and the diffs already parsed for it.
- The picker's file list survives a gateway restart and browser reload (persisted, revalidated on every open).
- Herdr stays the only authority. No cache holds Herdr pane, process, or focus evidence.

## 2. Evidence

### 2.1 Root cause: every Context/Review request re-runs repository discovery

- `ContextService::directory` / `document` call `inspect_pane` on every request (`crates/cockpit-core/src/context.rs:237-261`).
  - `inspect_pane` → `presentation` → `authorized_roots` (`context.rs:467-479`).
  - `authorized_roots` first runs `RepositoryCatalog::new(..).list()` and then `discover_checkout(cwd)` (`context.rs:614-625`). It does this even for Files viewer panes, whose loop skips every repository (`context.rs:722-724`: `if viewer_folder.is_some() { continue; }`).
- `RepositoryCatalog::list` (`crates/cockpit-core/src/repositories.rs:30-163`) walks each configured root to `catalog_depth` 3, up to `catalog_entries` 16,384 (defaults at `crates/cockpit-core/src/config.rs:285-296`).
  - For every checkout it finds, `admit_checkout` → `inspect_checkout` runs **4 sequential git processes** (`repositories.rs:340-456`): `rev-parse --show-toplevel`, `--git-common-dir`, `symbolic-ref`, `rev-parse HEAD`.
  - The `read_dir` walk is synchronous inside an async fn, so it blocks a tokio worker.
  - **Measured (R=72 checkouts): 574 ms per `list()`** (`GET /api/v1/project/repositories`, p50 of 10), **288 git spawns ≈ 2.0 ms each**. Earlier run: 637 ms at R=70 (`planning/stability-and-gitlab-2026-09-20/runs/run-20260920-a3e9b950/FLOW-01-restart-and-deadlines.md:80-85`).
  - The owner's real root has 54 checkouts, so the same path costs ≈ 434 ms there **[extrapolated]**.
  - The scan parallelises across workers (8 concurrent requests: 68 ms/request throughput) but each request still waits ≈ 580 ms.
- The Herdr part of `inspect_extension_pane` is cheap. It makes 4 socket reads per call (`ping`, `session.snapshot`, `pane.process_info`, `pane.get`; `crates/cockpit-herdr/src/cli/extensions.rs:481-523`). All of them are allowlisted read methods answered in under 1 ms (`crates/cockpit-herdr/src/cli/operations.rs:39-53`; latency facts in `skill://cockpit-disposable-herdr-fixture`). `GET /api/v1/status` measured 0.5 ms.
- `context_companions` reads every operation record and companion manifest on each call (`crates/cockpit-core/src/projects.rs:1145-1158`).
- `resolve_viewer_root` spawns one `git rev-parse` (`context.rs:1527-1561`).
- **Counted, confirmed by the git shim:** every Context request (presentation, directory, document, review file) is **4R + 5 = 293 spawns** at R=72, and **575-580 ms** at p50 whatever it does (Files pane, terminal pane, Review pane, cached diff). The real work in a directory or cached-diff request is milliseconds.

### 2.2 Who triggers it, and how often

| Trigger | Code | Repository discoveries per trigger |
| --- | --- | --- |
| Presentation poll: every **2.5 s** for each visible graphical pane, 3 at a time | `src/app/paneRenderers.ts:95-121` | 1 |
| Context invalidation poll: every **3 s** for each Context pane with open documents | `src/app/context/ContextSearch.tsx:12,142-182` → `context_search.rs:69-99` → `authorize_companion_root` (`context.rs:160-178`) | 1 |
| Each directory page or document read | `context.rs:237-261` | 1 |
| Picker open: one directory request per directory | `src/app/context/ContextViewer.tsx:978-1039` | **D** (directory count) |
| Review snapshot: runs on every `ReviewPane` mount, comparison change, or Refresh | `src/app/review/ReviewPane.tsx:220-270`; `review.rs:134-231` (`inspect_pane_with_evidence` + `authorize_review_pane`) | 2 catalog lists + discover: **8R + 27 = 603 spawns** (counted) |
| Review file open: every file click, even when the diff is already cached | `review.rs:233-265` → `active_snapshot` → `authorize_review_pane` (`review.rs:744-746, 806-825`) | 1 |

- **Measured idle cost** (workspace with terminal + Review + Files visible, nothing clicked): **12,577 git spawns per 60 s (≈ 210/s, ≈ 43 scans/min) and ≈ 55 % of one core** (git 46 %, gateway 8 %). With no browser connected: 0 spawns. This is the constant background load behind "sluggish", on top of every click paying 575 ms.
- Panes remount whenever their tab is hidden or the Library opens. The code renders only the current and outgoing projection (`src/app/App.tsx:1610`) and renders no panes while the Library is open (`App.tsx:1627`).
- So every switch back to a Review tab re-runs a full snapshot.

### 2.3 The file-picker "index build"

- Client-side breadth-first walk in `ContextViewer.openFilePicker` (`ContextViewer.tsx:978-1039`):
  - one HTTP request per directory page;
  - 8 concurrent (`PICKER_DIRECTORY_CONCURRENCY`);
  - caps of 10,000 directories and 10,000 files (`ContextViewer.tsx:180-182`).
- **Measured:** `mid` (363 dirs, 2,969 files) **32.3 s**, and **32.2 s on immediate reopen**. `big` (55,001 files) **53.6 s** and it ends `10000 files · index incomplete`: only 10,000 of 55,001 files are searchable. The Library (1,203 dirs) **43.8 s**, because the exclusive flock serialises the 8 workers (32.6 ms serial vs 33.5 ms at 8 concurrent). The real cockpit checkout has 13,908 dirs and 193k files on disk (`target/`: 141k), so its picker always hits both caps.
- **Discarded on close** (`closeFilePicker`, `ContextViewer.tsx:971-977`) and on identity change (`ContextViewer.tsx:894-899`), so every open rebuilds it from nothing.
- There is no ignore handling beyond `.git` and Cockpit metadata (`reserved_context_path`, `context.rs:1278-1311`). `target/`, `node_modules/` and similar folders are walked and use up the 10k caps.
- Every 100 ms it publishes a full `new Map(entries)` (`ContextViewer.tsx:1030-1033`).
- The `<FilePicker candidates=…>` array is rebuilt inline on every `ContextViewer` render (`ContextViewer.tsx:1748`). Because of that, `FilePicker`'s `useMemo` re-ranks every candidate on each parent render as well as on each keystroke (`src/app/input/FilePicker.tsx:17`).
- `rankFileMatches` calls `Array.from` + `toLocaleLowerCase` for every candidate, token, and call, then sorts everything (`src/app/input/fileNavigation.ts:26-58, 75-97`).
  - **Measured in Chromium:** **134-176 ms of script per keystroke at 10,000 candidates** (`big`), **180-270 ms at 2,969** (`mid`, longer paths), 2 ms idle. The function alone takes 46-55 ms at 3k, 111-122 ms at 10k, **561-584 ms at 50k**, whatever the match count.
  - A prepared-ranking prototype with identical scoring (same top-50 on the tested queries) takes 1.7-2.7 ms at 10k and 8.5-9.4 ms at 50k.
- The Review picker only ranks the snapshot's changed files (`ReviewPane.tsx:540`), which is cheap.

### 2.4 Library store read path

- `LibraryService::open()` runs on **every** Library call after the first (`crates/cockpit-core/src/library.rs:135-148`):
  - `store.recover_pending()`: exclusive `flock` plus `recover()`, which **parses the whole `index.json` before looking at the journal** (`library/store.rs:288-291, 820-822`);
  - `space::recover_attempts`: another exclusive `flock` plus **another full index parse** (`library/space.rs:188-192`).
- `index()` parses to `serde_json::Value`, then again with `from_value`, then validates every entry (`library/store.rs:320-348`). The index is pretty-printed JSON capped at 64 MiB (`store.rs:19, 1284-1295`).
- **Counted** parses per call:
  - `listing(offset)`: 3 full index parses (2 in `open()` + 1) per 256-item page (`library.rs:149-170`). The client reads all pages on every Library open (`src/app/library/useLibraryOperation.ts:181-221`).
  - `directory` / `document` / `media`: 2 parses + 3 flocks per request (`library/reader.rs:25-77`).
- **Measured (1,200 items, 2.35 MB index; one parse to `Value` = 7.0 ms):** a full listing (5 pages) **261 ms** p50; one page 51 ms; one directory request **32 ms**, serialised across concurrent readers. Extrapolating the parse cost linearly, a 20,000-item Library (≈ 40 MB) parses in ≈ 120 ms per call **[INFERENCE]**, i.e. 79 pages × 3 parses ≈ 28 s per full listing **[INFERENCE]**.
- None of these handlers use `spawn_blocking`. The only core users are search, review file I/O, comments, and folder capture (checked with a grep for `spawn_blocking`).
- `index.json` already holds a trusted per-item file inventory (`LibraryIndexEntry.inventory`, `library/store.rs:26-34`). That is the "primary metadata on disk" the request refers to.

### 2.5 Review diff path

- `snapshot` reads `revision_tokens` twice (before and after `collect`) (`review.rs:188-192`). Each read runs:
  - `rev-parse HEAD`;
  - `ls-files --stage -z`, hashed;
  - a full `git diff --binary`;
  - `ls-files --others` (`review.rs:1416-1449`).
- `collect` for AllLocal runs name-status ×2, numstat ×2, and `status --untracked-files=all` (`review.rs:854-1000`).
- **Counted and measured:** 603 spawns = 8R + 27 per `mid` AllLocal refresh; **1,456 ms p50** (p95 1,499). The git work itself is ≈ 220 ms (tokens 74 ms × 2 + collect 71 ms); **≈ 1,150 ms (79 %) is two repository scans.**
- The token commands use the default `git_output_bytes` cap of 1 MiB (`review.rs:1478-1489`; `config.rs:303-308`).
  - **Measured: the snapshot fails with `503 bounded_output` as soon as `git diff --binary` exceeds 1,048,576 bytes** (1,010,893 B works, 1,057,693 B fails), for every comparison including `staged` and `untracked`. It also fails on `big` (`ls-files --stage -z` = 3.5 MB, ≈ 70 B per file, so ≈ 15,000 files) and on `bigdiff` (11.2 MB).
- Each snapshot mints a new `review_id` (`review.rs:194`). Parsed per-file diffs are persisted per `review_id` (`save_file_cache` / `load_file_cache`, `review.rs:287-299, 353, 1600-1700`). As a result, **no refresh ever reuses an earlier parsed diff**, even when nothing changed. The real work of a first file click is ≈ 10 ms (597 ms vs 587 ms cached).
- `ReviewFileTree` computes `duplicates` with a nested `files.some` inside `files.filter`, which is O(n²) on every render (`ReviewPane.tsx:104`). **Measured:** 2,000 files 65 ms, 5,000 files 146 ms per render (185 files 0.6 ms).

### 2.6 Already cheap (no change planned)

- Space git status: every 15 s (`src/app/session/spaceGitStatus.ts:5`). It makes 3 git calls per distinct checkout, deduplicated (`crates/cockpit-core/src/space_git.rs:47-104`). **Measured: 17 ms p50, 7 spawns.**
- Herdr schema is cached. `plugin.list` has a TTL cache (`extensions.rs:146-184, 194-211`).
- The syntax highlighter is size-capped (`src/app/viewer/highlight.ts:61-63`).
- Server enumeration primitives (warm): `git ls-files -z --cached --others --exclude-standard` is **86 ms for 50,000 tracked files, 10 ms for this repository even with 193k files on disk**; a no-follow Rust walk of 50,000 files takes 64 ms and the Library walk 9.7 ms. Enumeration is not the bottleneck.

## 3. Baseline table (measured 2026-09-28; R=72 unless noted)

| # | Path | Trigger | Work per trigger (counted) | Measured baseline | Blocking |
| --- | --- | --- | --- | --- | --- |
| B1 | `RepositoryCatalog::list` | Every B2–B9 request | Walk + 4 sequential git spawns per checkout = 4R | **574 ms** p50, 288 spawns (R=72) | Tokio worker (sync `read_dir`) |
| B2 | Context request (presentation, directory, document) | Tree expand, doc open, picker, polls | B1 + discover (4 git) + viewer root (1 git) + 4 Herdr reads + companion scan | **573-580 ms** p50, p95 ≤ 599, **293 spawns**; same for Files, terminal and Review panes | Server |
| B3 | Files picker | Every open; thrown away on close | D × B2, 8 concurrent | `mid` (D=363) **32.3 s**, reopen 32.2 s; `big` **53.6 s to 10,000 of 55,001 files** ("index incomplete") | Server scan; client Map copy every 100 ms + full re-rank |
| B4 | Library picker | Every open | D × (2 index parses + 3 flocks + readdir), serialised by flock | D=1,203: **43.8 s** (1,201 files) | Server worker threads |
| B5 | Library listing | Each Library open and `LIBRARY_CHANGED` | ⌈N/256⌉ pages × 3 index parses | N=1,200: **261 ms** (5 pages, 51 ms each); N=20k ≈ 28 s **[INFERENCE]** | Server |
| B6 | Presentation poll | Every 2.5 s per visible graphical pane | B1 + discover + Herdr inspect | **575 ms each; idle total 12,577 spawns and ≈ 55 % of a core per minute** with 3 panes | Server CPU |
| B7 | Context invalidation poll | Every 3 s per Context pane with docs | B1 + discover + ≤128 stats | Included in B6's idle total (not separated) | Server CPU |
| B8 | Review snapshot (AllLocal) | Each mount, tab return, Refresh | 2×B1 + discover + 2 token reads + collect | `mid`: **1,456 ms** p50, **603 spawns**; git work only ≈ 220 ms; **fails `bounded_output` above 1 MiB diff / ≈ 15k files** | Server |
| B9 | Review file click | Each selection | B1 + first time only `git diff` + parse | **587 ms** cached, 597 ms first; real work ≈ 10 ms | Server |
| B10 | `FilePicker` ranking | Each keystroke **and** each parent render | Full `rankFileMatches` over all candidates | **134-176 ms/key at 10k**, 180-270 ms/key at 3k, ≈ 570 ms at 50k (function only) | **UI main thread** |
| B11 | `ReviewFileTree` duplicates | Each render | O(n²) | 65 ms at 2,000 files, 146 ms at 5,000 | **UI main thread** |
| B12 | Space git status | Every 15 s | 3 git per distinct checkout | 17 ms, 7 spawns | none (already cheap) |

## 4. Ranked opportunities (by measured impact on "sluggish")

| Rank | Opportunity | Measured impact | Effort | Risk | Slice |
| --- | --- | --- | --- | --- | --- |
| 1 | Cache repository discovery for read-path authorization (stale-while-revalidate, prewarmed at start); skip it for Files viewers | Every request 575 ms → ≈ 0-30 ms; idle 12,577 → ≤ 700 spawns/min, ≈ 55 % → ≤ 3 % of a core | S–M | Medium: adjacent to authorization; one review round | S1 |
| 2 | One-shot server file index (git-aware) + prepared ranking + client SWR | Picker 32-54 s → ≈ 0.3-0.7 s and complete (not 10k of 55k); keystroke 134-176 ms → ≈ 3-9 ms at 10k-50k | M | Low: read-only, same authorization | S2 |
| 3 | Stream-hash revision tokens | Review opens on > 1 MiB diffs and > 15k-file repos (today: 503) | S | Low | S5 (**required**) |
| 4 | Review snapshot reuse on equal tokens + O(n) duplicates | Refresh 1,456 ms → ≈ 350 ms changed, ≈ 120 ms unchanged (with S1); file click 587 → ≤ 30 ms | S–M | Low–medium: review_id reuse; one review round | S4 |
| 5 | Library store read path | Listing 261 → ≤ 60 ms; directory 32 → ≤ 10 ms; picker 43.8 s → (S2 server index) ≈ 0.3 s | S | Medium: recovery/concurrency; one review round | S3 |
| 6 | Persisted picker file list (XDG cache dir), served stale, revalidated on every open | ≤ ≈ 80 ms of warm enumeration at 50k; first paint after restart ≤ 100 ms; cold-disk gain unmeasured | M | Low: hint only, never authoritative | S6 (**user decision: in scope**) |
| — | Persisting repository scans, Review snapshots/diffs, or the Library index | ≤ 575 ms once per restart / ≤ 200 ms once / ≈ 10 ms / 7 ms once | — | — | **Not persisted** (numbers in D10) |

## 5. Decisions

**D1 (revised). Remove the per-request overhead first; add a persisted picker file-list cache after it, as a hint.**
- Evidence: the picker cost is D × (repository discovery + index parses) + main-thread ranking, not enumeration (§2.6: 10-86 ms).
- The user decided a persisted cache is in scope ("it feels sluggish"). It is sequenced last (S6) because S1-S5 remove ≥ 99 % of the measured cost and the persisted list only removes the remaining enumeration (≈ 80 ms warm at 50k) and the cold-disk case.
- The list is a **hint**, never authoritative: revalidated on every open, never used for any write, teardown or authorization decision (D10).
- Still rejected: an fs watcher (no `notify` crate in `Cargo.lock`, inotify limits) and trusting a fingerprint to skip revalidation.

**D2 (amended). `RepositoryDiscoveryCache`, owned by `ProjectService`, used only by read-path authorization.**
- In memory. Stores the last `RepositoryListResponse` and a bounded map (64 entries) of `discover_checkout` results, keyed by canonical cwd + (dev, ino).
- **Freshness, revised by the measurement:** a refill costs 575 ms and 4R spawns, so a hard 30 s TTL would stall a user action every 30 s and would still cost ≥ 576 spawns/min.
  - Age < **30 s** and `ProjectStore::mutation_generation()` unchanged → serve.
  - Age 30 s – **5 min** and generation unchanged → serve the stale value **and** start one background refill (single-flight). The refill is triggered only by a request, so with no UI connected nothing runs.
  - Older than 5 min, or generation changed → wait for a fresh refill.
  - **Prewarm:** the gateway starts one background catalog fill at boot, so the first request after a restart normally hits.
- `mutation_generation` is a new `Arc<AtomicU64>`, bumped after every successful `ProjectStore::update`, `write_companion`, `reattach_companion`, and `remove_owned_companion`.
- Single-flight: one `tokio::sync::Mutex` around a refill, so concurrent misses await a single walk.
- Consumers:
  - `ContextService::authorized_roots` (`context.rs:614-625`);
  - `ReviewService::discover_checkout` (`review.rs:843-852`);
  - `ContextService::review_checkout_for_evidence` (`context.rs:449`).
- **Not** consumers: setup, plan, resume, and teardown freshness checks (`projects.rs:179, 1380, 1627, 1682`; `projects/defaults.rs:31`). They keep calling `RepositoryCatalog` uncached, as DECISIONS requires for mutations.
- `ProjectService::repositories()` (the Setup dialog list) stays fresh and **publishes** its result into the cache.
- A per-cwd `discover_checkout` **miss** is never served stale: an unknown cwd is resolved fresh (4 git spawns, ≈ 10 ms).
- Per-request checks stay fresh and unchanged:
  - Herdr evidence;
  - companion validation;
  - `canonical_directory` and `filesystem_identity` of every admitted root (`context.rs:726-750`).
- A stale list can at most (a) list a checkout that has since vanished (opening it then fails, as today) or (b) omit a checkout created outside Cockpit in the last ≤ 5 min for **other** repositories' root lists, until the background refill lands (≈ 0.6 s). Panes whose own cwd is a new checkout resolve through the fresh per-cwd discover.
- Rejected:
  - caching `PanePresentation` or Herdr evidence (a second source of truth for Herdr state, and against the per-request proof in `context.rs:215-217`);
  - an fs watcher on the repository roots (new dependency, inotify limits);
  - **persisting the scan** (D10);
  - collapsing the 4 spawns per checkout into fewer (would cut the refill 2×; optional later, not needed once refills are ≤ 2/min).

**D3. Verified Files viewers skip repository discovery.**
- Their roots never include repositories (`context.rs:722-724`), and `current_repository_id` is unused when the renderer is Context (`context.rs:531-533`).
- Measured: a Files-pane presentation costs the same 575 ms / 293 spawns as any other pane.
- Reorder `authorized_roots` to: companions → viewer root → catalog and discover only when `viewer_folder.is_none()`.
- Consequence: repository-catalog diagnostics no longer appear on Files panes. They were unrelated to file browsing.

**D4 (resolved). Server-side file index, one authorized request per picker open; gitignored files are excluded in git checkouts.**
- **User decision recorded:** the picker excludes gitignored files in git checkouts (tracked + untracked-not-ignored, as VS Code and fzf do). The tree still shows ignored files. Measured reason: on the real cockpit checkout `git ls-files` lists 2,975 paths in **10 ms**, while the disk holds 193,104 files (141,497 in `target/`) and a plain walk stops at the 50k cap after 142 ms with the wrong files.
- New `ContextFileIndexRequest { binding_id, root_id, mode }` with `mode: "cached" | "fresh"` (`"cached"` is D10) → `ContextFileIndex { binding_id, root_id, files: Vec<ContextIndexedFile { path, bytes: Option<u64> }>, truncated, source: "git" | "walk", state: "fresh" | "cached" | "miss", diagnostics }`.
- Library variant: `LibraryFileIndexRequest { mode }` → the same response type. The Library list is never persisted (D10).
- Enumeration runs in `spawn_blocking`:
  - **Git roots** (Repository roots, and Folder/Companion roots whose `git rev-parse --show-toplevel` equals the root): `git ls-files -z --cached --others --exclude-standard`, with the same hardened env as `repositories.rs:469-488` and a 16 MiB output cap (`MAX_FILE_LIST_BYTES` pattern, `review.rs:34`; 50k × 115 B = 5.7 MB fits). Drop `.git` and reserved paths, and keep only regular files.
  - The per-path `symlink_metadata` must stay cheap: a Python `lstat` of 50k paths takes 234 ms. Resolve file type without a per-file syscall when possible (drop entries git reports as mode 120000 via `ls-files --stage`, or stat lazily through the root `Dir` no-follow only for paths whose parent directory is a symlink candidate), and record the measured cost in S2. Never follow a symlink.
  - **Other roots** (Library, plain folders, non-git companions): a no-follow `Dir` walk shaped like `library/folder.rs:63-84` using the directory entry file type. It skips `reserved_context_path` and `excluded_source_path` (`context_assets.rs:1808-1818`). Measured: 64 ms for 50,000 files, 9.7 ms for the Library.
- Byte-wise sort. Cap **50,000 files** with `truncated: true`, keeping the sorted prefix.
- Rejected:
  - keeping the client BFS with more concurrency: still D authorizations;
  - serving the Library `inventory` from `index.json`: misses non-owned files and would be a second enumeration rule;
  - the `ignore` crate: new dependency, and git already does this for git roots;
  - applying `excluded_source_path` to git roots: it would hide tracked `build/` or `dist/` sources.

**D5. The client keeps a stale-while-revalidate file index across picker opens and pane remounts.**
- Module-level LRU of 8 entries, keyed by `binding_id + "\0" + root_id` (Library: `"library"`).
- On open, fire **both** requests at once: `mode: "cached"` (server memory or persisted list, D10) and `mode: "fresh"`. Paint the first answer that arrives (the in-memory LRU entry, if any, paints immediately). A `fresh` answer always replaces a `cached` one; a `cached` answer never replaces a `fresh` one. Skip the swap (no re-render) when the paths are identical.
- Status text is "Refreshing…" while only a non-fresh list is shown.
- An entry is dropped on identity change or an error response.
- The picker opens files with `revision: null`. The document read then validates freshness itself (`ContextViewer.tsx:918`), so a stale list can at worst produce the existing "file unavailable" error.

**D6 (numbers added). Prepared ranking; scoring semantics unchanged.**
- `prepareFileCandidates(candidates)` computes once per index: folded lower-case string, `nameStart`, and the original path.
- `rankPreparedFileMatches(query, prepared)` must return exactly what `rankFileMatches` returns today. `src/app/input/fileNavigation.test.ts` pins this.
- Compute `matchedIndices` only for the rows actually displayed (100); score everything else without allocation. Keep a slow path for paths whose folded string has a different length (non-BMP or case-expanding characters).
- `FilePicker` ranks `useDeferredValue(query)` and memoizes on the prepared array's identity. `ContextViewer` memoizes the candidate array.
- **Prototype, measured in Chromium (identical top-50 on 3 queries):** original 46-55 ms / 111-122 ms / 561-584 ms at 3k / 10k / 50k; prepared **0.7-2.2 / 1.7-2.7 / 8.5-9.4 ms**; prepare 3-18 ms once per list.
- Rejected: Web Worker, WASM fuzzy matchers. The prototype is already below one frame at 50k.

**D7. Library store read path.**
- (a) `recover()` lists the journal first and parses the index only if a `*.json` intent exists. `recover_attempts()` reads attempts first and parses the index only for a pending attempt whose lease it acquired. This reorders work without changing semantics.
- (b) `Store` caches `Arc<Index>`, keyed by `index.json` (dev, ino, len, mtime) and read under the caller's lock.
  - `commit` writes through `atomic_write_bytes`, which is a new inode every time (`store.rs:349-352, 1284-1295`), so other hosts' commits are detected.
  - Read paths get `Arc<Index>`. Mutators clone it before modifying.
  - Parse single-pass straight into `Index`. Probe `{schema}` only when that fails, to keep the `library_layout_outdated` error.
- (c) `listing`, `directory`, `document`, `media`, `space_listing` and the new file index run in `spawn_blocking`.
- Measured motivation: 7.0 ms per parse × 3 per page = 21 ms of a 51 ms page; directory 32 ms, and 8 concurrent readers gain nothing.
- Rejected:
  - an authoritative in-memory index: native and gateway hosts can share one Library root;
  - skipping recovery on reads;
  - a persisted derived index (D10).

**D8. Review snapshot reuse on identical tokens.**
- `ReviewService` gets an in-memory reuse map, bounded to `MAX_SNAPSHOTS` (8).
  - Key: (session_id, pane_id, binding_id, repository_id, source_id, comparison, base_revision, head, index, worktree).
  - Value: `review_id`.
- `snapshot()` authorizes as it does today, computes `before`, and computes `merge-base` for Branch. On a hit whose stored file still loads, it returns that `StoredSnapshot.snapshot` unchanged: same `review_id` and `generation`. It skips `collect` and the second token read.
- Equal tokens are the same criterion the existing before/after check uses to accept a snapshot (`review.rs:193`).
- Per-file parsed diffs then hit `load_file_cache`.
- Expected on `mid` **[derived from measured parts]**: one token read 74 ms + authorization ≈ 20 ms → ≈ 100-120 ms unchanged; changed ≈ 2 × 74 + 71 + parse ≈ 300-350 ms. Baseline 1,456 ms.
- Rejected:
  - caching raw `git diff` output keyed by oids: the parsed `ReviewFileDiff` cache already exists, and the first click's real work is ≈ 10 ms;
  - persisting the reuse map (D10).

**D9 (now required). Stream-hash revision tokens (S5).**
- Gate met: `review_snapshot` fails with `503 bounded_output` when `git diff --binary` > 1,048,576 B (measured at 1,057,693 B) or the index is > ≈ 15k files, for all comparisons.
- Replace buffering `ls-files --stage` / `diff --binary` under `git_output_bytes` with a bounded-time streaming SHA-256 of stdout.
- Token inputs and format are unchanged, so existing snapshots stay comparable.
- Open check for S5: `collect` (name-status, numstat, status) and the per-file `git diff` also run under caps; the acceptance run on `big`, `bigdiff` and the 1.06 MB `mid` copy shows whether any of them still fails and, if so, which code (the 11.2 MB `bigdiff` case may need a "diff too large" degraded state instead of a full snapshot; decide from the measurement).

**D10 (new). Persisted picker file-list cache.**

*What is persisted.* One entry per picker root that is a **git checkout or a plain folder/walk root**: the sorted path list (paths only; `bytes` are not stored and are `null` while a cached list is shown). Nothing else.

*What is not persisted, with the measured reason:*

| Candidate | Measured cost it would save | Verdict |
| --- | --- | --- |
| Repository scan (`RepositoryListResponse`) | 575 ms **once per gateway start**; refills otherwise hidden by SWR (D2) | **No.** Startup prewarm hides it (the UI connects > 1 s after boot). A persisted scan would also carry authorization-adjacent data across restarts. |
| Review snapshot / reuse map | ≈ 200 ms once per restart (≈ 300 ms cold vs ≈ 100 ms unchanged) | **No.** Parsed per-file diffs are already persisted per `review_id` under `<state_root>/review`. |
| Parsed per-file diff keyed by blob oids | ≈ 10 ms per first click (597 vs 587 ms) | **No.** |
| Library `index.json` (already persisted) and a derived binary index | 7.0 ms parse at 1.2k items; ≈ 120 ms at 20k **[INFERENCE]**, once per restart | **No.** S3's in-memory identity cache removes the repeated parses. |
| Library file list | 9.7 ms walk | **No.** Not worth a file. |
| Anything from Herdr | n/a | **Never.** |

*Location.* `<cache_root>/file-index/v1/<sha256(canonical root path)[..32]>.json`, with `cache_root` = `COCKPIT_CACHE_ROOT` > `cache_root` in the config file > `$XDG_CACHE_HOME/cockpit` (default `~/.cache/cockpit`), mirroring `state_root` (`config.rs:176-192`; `xdg_directory` already exists). The disposable fixtures set `XDG_CACHE_HOME=<root>/cache`, so measurements never touch the owner's cache. Directory mode 0700, files 0600, written with the existing `atomic_write_bytes` (temp file + rename, `project_store.rs:1244`).

*Format and version.* One JSON document:

```json
{ "schema": 1, "kind": "git" | "walk", "root": "<canonical path>", "root_dev": 0, "root_ino": 0,
  "created_unix": 0, "truncated": false, "count": 50000,
  "validators": { "head": "<oid>|null", "index": {"dev":0,"ino":0,"size":0,"mtime_ns":0}|null,
                  "ignore": [{"path":".gitignore","mtime_ns":0},{"path":"<common-dir>/info/exclude","mtime_ns":0}],
                  "root_mtime_ns": 0 },
  "files": ["path", …] }
```

- Measured with a Rust prototype (release): 50,000 paths of ≈ 115 B → **6.6 MB** (5.9 MB paths only), serialise 5.7 ms, write + rename 4.3 ms, read 1.1 ms, **parse 7.0 ms** (tmpfs); this repository's 2,969 paths → 370 KB, parse 0.4 ms. A NUL-separated plain list parses in 9.8 ms, no better, so JSON stays.
- The schema number is bumped on any change to the document shape or to the enumeration rules (for example a new exclusion). A different `schema` is a miss.

*Cache key.* `(canonical root path, root dev/ino)` selects the file; a mismatch of `root`, `root_dev` or `root_ino` is a miss. The **validators** do not select the entry; they describe how trustworthy it is:

| Root kind | Validators | Cost to compute |
| --- | --- | --- |
| Git checkout | `HEAD` oid (`rev-parse HEAD`, 1.7-2.3 ms), `$GIT_DIR/index` stat via `git rev-parse --git-path index` (dev, ino, size, mtime_ns), mtime of the root `.gitignore` and of `<common-dir>/info/exclude`, root dir mtime | ≈ 2-3 ms, one spawn |
| Non-git dir / walk root | root dir mtime, plus the mtime of every directory in the first two levels (≤ ≈ 500 `lstat`s); a full all-directory fingerprint costs 11.5 ms at 5,051 dirs | ≤ 12 ms |

- All validators equal → the list is **probably current**: shown without the "Refreshing…" state.
- Any validator differs → shown as **stale** ("Updating…").
- Validators can never prove the list current. Untracked additions in subdirectories change neither the index nor the root mtime, and content edits to `.gitignore` files below the root change no listed stat. So **revalidation by real enumeration always runs**, and the validators only choose the label.

*Load: lazy on first use.* Nothing is read at startup except a background sweep of the cache directory (see eviction; ≤ 33 directory entries). A `"cached"` request loads the file (1 ms read + 7 ms parse at 50k), keeps up to 8 parsed entries in memory (`Arc<[String]>`), and serves them. Rejected: loading every entry at startup (up to 64 MiB parsed for lists nobody opens).

*Stale-while-revalidate.*
1. The client sends `mode: "cached"` and `mode: "fresh"` together (D5).
2. `cached` returns memory or disk state for the authorized root, or `state: "miss"`.
3. `fresh` enumerates (D4), returns, and then in `spawn_blocking` compares with the loaded list. It writes the file only if the paths differ, the entry is missing or older than 24 h, or the validators changed. Write failures are logged, never surfaced.
4. The client swaps only when the fresh paths differ (D5).

*Correctness bounds.*
- **How long a stale list can be shown:** from picker open until the `fresh` response (measured server cost 10-86 ms warm plus transfer), and, if `fresh` fails or times out (10 s), until the user closes the picker, with a persistent "may be out of date" label. **An entry older than 7 days, or whose root no longer canonicalises to the same path and (dev, ino), is discarded, never served.**
- The list is a name hint only. It is **never** used for: opening a file (`revision: null` + the fail-closed document read, D5), Review, comment capture or delivery, setup, teardown, deletion, companion operations, or any authorization decision.
- It is served only to a request that passes the same authorization as `directory` for the same canonical root, so one pane cannot read another root's names.
- The file holds relative paths only. It sits under the user's cache directory with 0600/0700; it discloses file names of the owner's checkouts to other processes of the same user, which is the same exposure as the checkouts themselves.
- No Herdr state, session id, pane id or binding id is stored.

*Size cap and eviction.* At most **32 entries and 64 MiB** in `file-index/v1`, at most **8 MiB per entry** (50,000 × ≤ 168 B; a longer list is not persisted, but still served fresh). After each write, and in a background sweep at gateway start, delete entries older than 7 days, then oldest-first by mtime until under both caps. Reading an entry `touch`es it (at most once per hour) so eviction is LRU. Unknown files in the directory are left alone.

*Corruption and version mismatch.* Any of: unreadable file, JSON parse error, `schema` ≠ current, `count` ≠ `files.len()`, `root` ≠ requested canonical root, `kind` ≠ detected kind, size > 8 MiB → **delete the file, treat as a miss, rebuild through the normal fresh path**. The user sees nothing but a slower first paint. A partial write cannot be observed because of temp file + rename. Two gateways (native and web) sharing one cache directory are safe: last complete rename wins.

*Expected gain, measured parts and derived parts.*

| Case | Without persistence (after S1+S2) | With persistence | Basis |
| --- | --- | --- | --- |
| Server enumeration, git, 50,000 files (warm) | 86 ms | ≈ 8 ms (1 read + 7 parse) → **≈ −78 ms** | measured |
| Server enumeration, this repository (2,975 paths) | 10.3 ms | ≈ 0.5 ms → **≈ −10 ms** | measured |
| Server enumeration, plain walk, 50,000 files | 64 ms | ≈ 8 ms → **≈ −56 ms** | measured |
| First open after gateway restart / browser reload, 50k git | authorization (prewarmed) + 86 ms enumerate + ≈ 100-200 ms transfer, parse and prepare ≈ 0.3-0.7 s **[INFERENCE for the transfer/parse part]** | first paint from the persisted list ≈ 0.1-0.3 s **[INFERENCE]**, then swap when fresh lands | measured server side, inferred client side |
| Cold disk (uncached inodes), 55k files | ≈ 55k inode lookups: **unmeasured** | one sequential 6.6 MB read | **[INFERENCE]**, `/tmp` is tmpfs and `/home` was fully cached |
| Versus today's picker (client BFS) | 32.3 s (`mid`), 53.6 s and 10k of 55k files (`big`) | ≥ 99.5 % lower either way | measured |

Honest reading: **the persisted list on its own removes ≈ 80 ms of a ≈ 0.5 s first open on warm caches.** The 32-54 s becomes ≈ 0.5 s through S1+S2. Persistence is kept because the user decided it is in scope, because it is the only layer that survives restart/reload, and because the cold-disk case is where it can matter; S6 measures that case (`drop_caches` or a first-run-after-reboot) and reports the result.

## 6. Open questions (user choice)

Resolved:
1. **Picker lists gitignored files in git checkouts?** **No** (user decision, recorded in D4 and §9). The tree still shows them.
2. **Persist any file index across restarts?** **Yes** (user decision), designed as D10 and sequenced as S6 after S1-S5.

Still open:
1. **Cold-disk gain is unmeasured.** Measuring it needs root (`drop_caches`) or a first run after reboot. Should S6 wait for that measurement, or ship and report?
2. **Retention:** is 7 days / 32 entries / 64 MiB right for a machine with ~54 checkouts? The measured largest real repo (`lilygo-t3`, 8,358 paths ≈ 1 MB) means 64 MiB is far above the real need; a 16 MiB cap would also do.
3. **Review > 1 MiB.** After S5 streams the tokens, should a diff that is too large for the full snapshot (`bigdiff`, 11 MB) degrade to a "diff too large, showing the file list" state, or must every diff open? Decide from the S5 acceptance run.
4. **Idle polling:** after S1 the polls cost ≈ 1 scan refill per ≥ 30 s. Should the frontend also stop the presentation poll for panes that are not visible? Not measured, not planned.

## 7. Targets and how to re-measure

Fixture: the S0 fixture in the baseline file (R=72 checkouts, `big`, `mid`, `bigdiff`, 1,200-page Library). Recipes in `perf/measurement-recipes.md`. Report p50/p95 of 20 runs, release binary.

| Metric | Baseline (measured) | Target | Method |
| --- | --- | --- | --- |
| `GET …/presentation` (Files pane), warm | **575 ms** p50, p95 598 | ≤ 30 ms p50, ≤ 50 ms p95; no request pays a scan except the very first after a cold start (prewarmed) | R2 |
| `POST …/context/directory` (`big` `src/s00`), warm | **580 ms** p50, p95 599 | ≤ 30 ms p50; ≤ 50 ms p95, including the first request after the 30 s freshness expiry (stale served, D2) | R2 |
| Git spawns / gateway CPU over 60 s idle, Files + Review + terminal visible | **12,577 spawns; ≈ 55 % of a core** (git 46 %, gateway 8 %) | ≤ 700 spawns (≤ 2 refills × 288 + Review discover; −94 %); ≤ 3 % of a core; 0 with no UI | R1, R1b |
| Git spawns for one Context request, warm | **293** (4R + 5) | ≤ 5 (viewer root only) | R1 |
| Picker on `mid` (2,969 files): open → full list | **32.3 s**; reopen 32.2 s | ≤ 400 ms cold; ≤ 50 ms to first paint on reopen (client SWR) | R4 |
| Picker on `big`: open → full list | **53.6 s, 10,000 of 55,001 files, "index incomplete"** | complete list of the 50,000 tracked files (`truncated` only at the 50k cap) in ≤ 700 ms cold; ≤ 50 ms first paint on reopen | R4 |
| Picker after gateway restart / browser reload (S6) | same as the cold rows | first paint ≤ 150 ms from the persisted list on `big` (50k), then swap | R4 + restart |
| Picker on the Library (1,201 files): open → full list | **43.8 s** | ≤ 300 ms cold | R4 |
| Picker keystroke, 10k candidates | **134-176 ms** script/key (p50, 5-6 keys) | ≤ 8 ms script/key p95 | R5 |
| Picker keystroke, 50k candidates | **≈ 570 ms** ranking alone (function, V8) | ≤ 20 ms script+layout/key p95 | R5 |
| Library listing, 1,200 items (all pages) | **261 ms** p50 | ≤ 60 ms | R3 |
| Library directory request p50 | **32 ms** | ≤ 10 ms; 8 concurrent readers ≤ 2× the single-request time | R3 |
| Review AllLocal refresh (`mid`, 185 files, 507 KB), changed tree | **1,456 ms** p50, **603 spawns** | ≤ 350 ms, ≤ 30 spawns | R1 + R2 |
| Review AllLocal refresh, unchanged tree, returning to tab | 1,456 ms, 603 spawns | ≤ 120 ms, ≤ 8 spawns, 0 `git diff --unified` for files already opened | R1 + R2 |
| Review file click, cached and first | **587 / 597 ms** | ≤ 30 ms both | R2 |
| Review snapshot with `git diff --binary` > 1 MiB (1.06 MB copy of `mid`), `big`, `bigdiff` | **503 `bounded_output`** | succeeds (or degrades as decided in open question 3); record the time | R2 |
| `ReviewFileTree` duplicates, 2,000 files | **65 ms/render** | ≤ 2 ms | browser eval |
| Space git status | 17 ms, 7 spawns | unchanged | R2 |

## 8. Slices

Dependencies: **S0 (done) → {S1 ∥ S3 ∥ S5} → S2 (after S1; shares `context.rs`) → S4 (after S1 and S5; both edit `review.rs`) → S6 (after S2) → S7**. S1, S3 and S5 touch disjoint files.

### S0 — Baseline (measurement only) — DONE 2026-09-28

- Result: [`perf/baseline-2026-09-28.md`](perf/baseline-2026-09-28.md). Recipes were corrected while running (see the changes list at the top of `perf/measurement-recipes.md`).
- S5 gate: **decided yes** (503 above 1,048,576 B of diff; 3.5 MB stage list).
- Fixture data kept for re-measurement: `/tmp/cockpit-perf-repos` (72 checkouts), `/tmp/cockpit-perf-plain`, helper scripts `/tmp/cockpit-perf-*` and `/tmp/cockpit_perf_lib.py`. The fixture root itself was stopped; re-create it with `ui_polish_runtime.py start --with-plugins` and `/tmp/cockpit-perf-fixture-cockpit.toml`.

### S1 — Repository discovery cache + Files-viewer skip + prewarm (D2, D3)

- **Goal:** Context/Review read paths stop paying B1 per request; idle stops burning a core.
- **Files and symbols, exclusively:**
  - new `crates/cockpit-core/src/repository_cache.rs` (`RepositoryDiscoveryCache`);
  - `crates/cockpit-core/src/lib.rs` (module declaration);
  - `crates/cockpit-core/src/project_store.rs` (`ProjectStore` gains `mutation_generation: Arc<AtomicU64>`, bumped in `update`, `write_companion`, `reattach_companion`, `remove_owned_companion`; `pub(crate) fn mutation_generation()`);
  - `crates/cockpit-core/src/projects.rs` (`ProjectService` owns the cache; `repositories()` publishes; new `pub(crate) async fn cached_repositories()` and `cached_discover_checkout(&Path)`; `pub fn prewarm_repositories()`);
  - `crates/cockpit-core/src/context.rs` (`authorized_roots` reorder + cached calls; `review_checkout_for_evidence` line 449);
  - the gateway/native start-up call site that builds `ProjectService` (one `tokio::spawn(prewarm)`).
- **Interface, fixed:**
  ```rust
  pub(crate) struct RepositoryDiscoveryCache { /* Mutex<State>, refill: tokio::sync::Mutex<()> */ }
  impl RepositoryDiscoveryCache {
      pub(crate) fn new(fresh: Duration, stale_max: Duration) -> Self;   // 30 s, 5 min
      pub(crate) async fn list(&self, catalog: &RepositoryCatalog, generation: u64) -> Result<RepositoryListResponse, InspectionError>;
      pub(crate) async fn discover(&self, catalog: &RepositoryCatalog, cwd: &Path, generation: u64) -> Result<RepositoryCandidate, InspectionError>;
      pub(crate) fn publish(&self, listed: RepositoryListResponse, generation: u64);
  }
  ```
  - Errors are not cached.
  - Discover entries are keyed by canonical cwd + (dev, ino). A cwd that is not a directory is a miss, and a miss is resolved fresh.
  - `list` follows the fresh / stale-while-refilling / must-wait rules of D2. A refill runs only after a request and is single-flight.
- **Steps:**
  1. Add the counter to `ProjectStore`.
  2. Add the cache module.
  3. Wire it into `ProjectService`, and prewarm at startup.
  4. In `authorized_roots`, compute `companion_roots` → `viewer_root` / `viewer_companion` / `viewer_folder` first. Call `cached_repositories` and `cached_discover_checkout(cwd)` only when `viewer_folder.is_none()`. Otherwise keep `diagnostics` empty of catalog output and `current_repository_id = None`.
  5. Switch `context.rs:449` to the cached discover.
  6. Update existing tests that pin Files-pane catalog diagnostics, if any. Add core tests: two `inspect_pane` calls with a counting `git` on `PATH` spawn catalog git once; a `ProjectStore::update` in between spawns it again; a request after the freshness window returns the stale value without waiting and triggers exactly one refill.
- **Non-goals:** changing setup/teardown freshness, Herdr inspect, or `context_companions`; reducing the 4 spawns per checkout.
- **Acceptance:**
  - `cargo test -p cockpit-core context repositories projects` passes.
  - R2: presentation and directory p50 ≤ 30 ms.
  - R1/R1b: idle 60 s ≤ 700 spawns and ≤ 3 % of a core; 293 → ≤ 5 spawns per request.
  - One independent review round on authorization equivalence (high severity only).

### S2 — Server file index + client SWR + prepared ranking (D4, D5, D6)

- **Goal:** one request per picker open, a complete git-aware list, instant reopen, smooth typing.
- **Files and symbols, exclusively:**
  - `crates/cockpit-protocol/src/context.rs` (new DTOs, including `mode` and `state`); `crates/cockpit-protocol/src/library.rs` (`LibraryFileIndexRequest`); the TypeScript exporter list in `crates/cockpit-protocol/src/typescript.rs`; regenerated `src/protocol/generated/v1.ts`.
  - New `crates/cockpit-core/src/context_file_index.rs`, containing `pub(crate) fn enumerate_files(root: &AuthorizedRoot, git_root: bool, limits) -> Result<ContextFileIndex, InspectionError>` and the git variant.
  - `context.rs`: `impl ContextService { pub async fn file_index(&self, session, pane, &ContextFileIndexRequest) }`. It authorizes like `directory` (`context.rs:237-249`) and decides `git_root` with one `git rev-parse --show-toplevel` equality check. `mode: "cached"` answers `state: "miss"` until S6 lands.
  - `crates/cockpit-core/src/library/reader.rs`: `pub async fn file_index(&self)`.
  - Host routes `POST /api/v1/sessions/{session_id}/panes/{pane_id}/context/files` (`crates/cockpit-host/src/server/context.rs`) and `POST /api/v1/library/files` (`server/library.rs`).
  - Tauri commands `cockpit_context_file_index` and `cockpit_library_file_index` (`src-tauri/src/context.rs`, `src-tauri/src/library.rs`, registered in `src-tauri/src/lib.rs:1767-1786`).
  - Client: `src/client/CockpitClient.ts`, `browser.ts`, `native.ts` (plus the parse/match helpers, following `contextDirectory` / `libraryDirectory`).
  - UI:
    - `ContextViewer.tsx`: the `ContextReader` type gains `fileIndex`; `openFilePicker` / `closeFilePicker` / `pickerIndex` / the `<FilePicker>` props; delete `MAX_PICKER_DIRECTORIES`, `PICKER_DIRECTORY_CONCURRENCY` and `MAX_PICKER_FILES`.
    - New `src/app/input/fileIndexCache.ts` (module LRU).
    - `src/app/input/fileNavigation.ts`: `prepareFileCandidates` and `rankPreparedFileMatches`. `rankFileMatches` becomes a thin wrapper, or is removed if nothing else calls it.
    - `FilePicker.tsx`: accepts a prepared list; uses `useDeferredValue`.
- **Interface, fixed:**
  ```ts
  type ContextFileIndex = { binding_id: string; root_id: string; files: { path: string; bytes: number | null }[]; truncated: boolean; source: "git" | "walk"; state: "fresh" | "cached" | "miss"; diagnostics: ProjectDiagnostic[] };
  interface ContextReader { /* existing */ fileIndex(rootId: string, mode: "cached" | "fresh", signal: AbortSignal): Promise<ContextFileIndex>; }
  ```
- **Steps:**
  1. DTOs + exporter + regenerated TypeScript.
  2. Core enumerate: git and walk variants, both `spawn_blocking`, sorted, 50k cap, gitignored files excluded in git roots, symlinks never followed. Measure the type-check cost on `big` and keep it ≤ 30 ms.
  3. Core service methods (context + library).
  4. Host + Tauri + client.
  5. `ContextViewer` picker: fire `cached` and `fresh` together, SWR from `fileIndexCache`; candidate `id = path`; choose → `openFile(path, null)`; status text "Refreshing…" while a non-fresh list is shown.
  6. Prepared ranking + deferred query; identical scoring, visible-rows-only `matchedIndices`, slow path for non-ASCII or case-expanding paths.
  7. Core test: a git root lists tracked + untracked, not ignored, not `.git`, no symlinks. A walk root skips `.cockpit` / `node_modules`. The cap sets `truncated`.
  8. Existing `fileNavigation.test.ts` ranking expectations still pass unchanged; add a test comparing prepared and original ranking on a mixed ASCII / non-ASCII / uppercase path set.
- **Non-goals:** the Review picker (its candidates are already local); the tree/directory endpoints; Ctrl+P key handling (owned by `02-keyboard-shortcuts`, which edits `ContextViewer.tsx:1636-1660` only); persistence (S6).
- **Acceptance:**
  - R4 and R5 targets met on `mid`, `big` and the Library.
  - A browser smoke on the disposable fixture: open picker → type `sd deploy` → Enter opens the file; close and reopen shows the list instantly.
  - `bun run test -- src/app/input src/app/context`.
  - `cargo test -p cockpit-core context_file_index`.

### S3 — Library store read path (D7). Parallel with S1

- **Files and symbols, exclusively:**
  - `crates/cockpit-core/src/library/store.rs`: `recover`, `index`, new `index_shared() -> Arc<Index>`, `Store` gains `index_cache: Mutex<Option<(IndexIdentity, Arc<Index>)>>`;
  - `library/space.rs`: `recover_attempts`, and `space_listing` read path;
  - `crates/cockpit-core/src/library.rs`: `listing`;
  - `library/reader.rs`: `directory`, `document`, `media` bodies moved into `spawn_blocking`.
- **Steps:**
  1. Journal-first `recover` and attempts-first `recover_attempts`, so the index is parsed only when needed.
  2. Identity-keyed `Arc<Index>` cache with a single-pass parse and schema-1 detection on failure.
  3. Read paths use `index_shared`. Mutators keep an owned `index()`, which clones the cached `Arc`.
  4. `spawn_blocking` for the read handlers.
  5. Tests:
     - a commit by a second `Store` instance on the same root is visible to the first on its next read (identity change);
     - an interrupted journal intent is still recovered on the next read;
     - the `library_layout_outdated` error is preserved (existing test at `store.rs:2346-2350`).
- **Non-goals:** listing page size, follow/refresh performance, the tree UI.
- **Acceptance:**
  - `cargo test -p cockpit-core library` passes.
  - R3 targets met (listing ≤ 60 ms, directory ≤ 10 ms, 8 concurrent readers ≤ 2× single).
  - One independent review round on recovery ordering and cross-host visibility (data-loss area).

### S5 — Streaming revision tokens (D9). Required; parallel with S1

- **Files:**
  - `crates/cockpit-core/src/process.rs`: add a `run_streaming_hash_command` next to `run_bounded_command` (`process.rs:196`), with the same timeout, process group and stderr cap;
  - `review.rs`: `revision_tokens` only (S4 later edits `snapshot` in the same file, so S5 lands first).
- **Acceptance:**
  - The token strings for a fixture are byte-identical before and after the change. Record both.
  - On `mid` copy (1.06 MB diff), `big` (3.5 MB stage list) and `bigdiff` (11.2 MB diff) the snapshot returns 200, or fails with a **different** code from a later stage; record each result and time, and decide open question 3 from them.
  - `cargo test -p cockpit-core review`.

### S4 — Review reuse + O(n) duplicates (D8). After S1 and S5

- **Files and symbols, exclusively:**
  - `crates/cockpit-core/src/review.rs`: `ReviewService` gains `reuse: Arc<Mutex<VecDeque<(ReuseKey, String)>>>`; `snapshot` gains a reuse lookup after `before`; `save_snapshot` records the key; `discover_checkout` switches to the S1 cached discover via `self.context` → projects;
  - `src/app/review/ReviewPane.tsx:99-104`: `duplicates` via a single `Map<string, number>` inside `useMemo([files])`.
- **Steps:**
  1. Reuse key, lookup and record.
  2. On a hit, `load_snapshot(review_id)`. If it has been pruned (`None`), fall through to the full path.
  3. Branch comparisons compute `merge-base` before lookup and put it in the key.
  4. Core test with a real git fixture:
     - refresh twice with no change → same `review_id` and no second `collect` (count via the injectable git path or by asserting unchanged `created_at`);
     - edit a tracked file → new `review_id`;
     - touch an untracked file → new `review_id`.
- **Non-goals:** token semantics, frontend refresh triggers, comment anchoring.
- **Acceptance:**
  - `cargo test -p cockpit-core review` passes.
  - R1/R2 Review rows meet their targets (≤ 350 ms changed, ≤ 120 ms unchanged; file click ≤ 30 ms).
  - Browser smoke: switch away from and back to a Review tab, and the selected diff shows without a spinner.

### S6 — Persisted picker file-list cache (D10). After S2

- **Goal:** the picker's first paint after a gateway restart or browser reload comes from disk, is revalidated by the normal fresh enumeration, and can never be trusted for anything but names.
- **Files and symbols, exclusively:**
  - `crates/cockpit-core/src/config.rs`: `cache_root` (`COCKPIT_CACHE_ROOT` > config file `cache_root` > `$XDG_CACHE_HOME/cockpit`), origin map entry, `validate_paths`, and the same overlap check with `state_root`, `companion_root`, `worktree_root`, `library_root` that those already get;
  - new `crates/cockpit-core/src/file_index_cache.rs`: `FileIndexCache { load(root_key) -> Option<CachedIndex>, store(&Enumerated, &Validators), sweep() }`, schema constants, the 8-entry in-memory layer, the caps and the corruption rules;
  - `context_file_index.rs` (from S2): compute validators, answer `mode: "cached"`, and write after `fresh` per D10 step 3;
  - `crates/cockpit-core/src/lib.rs` (module) and the start-up call site (one background `sweep()`).
- **Steps:**
  1. Config `cache_root`, defaults, tests in `config.rs` following the existing path tests.
  2. Cache module: JSON schema 1, 0700/0600, `atomic_write_bytes`, validators, load/store/sweep/eviction, deletion of corrupt or mismatching entries.
  3. Wire `mode: "cached"` and the post-fresh write.
  4. Client: no new code beyond S2's paint-first-arrival rule; add the "may be out of date" label for a failed fresh.
  5. Core tests:
     - round trip and corruption: truncated file, wrong `schema`, wrong `count`, wrong `root`, oversize → each is deleted and returns a miss;
     - eviction: with caps lowered, the oldest entries go first, and entries > 7 days old are never served;
     - the served entry is refused for a root with a different (dev, ino);
     - the write is skipped when nothing changed.
  6. Measure, with the S0 fixture and a gateway restart between runs (`XDG_CACHE_HOME` is inside the fixture root): first open after restart with and without the cache on `big` and `plain`, and record enumeration ms and time to first paint. Then measure cold: `echo 3 | sudo tee /proc/sys/vm/drop_caches` (needs root) or the first run after a reboot, on the same repositories and on the real cockpit checkout (read-only).
- **Non-goals:** persisting the repository scan, Review data, the Library list or index, or any Herdr state (D10 table); an fs watcher; loading entries at startup.
- **Acceptance:**
  - `cargo test -p cockpit-core file_index_cache config` passes.
  - After a gateway restart on `big`, the first paint comes from the persisted list in ≤ 150 ms, is replaced by the fresh list, and a file that no longer exists opens as the existing "file unavailable" error.
  - The cold and warm numbers are recorded in `perf/` next to the S7 results, including the case where persistence gains little; the code stays (user decision), the measurement decides only how the feature is described.
  - One independent review round on "list never used for authorization or writes" and on the cache-directory permissions (high severity only).

### S7 — Re-measure, docs, decisions

- **Steps:**
  1. Rerun R0–R6 and write `perf/after-<date>.md` with the "After" column for every §7 row, next to `baseline-2026-09-28.md`.
  2. Add the proposed entries (§9) to `DECISIONS.md` in "Context & Review".
  3. Add `CODE_GUIDE.md` table rows: "Repository discovery caching | `crates/cockpit-core/src/repository_cache.rs`, `projects.rs` | cached-vs-fresh consumer list" and "Picker file index and persisted cache | `crates/cockpit-core/src/context_file_index.rs`, `file_index_cache.rs` | cache_root, schema, eviction".
- **Acceptance:** every §7 target is met, or the miss is recorded with its number and a follow-up decision.

## 9. Proposed DECISIONS.md entries

- Repository discovery for **read-only** Context/Review authorization may be served from an in-memory cache.
  - The cache is fresh for 30 s, may be served stale for up to 5 min while one background refill runs, and is invalidated by any Cockpit project-store mutation. It is prewarmed at startup and refilled only when a request asks.
  - Setup, plan, resume and teardown always rediscover.
  - Herdr pane/process evidence and root identity are never cached: they are read fresh on every request.
- Files viewers do not run repository discovery.
- The file picker gets its candidates from one server-enumerated index per open.
  - Git checkouts list tracked and untracked-not-ignored files (**gitignored files are excluded from the picker; the tree still shows them**); other roots use a no-follow walk with fixed exclusions; the list is capped at 50,000 files.
  - The client may show the previous list while it refreshes, and opens picked files without an expected revision.
- The picker's file list may be persisted under `cache_root` (`$XDG_CACHE_HOME/cockpit`) as a **hint**.
  - It is keyed by canonical root path and identity, versioned, size-capped (32 entries / 64 MiB, 7 days), served stale, and always revalidated by a fresh enumeration on every open.
  - It is never used for authorization, file opens, Review, comments, setup, teardown or deletion, and a corrupt, mismatched or over-age entry is discarded and rebuilt.
  - Repository scans, Review data, the Library index and any Herdr state are not persisted.
- The Library store may cache its parsed index only by `index.json` file identity, read under the Library lock. Recovery still runs before every Library read, but parses the index only when a journal intent or pending attempt exists.
- A Review refresh whose head/index/worktree tokens (and base, for branch) equal a retained snapshot's returns that snapshot unchanged, with the same `review_id` and generation. Revision tokens are hashed from a stream, so Review opens on repositories and diffs larger than the 1 MiB command-output cap.

## 10. Risks and verification

| Risk | Where | Guard |
| --- | --- | --- |
| Stale repository list admits a root that should not be admitted | S1 | Only read paths use the cache. Every admitted root is still opened and identity-checked per request. Mutating setup/teardown stays fresh. Store-generation invalidation, 30 s fresh window, 5 min stale cap. One review round. |
| Stale-while-refilling omits a checkout created outside Cockpit for ≤ 5 min | S1 | Only the other-repositories root list is affected; a pane's own cwd resolves through a fresh per-cwd discover; the refill lands in ≈ 0.6 s. |
| Behavior drift: Files panes lose catalog diagnostics | S1 | Intended (D3). Update the pinned tests and state it in the S1 commit message. |
| Picker lists a file that vanished (SWR, persisted) | S2, S6 | Open with `revision: null`. The existing document read fails closed and shows an inline error. |
| Git enumeration escapes the root through symlinks or `..` | S2 | Validate paths with `safe_source_relative`-style checks (`context_assets.rs:1036`); never follow symlinks; regular files only; the type check is measured (≤ 30 ms on `big`). |
| Prepared ranking changes result order for unusual paths | S2 | Identical-scoring requirement, the existing `fileNavigation.test.ts`, and a new mixed ASCII / non-ASCII / uppercase equality test. |
| Persisted list treated as authoritative | S6 | D10: hint only, revalidated on every open, served only after normal authorization of the same canonical root and (dev, ino), never used by any write path. One review round. |
| Cache directory leaks file names or grows without bound | S6 | 0700/0600, relative paths only, 32 entries / 64 MiB / 8 MiB per entry / 7 days, sweep at start and after each write. |
| Corrupt or half-written cache entry | S6 | Temp file + rename; any parse, count, schema, root or kind mismatch deletes the entry and falls back to a miss. |
| Persisted gain is smaller than the effort | S6 | Measured: ≈ 80 ms warm at 50k; cold-disk gain is unmeasured and is what S6 step 6 measures. Recorded honestly; the user decided the feature is in scope. |
| Library cache returns an old index after another host commits | S3 | Identity (dev, ino, len, mtime) is read under the lock on every call. Atomic rename gives a new inode. Covered by a two-`Store` test. |
| Reordered recovery skips a needed recovery | S3 | Same work whenever a journal intent or pending attempt exists. The existing crash-recovery tests plus a new next-read recovery test. One review round. |
| Reused `review_id` confuses comment anchors or scroll state | S4 | Identical tokens mean identical content, which is the criterion the existing before/after check uses. The frontend already treats the same `review_id` + `generation` as the same review. |
| Streaming hash changes the token format | S5 | Byte-identical token strings before and after, recorded on a fixture. |
| A cache hides Herdr changes | all | Nothing Herdr-derived is cached. The discovery cache holds filesystem/git metadata of configured roots only; the file list holds relative names only. |
| Mid-flight edits collide with sibling specs and running implementers | S2 | Neither sibling touches `openFilePicker`, `pickerIndex` or `FilePicker` ranking, so either may land first. `02-keyboard-shortcuts` edits only `ContextViewer.tsx:1636-1660` / `ReviewPane.tsx:499-505`. `03-library-and-top-bar` edits `ContextViewer.tsx` toolbar ~1643-1660, documentDetails ~1557-1575, empty-state ~1535/~1694 and `useTreeWidth` ~684. Re-read the files before each edit; the current UI implementers (shortcuts, sidebar, Library) own their files. |
| Measurements touch the user's session | S0/S6/S7 | Disposable fixture only (`ui_polish_runtime.py`). Git shim on the fixture gateway's `PATH` only. Scripts live in `/tmp`. Read-only git commands only against the owner's checkouts. Stop everything recorded; `plugins.json` mtime unchanged (1790518335 before and after S0). |
