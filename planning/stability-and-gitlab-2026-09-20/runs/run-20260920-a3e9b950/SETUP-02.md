# SETUP-02 — descriptor-bound companion publication

## Accepted plan

Accepted by Main, 2026-09-21, consuming settled SETUP-01 interfaces under OBS-011. Baseline commit remains `6593cf4ecba87687c809ee9f3d2a295c757db787`. Expected publication/recovery behavior remains; OBS-013 assigns actual macOS execution to the user and OBS-014 selects lightweight delivery checks.

Observable outcome: Linux retains atomic no-replace publication; Darwin uses its supported descriptor-bound atomic no-replace primitive. Unsupported platform/primitive and known parent prerequisites fail before Herdr mutation. Later failures preserve exact existing operation ownership and recovery.

### Ownership and integration contract

- Publication worker owns only `crates/cockpit-core/src/project_store.rs` and, only if indispensable, its filesystem dependency declaration. Reuse existing rustix/nix/capability APIs; no generic filesystem layer or ordinary-rename fallback. Read staging, manifest, journal and recovery consumers together before editing. Preserve foreign destinations, including empty directories, and clean only proven operation-owned staging.
- Expose a narrow `ProjectStore::preflight_companion_publication(&self, companion_root: impl AsRef<Path>) -> Result<(), InspectionError>` that checks supported publication capability and existing parent validation. It must not create a final companion or any Herdr resource. Reuse existing storage-root preparation/validation rather than inventing generic capacity/probe infrastructure. Report limitations honestly; final publication remains authoritative under races.
- Preserve one publication commit point after complete staged manifest/durability preparation. Include supported parent durability after publication and retain enough existing evidence to reconcile a post-publication acknowledgement failure. Do not rewrite ownership/journal models unless a demonstrated crash window requires a minimal change.
- Main owns the `projects.rs` integration after the GLAB-02 core worker hands it off, placing preflight before the first Herdr mutation through existing revalidation/sequencing. Main also reviews any demonstrated same-operation recovery gap requiring a shared-boundary repair. No other worker edits `projects.rs` concurrently.
- Filesystem worker may inspect consumers but must report, not edit, required shared-boundary changes. No browser/native UI, source, installer, or generated-file edits.

### Verification and blockers

Filesystem/lifecycle review and repairs are complete. Darwin uses rustix's descriptor-relative no-replace primitive; Linux retains no-replace publication. No ordinary-rename fallback was added. Existing exact-manifest recovery now re-syncs the descriptor-bound manifest, child and parent before acknowledging an already-published companion.

Actual macOS filesystem/native checks are **user verification—not executed**, covered by [MACOS-HANDOFF.md](MACOS-HANDOFF.md). They are not a delivery blocker under OBS-013, and Linux results do not imply a Darwin pass.

## Evidence

`CARGO_BUILD_JOBS=2 cargo test -p cockpit-core companion -- --nocapture` passed: 5 relevant tests across two suites, zero failures, 17.21s including compilation. The integrated core/providers/host compiler check and host build also passed.

The actual gateway setup smoke completed the reviewed operation, created its owned worktree/companion and rendered the ready Space/terminal. Main read the persisted completed receipt and both screenshots. [SETUP-browser-evidence.json](SETUP-browser-evidence.json) records exact identities, paths and retained-resource ownership. This exercises the Linux publication path; no injected crash or native/macOS claim is made.

The same increment integrates publication preflight and exact-manifest recovery in ProjectService, explicit reviewed Start in SetupDialog, and the already-implemented source branch/head defaults. Deeper crash/platform scenarios remain named follow-ups under OBS-014 rather than blockers to this verified working path.
