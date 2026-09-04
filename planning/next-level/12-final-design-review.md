Current selection: the user subsequently chose stable Herdr as the default and parked protocol 22, while requiring mouse handling to be preserved. [13-terminal-stability.md](13-terminal-stability.md) supersedes the alternatives in this review.

# Final design review

Reviewed 2026-09-04 against `8fac660`. Planning only. This review records recommendations, distinguishes existing decisions from proposed refinements, and does not authorize application changes.

My recommendation is a focused restructuring before the new features, followed by an early real-task trial. Keep the Rust/React/Tauri stack and Herdr authority. The reported whole-view redraw flicker and poor scrolling reopen the current terminal renderer and custom protocol-22 dependency decision; neither is an accepted stability baseline. The strongest opportunities are concentrated command/state ownership and shared document/comment behavior. Rewriting the application would put the most expensive existing compatibility work at risk without resolving those problems by itself.

The [existing-code review](../../research/next-level-existing-code-review.md) records concrete failures and verification limits. Repair those in separately reviewed changes before treating the current behavior as the cleanup baseline.

## 1. Restructure the workbench before adding features

Already planned in CLEAN-02; make it the priority extraction.

`src/app/App.tsx` currently contains layout projections, the mutation reducer, focus fallback timers, command dispatch, several views, and the session lifecycle. `PaneView` at line 540 directly composes `TerminalPane` with terminal-specific geometry and control arguments. `Workbench` at line 668 owns menus, the prefix handler, navigation, and mutation dispatch. `App` at line 788 owns subscription and focus recovery. This makes a new interaction likely to touch unrelated lifecycle code.

Extract the session/focus/mutation coordinators and pure layout projection first. Keep `sessionReducer` as the existing event acceptance implementation; do not introduce a second store or state-management package merely to move it. Then make `PaneView` the Herdr-owned rectangle and local renderer host. Terminal, Context, and Review each manage their own visible renderer resources behind that host. A renderer failure must not kill a pane or affect the next renderer's input subscription.

Acceptance should include switching Terminal → GUI → Terminal, external focus changes, reconnect, and pane movement with no duplicate input listener or forgotten subscription. Preserve terminal cell geometry and the complete-tab client-shell surface. The terminal path in `TerminalPane.tsx` and `cockpit-herdr/src/terminal_wire.rs` now needs a dedicated stability assessment before extraction. A bounded rewrite of patch/image presentation or attachment identity may be justified by reproduced failures. This does not justify rewriting the whole application.

## 2. Give commands one implementation

A small additional refinement to CLEAN-02 and PANE-02.

`Workbench.runCommand` currently dispatches many actions directly, while row/menu/pane callbacks also build mutations. As quick-open, comments, context menus, and new GUI renderers arrive, duplicated action handlers would make personal changes harder.

Use a closed, typed command table with an action ID, label, applicable focus scope, availability reason, and handler. Menus, the command chooser, and shortcuts invoke that same handler. Separate resolving a keyboard gesture from executing an action. Keep the existing Herdr prefix behavior and precedence. Ordinary terminal input never goes through the GUI command table.

Put editable personal defaults in one ordinary code module. A shortcut or target-selection tweak should require changing that module and its focused behavior test. No downloadable command plugins, scripting language, workflow engine, or generic dependency-injection system is needed. Rewrite the small duplicated command dispatcher after characterization tests; preserve the public behavior during extraction.

## 3. Share document and comment behavior from the first viewer slice

A contract refinement across FND-03, VIEW-01, REF-01, and REV-01/02.

Context and Review should share an immutable document snapshot, source-line selection, annotation identities, batch persistence, and payload formatting. They need different readers: companion files, an authorized local checkout, and an old/new Git revision. A diff is a presentation of two revisions, not a second implementation of commenting.

The current FND `FileRef` definition assumes a companion, while PANE-02 explicitly supports a file viewer rooted in an authorized repository and Review reads the worktree. Introduce a closed tagged root reference with companion and authorized-checkout variants. A revision reference then distinguishes captured file bytes from a Git revision/side. Every variant retains its own containment/provenance validation; this is not permission to accept arbitrary absolute read paths.

Use one source-line/annotation module for both GUI replacements, with Markdown and diff presentation supplied by their owning modules. The root reference and document revision belong in the protocol/core contract; hover, selection, scrolling, and inline-comment placement stay in frontend modules. Share only behavior both readers actually need.

Accept a cross-view fixture that comments on a companion file, a checkout file, and a removed Git line; verifies the actual path and original numbered excerpt; edits each annotation; and sends one correct batch. Refresh must not overwrite captured excerpts. This avoids maintaining two subtly different versions of the user's main interaction.

## 4. Separate durable drafts from live session attachment

This is already the intent of the detailed draft lifecycle section. The shorter FND and REF definitions still mention the connection epoch as part of batch identity. Align them before agents implement either story.

A durable batch has its own stable ID, source provenance, and last-known owner location. Its current attachment to a Herdr pane has a connection epoch and freshly verified identities. Reconnect changes the attachment, not the batch ID. A closed pane leaves detached recoverable drafts. Recovery cannot grant permission to paste into a different tab; that always requires fresh same-tab target validation and explicit user intent.

This distinction also keeps file reads and annotations out of the live Herdr state reducer. Herdr event state, durable Cockpit data, and transient GUI selection have different lifetimes and should have separate owners.

## 5. Make source freshness explicit and setup repeatable

An interaction refinement within LIFE-01/03, CTX-02, and SRC-03, not another setup subsystem.

Use remembered non-secret defaults for each discovered repository: normal worktree location, selected reference repositories, and preferred supported source provider. Review the concrete effects before creating resources. Keep the local primary repository required and the issue/MR URL optional. Selecting a URL should not start downloads or provisioning by itself.

Local repository copies should behave as pinned snapshots by default. Show their source repository, captured revision/dirty status, and whether the current file is a live worktree file or copied reference. Refresh is an explicit operation. Reflinks save storage; they do not make the copy track future edits. Do not silently synchronize reference repositories while the user is annotating them.

Replace the mock's clunky source configuration with actions on the relevant source tree entry: add, refresh, inspect freshness, or retry. Ordinary reading should not require visiting a source-management screen. This is a recommendation for the next design iteration, not approval of an unshown layout. Pane/ellipsis button styling also remains unresolved.

## 6. Keep quality infrastructure smaller than the application work

Keep CLEAN-05 and its deterministic feedback. Reconsider the amount of custom integration required before allowing feature work, rather than dropping testing or relaxing the agreed target silently.

The highest-value first checks are ordinary tests/typechecking, import/crate dependency rules, coverage, and mutation fixtures for focus acceptance, reference formatting, path/ownership policy, and delivery state. Establish the changed-code report and reviewed baseline early. Add tools through small adapters; avoid constructing a general quality platform.

Probe per-function CRAP mapping before committing to a custom cross-language analysis layer. The proposed ceiling of 8 remains the default. A cohesive state transition or exhaustive protocol dispatch may deserve a narrow reviewed exception if splitting it would scatter its invariant across helpers. Such an exception must name the function, explain the structure, retain complexity/coverage/mutation evidence, and be revalidated when its source changes. It must not become a broad exclusion or let the implementing agent raise its own baseline.

Measure the fast-check and mutation runtimes during CLEAN-05. Run fast checks while editing and relevant changed-code mutation before handoff; full mutation remains a deliberate run. If a provider cannot produce trustworthy function mappings, report the gap explicitly. A temporary separate complexity/coverage policy would require an owner decision and must never be labeled a passing CRAP gate. The plan does not silently remove CLEAN-05 as a dependency.

Luna-high is a reasonable lane for bounded implementation under clear contracts. Model choice should not determine whether tests or gates are required. Reserve deeper review for ambiguous state ownership, filesystem effects, or protocol races.

## 7. Keep full GUI review, but protect its read-only scope

I would keep the full graphical replacement. It fits the chosen pane model and the reference workflow. Its cost grows sharply if it becomes a general Git client or a remote forge review client.

REV-01/02 should deliver correct local comparisons, changed files, bounded hunk expansion, old/new line comments, file inspection, and paste to an agent. Staging, commits, conflict resolution, and posting remote review comments are not needed for this loop. They remain outside this review's proposed scope. Side-by-side diff remains deferrable after a complete unified view. Remote review ingestion stays separate from local review rendering.

Keep provider breadth, remote access, multi-user credentials, distribution work, and TUI backport out of the first daily-use milestone. They already have selectable stories. Do not let the fact that they are thoroughly planned turn them into prerequisites.

Detection and GUI replacement should remain the selected architecture. Preserve the explicit per-pane renderer fallback already planned when process detection is ambiguous. Do not trade that fallback for extension IPC or a new server dependency.

## 8. Trial one real task before expanding the source matrix

Add a personal-use acceptance checkpoint after M2, using manually created context if downloads are not ready. Keep M3 in the main delivery plan; this checkpoint does not discard source downloads.

Use a disposable task worktree to read context, inspect source, collect comments across files, switch panes/tabs, reconnect, restore drafts, and paste into an actual supported agent without submission. Then run a real short task with the tested build while keeping the existing working build available. The implementation owner records input-focus mistakes, extra actions, stale context confusion, and draft recovery results. Feed those findings into the source setup and pane-control design before expanding providers.

This is more useful for a personal primary tool than completing every planned panel before using any of them. The HTML mock establishes interaction intent; it cannot establish real terminal focus, process detection, paste behavior, or acceptable latency.

## Order and parallel ownership

| When | Work | Owner / parallelism |
|---|---|---|
| First gate | Whole-view redraw and scrolling reproduction; stable/default transport decision | One terminal integration owner; native/browser evidence |
| Before implementation | Align FileRef and durable-batch definitions; preserve explicit current/future authority | One contract owner; documentation correction only |
| CLEAN-01/05 | Behavior baseline, quality probes, small deterministic gate | Quality lane can probe while the integrator records runtime behavior |
| CLEAN-02/03 | Workbench state/command ownership; narrow Rust transport/operation extractions | Frontend and Rust lanes in parallel; integrator owns shared contracts |
| FND-03 / VIEW-01 / REF-01 | Shared document references, snapshots, annotations, batches | One shared-contract owner; Context and Git readers consume those contracts |
| LIFE / CTX / SRC | Repository defaults and explicit source freshness/actions | Separate lifecycle/source lane after contracts are stable |
| After M2 | Real-task interaction checkpoint | Integrator plus user feedback; no provider expansion required to start |
| REV / later optional stories | Full local graphical review, then independently selected source/provider breadth | Review renderer can proceed against shared document/comment fixtures |

No application-wide rewrite is proposed. The terminal stabilization gate now precedes normal cleanup and feature delivery; see [the terminal stability decision](13-terminal-stability.md). Preserve one owner for each contract and put definitions in the owning plan rather than repeating competing versions in the root records, feature stories, and mocks. During implementation, mark completed planning proposals as implemented or superseded so later agents can distinguish current behavior from future intent.

## Further decisions to reconsider

**Freeze contracts per increment.** FND-02 currently lists all future operation families under a single contract-freeze instruction. Freeze shared identity/error/operation conventions and the next vertical slice first. Add source-provider and full-review DTOs when their owning story begins, before its parallel consumers. Planning the full inventory is useful; generating unused methods and placeholder modules is not. Keep stable story IDs and planned contracts without pretending every family must ship at M0.

**Keep the cache small initially.** The central source cache is already part of the chosen plan. Its first implementation can be immutable normalized objects plus atomic companion materialization, with simple per-source metadata. Do not add a general indexing/catalog database, revision browser, automatic cache GC, or dependency scheduler before repeated use needs it. Preserve provenance, copy isolation, conflict handling, and cross-process locking. I would not remove source downloads from the main scope to make the plan look cheaper.

**Revisit the exact Herdr release pin deliberately.** `cockpit-herdr/src/cli.rs` rejects any version unequal to `REQUIRED_VERSION` before checking the protocol/schema. That is defensible for initial compatibility work, but it can make a routine Herdr upgrade disable a personal primary tool. Keep the protocol-22/client-shell handshake and schema gates. Establish a small tested compatibility policy and fixture/native-browser smoke procedure for approving another release. Do not assume a matching protocol number guarantees compatibility or silently accept untested versions. This is a proposed upgrade workflow, not permission to relax today's gate.

**Make recovery behavior an explicit design topic.** Fixed-delay reconnect loops, unbounded request waits, mutation completion, and stream ordering are present-day concerns, not future feature details. A small coordinator should distinguish attempted, dispatched, awaiting confirmation, confirmed, rejected, and unknown states where the existing transport provides that evidence. Timeouts after a mutating request was written must not become automatic retries. Keep the current protocol unchanged unless a Cockpit-side ordering/receipt field is needed; no Herdr-server change is proposed.
