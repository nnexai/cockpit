# Reference workflow handoff

## Resume here

The user requested a pause at the next good handoff point, followed by this document. Implementation is paused. Resume orchestration only when instructed.

The unfinished todo backlog is blocked awaiting that authorization, not marked complete. On resume, unblock the selected work, starting with REF-01; retain the separate upstream mouse and user-excluded terminal-matrix blockers.

The checkpoint has integrated REF-01 source, passing focused static/test gates, and a reproduced-and-repaired linked-worktree authorization defect. **REF-01 is not complete and has no commit. No real graphical comment mutation or payload preview has been exercised.** The last verified feature commit remains `2827b40`, graphical Context.

All writing/review workers have returned. The reference browser, gateway, Herdr server and native display were stopped. No reference native application was launched. The working tree is intentionally uncommitted and unstaged; do not reset it or mistake worker completion for acceptance.

## First actions after authorization to resume

1. Read `.omp/AGENTS.md`, `skill://incremental-delivery`, `skill://cockpit-ui-parity`, `CONTEXT.md`, `DECISIONS.md`, the UI research authorities, and `planning/next-level/04-viewer-and-reference-comments.md`. The active implementation is REF-01, not the earliest open phase displayed by the todo UI.
2. Inspect current changes and this checkpoint's evidence. Preserve pre-existing execution bookkeeping. Review the remaining boundary concerns below before claiming the integration is ready.
3. Allocate a fresh, short-path disposable Herdr scenario and ledger. Use the installed stock Homebrew binary. Provision a companion through real Cockpit setup, create a source terminal at that exact companion directory, and launch its real file-viewer split through Open Context. Do not inject into default Herdr or the manual gateway at port 4173.
4. Complete every REF-01 runtime acceptance item below in the browser and real Tauri application. Repair failures, rerun affected gates, and obtain a final integrated review.
5. Update feature documentation, format only owned changes, stage only this increment, and commit after acceptance. Mark REF-01 complete only then. Continue the selected reference, source and review plan in bounded increments; do not silently waive earlier repair or release gates.

## What is implemented but awaiting runtime acceptance

### Core and persistence

- `crates/cockpit-core/src/comments/mod.rs`: six operations, fresh Context companion evidence, batch ownership/location checks, source capture, text-only editing, deletion, explicit reattachment, and preview.
- `comments/store.rs`: state-root `comments/` collection, descriptor-relative no-follow records, process-shared locks, atomic replacement, CAS generations, bounded scans and records. Persisted attachments are cleared. Loaded records are detached until fresh evidence permits attachment.
- `comments/format.rs`: deterministic path/range/draft-ID ordering, quote-escaped paths, linear terminal-control sanitization, per-line `crlf`/`lf`/`none` metadata, exact UTF-8 and 12-byte framing counts. Preview ordering uses references rather than cloning excerpts.
- `context.rs`: internal comment evidence and typed `context_file_missing` errors. Missing files no longer depend on localized error-message matching.
- `project_store.rs`: shared state descriptor, lock, bounded-reader and atomic-write helpers. Main repaired `LockGuard` visibility for sibling-module use.
- `projects.rs`: companion authorization now queries inventory through `operation.plan.repository.checkout_path`, the recorded primary source checkout, then separately verifies the exact linked checkout/workspace. See the reproduced defect below.
- `repositories.rs`: its existing test configuration was missing the four Context limit fields added by the previous increment. Main filled them so core tests compile.

Bounds currently include 64 drafts per batch, 8 KiB comment prose, 4 MiB serialized records, 256 batches, 4096 scanned entries, and a 64 KiB preview/export ceiling including framing. Sources are captured server-side from the expected Context revision. Whole-file references contain no source excerpt. Existing line excerpts are never rebased or replaced by fresh text.

Root incarnation matters in addition to companion UUID. Automatic attachment and mutations check captured draft root IDs against fresh evidence. A same-owner batch whose root was replaced is returned detached instead of silently creating a new batch. Explicit attach also rejects a different root incarnation. Empty batches have no captured root to conflict with.

### Protocol and transports

Frozen DTO authority: `crates/cockpit-protocol/src/comments.rs`, exported by protocol `lib.rs`/`typescript.rs` and generated into `src/protocol/generated/v1.ts`.

| Client method | Core method | HTTP POST suffix | Native command |
| --- | --- | --- | --- |
| `commentBatches` | `list` | `list` | `cockpit_comments_list` |
| `commentBatch` | `batch` | `batch` | `cockpit_comments_batch` |
| `commentUpsert` | `upsert` | `upsert` | `cockpit_comments_upsert` |
| `commentRemove` | `remove` | `remove` | `cockpit_comments_remove` |
| `commentAttach` | `attach` | `attach` | `cockpit_comments_attach` |
| `commentPreview` | `preview` | `preview` | `cockpit_comments_preview` |

HTTP prefix: `/api/v1/sessions/{session_id}/panes/{pane_id}/comments/`. Native arguments follow existing Context conventions: `sessionId`, `paneId`, `request`. Browser/native adapters use shared `src/client/commentProtocol.ts` validation and identity checks. Mutation responses require the next generation and a live attachment; reads may return detached batches. Preview matches batch ID and requested generation.

`CockpitService` now has optional CommentsService composition. Both hosts construct it from the project configuration and shared ContextService. HTTP adapters use existing origin, body-bound and identifier checks. `stale_generation` maps to HTTP 409. Tauri commands, build registration and default capabilities are wired. Permission/schema files under ignored generated paths are build output, not a reason to force-add them.

Main fixed compiler-visible omissions in worker output: the service adapter initializer, imports, lock-guard visibility, preview byte-count local, and NUL rejection in identity parsing. TypeScript LSP repeatedly claimed generated comment types were missing; actual `bun run typecheck` passed. Rust LSP references requests exited with code 0; explicit caller searches were used as fallback.

### UI

- `ContextViewer.tsx`, `CommentDrafts.tsx`, `comments.css`: whole-file/range editor, count and on-demand overview, inline and file-bottom drafts, edit/delete, same-source recovery, explicit attach, exact preview and stale-excerpt acknowledgement.
- Comment controls stay mounted while files load, fail, are refused, or have no selection.
- Range cards render once at their start line. Markdown shows current ranges below the file instead of hiding them.
- Selection anchors reset on file/revision changes. Preview stale IDs conservatively mark displayed drafts non-current.
- Unsaved source-bound prose is retained in `ContextViewState.commentEditor`. Existing pane renderer state retains this field across same-binding terminal/GUI switching. Recovery-batch changes clear the editor target; same-batch reload preserves prose.
- `App.tsx`: requesting control of a different pane now requests authoritative Herdr focus first. Local control alone previously could leave graphical controls permanently unusable.
- Existing App/client test fixtures gained six explicit throwing comment-method mocks. Those tests prove existing terminal/client behavior, not the new graphical workflow.

These are implemented changes, not claims that their real interactions passed. No Send or terminal-input comment delivery path was added; REF-02 remains separate.

## Checks that passed

The final Rust checks include the linked-worktree authorization repair. The final frontend source was unchanged after its passing checks.

```text
cargo build -p cockpit-host --bin cockpit
cargo test -p cockpit-core comments
  9 passed
bun run typecheck
bunx --bun vite build --outDir /tmp/cr-szrkq6lk/web-dist
bun run test src/app/App.integration.test.tsx src/client/client.test.ts
  27 passed
bunx tauri build --debug --no-bundle --config /tmp/cr-szrkq6lk/native-build.json
```

The Tauri build succeeded; it is not a native runtime smoke or an AppImage packaging check. Formatting and a final post-repair integrated review remain pending. Vite reported its large-chunk warning; Tauri reported the existing bundle-identifier warning. Neither warning was suppressed.

Nine focused core tests cover CAS persistence, symlink refusal, oversized prose refusal, source capture boundaries, sanitization, root-incarnation checks, quoted paths, and newline metadata. They do not replace real multi-window or host acceptance.

## Runtime checkpoint and reproduced defect

The real browser rendered the ordinary disposable workbench. Its setup flow created a linked checkout and a companion, with an initial root terminal and separate context-bearing terminal.

Before the repair, `w2:p2` presentation returned no roots and `provenance_conflict`: `worktree.list source checkout does not exactly match the requested cwd`. Stock Herdr returned the primary source checkout even when `worktree.list` was queried using the linked checkout. Core companion authorization had passed the linked checkout into a parser that requires the primary source cwd.

After the one-line Core repair, rebuild and gateway restart, the same presentation returned the expected authorized companion and an empty diagnostics array. This was asserted and saved as `linked-companion-presentation.json`. The terminal was still at its worktree cwd, so `can_open_context: false` remained correct. **No real file-viewer pane was launched for REF-01 before the pause.**

A separate setup layout observation is queued: at 1440×900, the setup footer intercepted the primary button's center. Keyboard activation worked. The todo item is `Repair setup footer overlap at supported window sizes`. Do not silently count pointer setup acceptance as passing.

## Remaining REF-01 acceptance

Use the complete original spec, including the lifecycle section, not only this list:

- Real companion-root viewer replacement and normal Herdr layout/focus. Compare with the disposable Herdr TUI; GUI drafts remain independent of the extension's private state.
- Collect comments from at least three files in browser and native UI. Include whole-file and range references, frontmatter, Unicode, CRLF/LF, missing final newline and quoted filenames.
- Verify exact preview bytes, absolute reviewed paths, relative display paths, ordering, line numbers, sanitizer count, framing count, deterministic duplicate preview and visible oversize refusal.
- Edit/delete through inline, file-bottom and overview presentations, preserving shared IDs and source capture.
- Two-window/two-host CAS conflict without overwrite; reload preserves unsaved prose. Prove persisted drafts across gateway/native restart.
- External source modification/deletion, explicit retain-captured-excerpt choice, stale markers after closing preview, and no automatic line rebasing.
- Root replacement and detached batch recovery without silent cross-tab/Space/source attachment.
- Graphical pane interaction after another pane has focus. Test file/revision selection reset and source-bound unsaved editor state across navigation and terminal/GUI switching.
- Keyboard ownership, normal terminal input and supported/minimum window geometry.
- Native command/runtime behavior, not merely HTTP requests or successful compilation.

Additional source-inspection concerns to resolve or explicitly test before acceptance:

- Core `capture_lines` splits on LF; Context UI `splitSource` also treats standalone CR as a line boundary. CR-only input may therefore disagree on numbered capture. This was noticed at handoff, not reproduced or fixed.
- The client anchor parser currently caps selected lines at 16,384 while Context configuration permits up to 20,000 lines. The client also bounds preview payloads at 4 MiB, which is not necessarily the same as the 4 MiB serialized-record bound after formatting expansion. Check end-to-end boundary consistency instead of assuming the default 5000-line fixture covers it.

## Evidence and stopped resources

Evidence directory:

`/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/reference-workflow/`

Contains `checkpoint.json`, `observations.json`, `resources.json`, `server-status.json`, `server-stop.json`, `initial-workspace.txt`, `fixture-files.json`, and `linked-companion-presentation.json`. There is no completed REF-01 browser/native comment proof in this directory.

Retained private fixture root: `/tmp/cr-szrkq6lk`.

| Resource | Identity/status |
| --- | --- |
| Herdr session | `run-20260904T214621Z-reference-szrkq6lk`, stopped through resource_guard; server exit 0 |
| Gateway | `cockpit-reference-web`, formerly `http://127.0.0.1:54277`, stopped with exit 143 |
| Browser | `reference-proof`, released |
| Native display | `cockpit-reference-xvfb`, Xvfb `:195`, stopped with exit 0 |
| Native application | Built, never launched |

Retained fixture layout:

- `catalog/demo`: disposable primary repository.
- `worktrees/demo-reference-proof`: disposable linked checkout.
- `companions/e4fa4ba1-9350-407e-81c2-08eaa1dc7e4d`: owned companion with three exact fixture files.
- `operations`: setup journal and empty comment collection; no runtime comment drafts were created.
- `config/herdr`, `state`, `home`: isolated runtime configuration and state. Plugin registry contains only the already-installed file viewer registration, pointing to its existing executable; no plugin was installed or modified.
- `cockpit.toml`, `native-build.json`, `web-dist`: isolated configuration and built frontend.

The original longer root exceeded Unix socket path capacity. Use a short root like `/tmp/cr-<unique>` for the next scenario. Never shorten by using default Herdr paths.

The stopped companion manifest pins the old Herdr endpoint process identity. Starting a new Herdr process does not make that old authorization current. Prefer a fresh disposable scenario; retained files are evidence and fixture material. Do not edit manifests to forge current provenance. Use real supported setup/reassociation and record any lifecycle limitation.

Selected executable: `/home/linuxbrew/.linuxbrew/Cellar/herdr/0.8.2/bin/herdr`, protocol 20/schema 1. Its recorded hash and source/schema artifacts are in the ledger. `scripts/verify/resource_guard.py` validates owned targets and the exact cleanup form `session stop <recorded-name> --json`.

Native tools are available under `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/native-tools/usr/bin/`; Xvfb and xdotool are not on the ordinary PATH. Native invocation needs explicit isolated `COCKPIT_CONFIG`, repository/state/worktree/companion roots, `COCKPIT_HERDR_EXECUTABLE`, `COCKPIT_HERDR_SESSION`, `COCKPIT_HERDR_SOCKET`, and display environment. Never let it autostart/select the protected default.

## Change ownership and continuation policy

Owned source changes span:

- Core comments, Context helper/error classification, service composition, project-store helpers, linked-companion authorization, and repository test configuration.
- Protocol comments, module/type exports and generated TypeScript.
- Host comment routes, route registration/error mapping and composition.
- Tauri comment commands, command/capability registration and composition.
- Client interface, strict comment parser, browser/native adapters and test fixture.
- Context comment UI, App focus boundary and App test fixture.

Pre-existing uncommitted bookkeeping includes `planning/next-level/execution/run-20260904T214621Z/goals/G02.json` and its top-level `resources.json`. `state.json` and `NEXT.md` contain earlier bookkeeping plus deliberate progress/handoff updates. Keep those histories distinct from source ownership when staging. This handoff is newly requested documentation. No source commit or bulk staging was performed.

`agent://ReferenceCoreReview` and `agent://ReferenceUiReview` contain the initial review findings; `ReferenceCoreRepair` and `ReferenceUiRepair` returned repairs. Main then fixed compiler failures and ran the gates above. Final integrated review is still required; old worker reports are not the final source state.

The broader todo remains intact. CTX-01/VIEW-01 are verified from `2827b40`; PANE-01/PANE-02 are partial because graphical Review is not implemented. REF-02 paste, media/Mermaid, watching/search, repository snapshots, source ingestion and REV-01/REV-02 remain open. Earlier LIFE/FND/repair/quality and release gates are not waived. At this historical handoff, stock Herdr application mouse was recorded as blocked under the then-active no-patch/no-expanded-investigation direction.

That handoff predates the 2026-09-06 evidence correction. Herdr 0.8.2 SGR delivery through `pane.send-keys` is verified, and ordinary xterm.js wheel delivery is user-observed. The active plans now treat physical direct-attach filtering as a path-specific repair target and allow explicit ownership-gated emulation; structured pointer routing remains a separate acceptance gate.
