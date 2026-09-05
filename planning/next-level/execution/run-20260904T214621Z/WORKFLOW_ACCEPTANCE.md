# Context workflow increment, 2026-09-05

This increment completes the resumed Context workflow implementation: durable whole-file/range reference comments, bounded companion search and known-file invalidation, acknowledged same-tab agent paste and explicit receipt resolution, task cleanup/recovery, and explicit local repository snapshots.

## Verification

- `cargo test --workspace --exclude cockpit-tauri`: all tests passed, including 50 core tests and the transport/protocol suites.
- `bun run typecheck` and `bun run test`: passed, 80 tests across 11 files.
- Browser gateway and frontend builds passed. Native debug no-bundle build and real Xvfb startup passed. The last snapshot safety repairs and receipt ordering fix were covered by the final Rust suite after that native build.
- Actual browser comments captured three files, Unicode, a quoted path, CRLF/frontmatter range, absent final newline, stale source with explicit excerpt retention, and concurrent-window CAS recovery without losing unsaved prose.
- Actual native comments persisted into the same batch; clipboard preview was exactly 2,784 bytes.
- Actual browser agent-paste action selected the stock-Herdr-detected same-tab fixture. Captured bytes equal the displayed 2,784-byte UTF-8 preview enclosed once in bracket markers: 2,796 bytes, no trailing submission byte. Accepted delivery archived all seven drafts.
- Browser search opened the matching file; known-file polling detected an external edit and refreshed the source. A file changed before its initial read refused the stale directory revision and recovered through Refresh.
- Browser cleanup blocked removal of a dirty owned worktree and preserved its file. A separate clean task was created through Setup then removed with typed confirmation. Durable unknown/orphan recovery has a global menu entry independent of live Spaces; its mounted UI test covers reconcile then explicit companion cleanup.
- Browser snapshot copied tracked dirty and untracked nonignored bytes, skipped ignored files and an outside symlink, and used distinct destination inodes. Editing the destination did not change the source. Reflink unsupported/cross-device was reported explicitly before verified copy fallback.

Review repairs include paste ambiguity classification, archive CAS reconciliation, bounded striped locks, full receipt safety scans, actionable history ordering, source-root substitution rejection, companion manifest serialization, and tracked build-directory exclusions.

## Evidence and limits

Retained evidence: `/home/nnex/.local/state/cockpit-execution/run-20260904T214621Z/continuation-20260905/evidence/`. Active disposable process identities remain in its parent `resources.json`; see `CONTINUATION.md` for guarded restart and next work. No default Herdr session or manual gateway was automated.

The paste runtime fixture is a harmless raw input collector named `codex`, detected by real Herdr. It proves delivered bytes and no submission, not semantic processing by a model-backed agent. Native session metadata strengthens target identity when available; absent metadata uses terminal and agent identity. Herdr lacks conditional atomic same-tab writes, so arbitrary concurrent external focus/moves cannot be made atomic by Cockpit.

Snapshot bounds are 512 files, 4 MiB per file, 32 MiB total. Source changes during capture are rejected; snapshots are current filesystem copies, not exact commit checkouts. No hardlinks or Git alternates are used. Search uses bounded known-file polling, not a filesystem event watcher.

This is not completion of the entire selected plan or release acceptance. Local graphical Review, sources/provider integration, safe media/Mermaid and remaining quality gaps continue. Upstream application mouse and the user-excluded expanded terminal matrix remain outside this continuation's completion scope.
