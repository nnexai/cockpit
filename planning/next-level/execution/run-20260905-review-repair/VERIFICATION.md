# Review and ordinary folder repair

Implemented local Git Review access independently of task setup (access/menu commit `23a261d`), verified file-viewer replacement for ordinary browsing roots, compact Review layout and file tree, inline comment editors, pane-local keyboard navigation, and bottom status actions. Comments retain captured revisions internally but export the concise Reviewr path/range, selected source, and message format. Saved batches can be discarded with generation checks and durable deleted-ID markers.

## Verification

All mutations and GUI interaction used disposable Herdr session `ck-rv-20260905`, gateway `127.0.0.1:54841`, and browser CDP `127.0.0.1:54842`. The default session and user's gateway at port 4173 were not changed or restarted.

- `cargo test --workspace`: passed, including core, Herdr, host, protocol, and native tests.
- Frontend suite: 103 tests passed before the final file-navigation regression; focused Review tests also cover collapsed ancestors and rendered file order.
- `bun run build`: passed; Vite reports its existing large-chunk advisory.
- `cargo fmt --all -- --check`, `git diff --check`, generated TypeScript check: passed.
- Native command registration, capability, and invocation added for batch discard; native compiles in the workspace gate. Native GUI discard was not separately exercised.
- Real browser proof covers inline C/Shift+C actions, old/new expanded source, selected ranges and hunk navigation, status comment count and overview, ordinary folder reads, hidden `.git`, and symlink refusal.
- Comment upsert measured about 770 ms after removing redundant inspection work, versus 4568 ms before, in this disposable fixture. This is a fixture measurement, not a general latency guarantee.

Evidence and retained fixture: `/tmp/cc-rv-20260905/`. Test logs: `sidebar-workspace-tests.log`, `sidebar-frontend-tests.log`, `sidebar-build.log`, `discard-store-tests.log`, `discard-transport-tests.log`. Runtime proof scripts and screenshots remain beside them.

Final browser evidence: `final-discard-reload-proof.json` confirms discarding the saved two-comment batch and reloading to zero comments. `final-tree-keyboard-proof.json` confirms navigation opens collapsed ancestors. `final-comments-compact-1440x900.png` shows the compact sidebar/payload. `final-loaded-1440x900.png` and `final-loaded-1024x640.png` were captured after waiting for actual diff lines. The nested tree fixture's 11 files now use three directory summaries instead of twelve. Final ordinary-folder presentation has an empty diagnostics array.
