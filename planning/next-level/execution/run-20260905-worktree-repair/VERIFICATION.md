# Worktree removal and branch comparison repair

Baseline: `858df61`; initial worktree clean.

## Review comparison

Selecting Branch previously triggered an immediate snapshot with a null base ref. Changing each input character also refreshed and disabled the input. Draft text is now separate from the applied ref: Enter or Refresh applies it, and an empty initial base shows instructions without requesting a snapshot or retaining the previous scope's files.

- Red: `bun run test src/app/review/ReviewPane.test.tsx`, captured in `/tmp/cc-branch-red.log`, observed the unwanted empty-base request.
- Green: all eight focused Review tests passed, including branch selection, typing without requests, retained input focus, and Enter applying the complete ref.
- Frontend suite: 114 tests passed across 17 files. Production build passed. Logs: `/tmp/cc-worktree-frontend-tests.log`, `/tmp/cc-worktree-frontend-build.log`.

Installed Reviewr source confirms explicit refs resolve through Git. This repair preserves Cockpit's explicit base-ref comparison control; it does not guess a repository's default branch.

Editing an applied base also cancels pending responses and clears the old comparison. Regression coverage includes a delayed response after an edit and clearing the ref before leaving and returning to Branch. Read-only review identified this case before delivery; the repair and focused tests passed.

## Removal receipt

Stock Herdr 0.8.2 returns `worktree_removed` with top-level `path`. Cockpit expected the nested `worktree.path` shape belonging to an event, so it could report a malformed response after successful removal. The adapter now reads the actual response shape and retains exact workspace, path, and force checks before companion deletion.

- `cargo test -p cockpit-herdr parses_installed_worktree_removal_receipt` passed. Agent first observed red `removed worktree is required`; the fixture uses the installed server's response. Mismatched workspace, path, and force are rejected.
- `cargo test --workspace` passed, captured in `/tmp/cc-worktree-rust-tests.log`. Backend build and formatter check passed.
- Disposable stock Herdr removal succeeded for a linked checkout, including with a real Reviewr pane focused. A primary or stale target reproduced `not_linked_worktree`.
- The user's default session was inspected read-only after they identified it. Its ordinary worktrees and `gui-ideas` task were correctly reported linked. Non-mutating teardown preview recognized the task and blocked removal for tracked/untracked changes. No mutation, focus/input control, or removal was performed in default.

The user's exact `not linked` rejection has not yet been reproduced through Cockpit with a valid linked target. Do not represent the receipt fix as proof of that symptom's cause.

## Live browser acceptance

Disposable session `ck-rp-20260905-a83`, gateway 54951, CDP 54952, evidence `/tmp/cc-rp-20260905-a83`.

- `branch-proof.json`: selecting Branch sends zero snapshots and shows no alerts. A real click followed by typing `main` retains focus and sends zero snapshots. Enter returns a real diff/hunk; changing to `HEAD~1` and clicking Refresh succeeds. Switching to all-local restores local/staged/untracked files.
- `teardown-proof.json`: a task created through Cockpit's GUI is verified as clean and owned, with its companion. Confirmed GUI Remove task worktree succeeds and the workspace disappears from the authoritative snapshot.
- Screenshots: `branch-selected-no-request.png`, `branch-draft-main-focus-retained.png`, `branch-main-diff.png`, `branch-changed-ref-refresh.png`, `local-scope-restored.png`, and teardown preview/completion screenshots.

The disposable session remains temporarily active for the user's follow-up file-viewer launch acceptance. Its runtime owner will stop it after that proof.
