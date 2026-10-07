# Library-direct context (no mandatory companion)

## Target behavior
Agents read live Library files in place. Each Space keeps a list of selected Library items (relevance only). It does not copy them, pin versions, or track copy/retry/update state. When a Library item refreshes, agents get the new version the next time they read it. The Library stays Cockpit-managed and read-only by convention. This is not sandboxed. Task notes go in the working checkout. Setup no longer creates a companion directory. Extra repositories are exposed as existing local paths and are never copied. Library folder capture stays as it is.

## Preservation / migration
- Leave existing companion directories and Space copies on disk untouched: no deletion, no import.
- Migration maps a Space's existing copy records (`context_assets.rs` manifest/state) to Library item IDs where the metadata already links them, and turns those into selections. Unknown files are left alone. No broad data audit.
- Pending saved Library items are never lost. Selections count as references for the existing reference-based GC.
- Repository worktree ownership/teardown safety in `project_teardown.rs` and `project_store.rs` stays as it is. Only companion duties are removed.
- Migrate persisted setup/teardown records before removing companion fields, so existing operations remain readable. Do not delete preserved companion files during teardown.

## Steps
1. **Core: selections replace copies.** In `crates/cockpit-core/src/library/space.rs`, bind selections to fresh Herdr session/Space identity, not branch or cwd; keep existing Library path safety checks. Drop the exactly-one-companion requirement (324-386) and store the per-Space selection. Remove obsolete companion-copy machinery/callers from `context_assets.rs`, without shims. Keep Library internals and folder capture. Migrate known copy records to selections. Outcome: adding to a Space copies nothing.
2. **Core: remove companion from Setup and teardown.** In `projects.rs`, remove companion creation (453-457, ~1888-1953) and resolution/provenance (1181-1240). In `project_store.rs`, remove `CompanionManifest` and the companion setup/teardown records. In `project_teardown.rs`, remove companion lifecycle. Keep `COCKPIT_LIBRARY_ROOT` in env (2008-2033) and drop the companion path. Outcome: Setup succeeds with no companion directory.
3. **Protocol and Context sources.** In `context.rs` `source_options` (229-284), expose the Library root kind (`cockpit-protocol/src/context.rs:16-21`), the selected items, the checkout and additional repo paths. Regenerate the protocol with the existing generator and migrate every client caller in `src/client/CockpitClient.ts`. Outcome: the Context viewer works without a companion and still uses real paths for comments and agent paste.
4. **UI.** In `App.tsx`, stop disabling Context without a companion. In `SpaceContextList.tsx`, show selected items instead of copy status. In `AddContextDialog.tsx`, save to Library, then associate with the Space. Reuse the repository catalog/picker to select additional local repo paths, without copying them. Show the Space name in `FilesLeaf.tsx`; adapt `ContextViewer.tsx` and `ContextResources.tsx`. Outcome: no Space-copy/update/retry controls; ordinary Library operations remain.
5. **Agent discovery command and docs.** Add a read-only `cockpit context --current` command in `crates/cockpit-host/src/bin/cockpit.rs`, modeled on `browser --current` (329-372). It prints the Library root, the selected real paths, the current checkout and the additional repo paths. Update the skill/instructions to point agents at it, without retrofitting env into existing terminals. Rewrite `CONTEXT.md` (128-152, 237-251, 314-331, 345) and `DECISIONS.md` (43-49, 53-58, 89-92). Update affected tests and remove obsolete ones.

## Acceptance
1. New Setup in a disposable fixture creates no companion. Context opens and lists the Library root, selections and checkout.
2. Add-to-Space saves the Library item and links it to the Space. No copy appears under any Space directory.
3. After refreshing a selected item, `cockpit context --current` returns the same path and reading it shows the new content.
4. A pre-existing companion directory with a user note survives the upgrade unchanged. Known Library-backed copies show up as selections.
5. Selected items survive Library GC. File comments and agent paste still use real paths.
6. In a fixture terminal, an agent runs `cockpit context --current` and reads the listed files. The browser fixture smoke passes.
