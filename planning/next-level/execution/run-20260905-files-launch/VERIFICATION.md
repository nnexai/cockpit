# Ordinary file-viewer launch and raster preview

Baseline: `9bdb262`. This increment owns the Pane/Commands file-launch controls, verified folder launch capability, and ordinary-folder image preview. It follows the user's additions during worktree repair.

Pane and Commands expose Open files right/below beside Review. Open Context retains companion-specific behavior. The frontend uses a freshly inspected opaque files-root ID; launch revalidates the source pane and browsing root before calling Herdr's existing plugin operation.

PNG/JPEG preview uses the existing bounded media reader for verified Folder and Companion roots. It retains revision, byte/pixel, path and no-symlink checks. Companion-only imports/search/snapshots remain separate.

## Verification

- `bun run test`: 117 passed across 17 files, including Pane-menu launch without a companion, file-root protocol validation, and ordinary Folder PNG routing. `/tmp/cc-files-frontend-tests.log`.
- `cargo test --workspace`: passed. New source-root test covers Git top-level resolution, unchanged default root, Files launch, refusing Git metadata/repository IDs, and Folder media authorization. Existing bounded PNG/JPEG tests also passed. `/tmp/cc-files-rust-tests.log`.
- Frontend production and backend host builds passed. `/tmp/cc-files-frontend-build.log`, `/tmp/cc-files-backend-build.log`.
- Formatter, generated-type, and diff checks passed. No native-only command/channel/window implementation changed.

The first live Files launch exposed an adapter regression: supplying browsing `cwd` to `plugin.pane.open` made stock Herdr resolve the file viewer's relative executable beneath the browsing folder. It failed to spawn and created no pane. `/tmp/cc-rp-20260905-a83/files-open-failure.png` captures the failure. The launch adapter must preserve plugin-root execution and use verified pane context for browsing.

The adapter repair omits Context launch `cwd` while preserving Review's explicit checkout. It verifies the source directory before launch, then the installed plugin root, executable/process generation, and actual `HERDR_PLUGIN_CONTEXT_JSON` browsing root. Focused tests pin the differing payloads and Git-root resolution.

Final live browser proof passed: `/tmp/cc-rp-20260905-a83/files-proof.json` has `failure: null`. Both Pane and Commands enable Open files right/below from the ordinary shell, keep Open Context disabled without a companion, and the new real Files pane reads `notes.md`. Selecting `pixel.png` produces an image with natural dimensions 1×1. Screenshots `files-pane-menu.png`, `files-folder-png.png`, and `files-commands-menu.png` were inspected. The initial tiny PNG fixture had invalid chunks and was replaced with a CRC-correct generated fixture; production validation correctly refused the invalid file.

After the adapter change, one parallel workspace test run hit the existing `try_execution_lease_is_nonblocking_and_exclusive` assertion. The isolated test and subsequent full workspace rerun passed without changing that test or its implementation. `/tmp/cc-files-lease-recheck.log` and `/tmp/cc-files-rust-tests.log` retain the successful reruns.

Disposable Herdr session `ck-rp-20260905-a83`, gateway 54951, and Chrome CDP 54952 were stopped using ownership-checked cleanup. Evidence and fixtures remain in `/tmp/cc-rp-20260905-a83`. The user's default session and gateway 4173 were not mutated.
