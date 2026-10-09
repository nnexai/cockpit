# WS-01 Repo hygiene

Wave 0 · Size S · Depends on: – · Blocks: nothing (do it first; it makes the tree easier to read for everyone)

## Goal
The repository root contains only the product, its docs and its verification. Experiments and one-off review artefacts live under `archive/`.

## Owns
`poc/` (147 tracked files), `spikes/` (18), `.audit/` (26), `.gitignore`, `planning/supervisor-atlas-2026-10-06/.~lock.ATLAS.md#`, and path references to these directories anywhere outside the three root docs.

## Change
1. `git mv poc archive/poc`, `git mv spikes archive/spikes`, `git mv .audit archive/audit`.
2. `git rm` the committed LibreOffice lock file.
3. `.gitignore`:
   - add `.~lock.*#`;
   - ignore build output and runtime profiles under `archive/poc/**` (`node_modules/`, `target/`, `dist/`, `runtime/`, browser profiles);
   - ignore any untracked tool directories found at the root (check `.codex/`, `.playwright/`).
4. Fix references. Search scripts, `package.json`, `tsconfig.json`, `vite.config.ts`, the vitest config, `Cargo.toml`, `quality/` and `docs/` for `poc/`, `spikes/` and `.audit/`. Make sure no workspace, test glob or build includes `archive/**`.
5. Report the on-disk size of the untracked build output under the moved PoCs (about 17 GB). Delete it only after the owner approves.

## Keep
- `planning/` content (history; no binary moves, no history rewrite).
- `research/`, `quality/`, `scripts/`.

## Acceptance
- The root listing shows no `poc/`, `spikes/` or `.audit/`.
- `git status` is clean after a build (no new untracked noise).
- The test and build inputs are unchanged.

## Verify
- `bun run typecheck`, `bun run test` (the collected test count matches before/after), `cargo metadata --no-deps` (unchanged members), `bun run build`.
