# 09. Maintainability priming and cleanup

Status: proposed planning milestone; no implementation. This plan keeps the current Herdr behavior and the feature plan independent. It uses Module, Interface, Depth, Seam, Adapter, Leverage, and Locality as design terms.

## Purpose and sequencing

Cockpit is a personal primary tool, so the useful measure is how many owning modules a small behavior change crosses. Do a short priming milestone before feature implementation, then keep the larger cleanup elective. The priming work should establish seams and characterization checks around existing contracts; it should not become a rewrite or a generic plugin framework.

### CLEAN-01: behavior baseline and architecture map

**Current evidence.** The wire path is `crates/cockpit-protocol/src/v1.rs` → `crates/cockpit-core/src/lib.rs` → `crates/cockpit-herdr/src/cli.rs` → `crates/cockpit-host/src/server.rs` or `src-tauri/src/lib.rs` → `src/client/CockpitClient.ts` → `src/app/App.tsx`. `HerdrAdapter` is defined in `cockpit-core/src/lib.rs:64-91`; `CockpitService` and its public operations begin at `:98-219`. `HerdrCliAdapter` is re-exported by `cockpit-herdr/src/lib.rs:1-5` and implemented in `cli.rs`, with transport and operation methods around `:900-1660` and the trait implementation around `:1997-2100`.

The frontend currently concentrates several Interfaces in `src/app/App.tsx` (986 lines): pure layout/selection helpers at `:96-300`, modal and navigation presentation at `:302-667`, `Workbench` orchestration at `:668-787`, and `App` session lifecycle at `:788-986`. `sessionReducer.ts:11-309` owns stream epochs, generation/sequence checks, focus confirmation, and terminal attachments. `TerminalPane.tsx:9-531` owns xterm creation, frame/input conversion, stream lifecycle, mouse geometry, and close behavior. These are useful existing seams; move behavior behind them before changing semantics.

**Baseline.** Record current `typecheck`, `test`, `cargo test --workspace`, and `cargo check -p cockpit-tauri`. Add characterization tests only for contracts at risk during extraction: stale stream rejection and sequence gaps in `sessionReducer`, focus timeout/token behavior in `App.tsx`, layout resize request derivation, terminal mouse coordinate conversion, Herdr mutation method/parameter mapping, and host resource-ID validation. Capture one disposable real session snapshot/stream/focus/terminal run. Do not add tests that merely repeat a moved function.

**Target map.** Protocol owns serializable meanings; core owns policy and the `HerdrAdapter` Interface; `cockpit-herdr` is an Adapter for Herdr CLI/socket behavior; hosts own request decoding and composition; the client owns wire parsing and transport; frontend Modules own projection, focus/input policy, and rendering. A feature should cross each Seam through one typed Interface.

Acceptance: the baseline commands and fixture are documented; each current behavior has one named owner; changing a layout rule, stream acceptance rule, or Herdr method mapping can be reviewed at one Seam. Dependencies: none beyond the current repository. Read the planned FND contracts as future requirements, but do not implement them in cleanup. This is the first milestone; CTX-01 follows the priming gate.

### CLEAN-02: frontend session, focus, input, and layout Modules

Extract by responsibility while preserving the current DOM and CSS contract.

1. Create `src/app/session/sessionStore.ts` around `SessionState`, `SessionAction`, `sessionReducer`, and `focusFulfilled`. Its Interface accepts epoch/session-scoped events and returns authoritative snapshot, stream, focus, and attachment state. Keep sequence/generation rules in this deep Module.
2. Create `src/app/session/focusCoordinator.ts` around `focus`, `scheduleFocusFallback`, focus tokens, and resync. It should expose `requestFocus(request, location)` and `retryFocus()`, with Herdr confirmation as the only success signal. `Workbench` consumes callbacks and does not own timers or stale-request checks.
3. Create `src/app/session/mutationCoordinator.ts` around `MutationOperation`, `mutationCoordinatorReducer`, `mutate`, retry classification, and operation errors. Its Interface returns accepted/succeeded/failed state and preserves the current one-mutation-at-a-time rule.
4. Create `src/app/layout/layoutProjection.ts` around `deriveResizeHandles`, `resizeRequest`, `projectedPaneIds`, `projectedPaneRect`, and `tabDropInsertionIndex`. It translates authoritative `TabLayout` into render rectangles and typed mutation requests; it never stores a competing layout.
5. Create `src/app/input/keymap.ts` around `PrefixCommand`, `prefixCommandForKey`, editable-target checks, modal escape/tab behavior, and the `Workbench` keydown policy. Keep terminal input conversion in `TerminalPane.tsx` initially; later extract `terminalInput.ts` only if it gives a stable Interface.
6. Leave `Spaces`, `Agents`, `TabStrip`, `PaneView`, `ResizeHandles`, and overlays as presentation Modules. `Workbench` becomes composition: select, focus, invoke a mutation, and render. `App` becomes host/session composition: status, session list, subscription, and recovery.

The selected design detects existing extension panes and replaces their renderer without extension IPC. This cleanup does not implement that feature. The future replacement surface can consume `projectedPaneRect` and the same focus/mutation Interfaces. A new Cockpit-created pane can receive launch env; Herdr/TUI-created terminals cannot be guaranteed to inherit it.

Acceptance: a keybinding change is confined to `keymap.ts`; a pane render/rectangle rule is confined to `layoutProjection.ts`; a stream/error policy change is confined to `sessionStore.ts` or `mutationCoordinator.ts`. Existing tests and a real adjacent-terminal focus/resize smoke remain green. Dependency: CLEAN-01; can run in parallel with CLEAN-03. Preserve current DTOs and wire behavior.

### CLEAN-03: Rust Herdr protocol, transport, and host Modules

Preserve the current deep `HerdrAdapter` Interface and split implementation detail behind narrower Modules.

- `cockpit-protocol/src/v1.rs` remains the canonical DTO Module. Keep generated TypeScript synchronized through the existing generation path; do not introduce a second schema or a generic message registry.
- In `cockpit-herdr`, extract schema/status validation from `cli.rs` into a focused `capabilities.rs` Module. Keep `HerdrCliConfig` and precedence in `config.rs`. Keep request ID allocation, socket framing, response demultiplexing, and reconnect in `transport.rs`; keep Herdr method names and parameter mapping in `operations.rs`. `terminal_wire.rs` remains a separate terminal surface Adapter; its current `EndpointRegistry` at `:63-168` and surface/input state around `:648-1390` should not be mixed with session operations.
- The `HerdrCliAdapter` in `cli.rs` becomes composition over these Modules. The only outward implementation is the `HerdrAdapter` Interface from `cockpit-core`; core does not depend on Herdr internals.
- In `cockpit-host/src/server.rs`, keep `validate_bind`, `validate_static_root`, `build_router`, and route handlers thin. Extract request/resource-ID parsing into `resource_ids.rs` and host error/status conversion into `host_errors.rs` only when the extraction preserves exact HTTP/WebSocket behavior. `src-tauri/src/lib.rs` remains the native Adapter and shares core operations rather than duplicating policy.

Characterization checks must cover required-method capability failure, malformed Herdr responses, request timeout/reconnect, mutation mapping, terminal stream ordering, loopback/Host validation, and resource traversal rejection. Do not add a generic dependency injection layer, configurable protocol DSL, or plugin framework. Acceptance: changing Herdr timeout/retry policy is one transport Module; changing a Herdr method mapping is one operations Module; changing host error policy is one host error Module. Dependency: CLEAN-01; can run in parallel with CLEAN-02. New FND configuration/storage behavior follows this behavior-preserving work.

### CLEAN-04: style, tests, development loop, and CODE_GUIDE map

There is no repository `CODE_GUIDE.md` or equivalent guide in the current tree; `.omp/AGENTS.md` is the only discovered local guidance file. Add a concise `CODE_GUIDE.md` only after CLEAN-01 agrees on ownership. Its map should state:

- protocol DTOs and generated exports have one owner;
- core policy depends on `HerdrAdapter`, never frontend or Herdr concrete details;
- adapters translate external behavior and do not own product policy;
- frontend state transitions go through typed reducers/coordination Interfaces;
- Herdr is authoritative for live workspace/tab/pane/focus/layout state;
- errors retain stable code plus user message and are classified at one owning Module;
- filesystem and process effects are explicit and testable through narrow Interfaces.

Keep formatting/lint commands in package and Cargo scripts rather than inventing a second task runner. The current JS loop is `bun run typecheck`, `bun run test`, and `bun run build`; the Rust loop is `cargo fmt --check`, `cargo check --workspace`, and `cargo test --workspace`. Add focused commands only where they shorten the real loop, and document a disposable Herdr fixture for acceptance tests. Prefer contract fixtures for malformed wire data and one real smoke path for process/layout behavior.

Acceptance: a new contributor can run one documented command set; tests identify the contract they protect; failure output names the owning Module. A render rule, keybinding, or error policy change has one obvious edit location. CLEAN-04 can follow CLEAN-02/03 and should be reviewed before feature stories consume the new seams.

## Delivery shape

Priming milestone: CLEAN-01 map plus the smallest safe extractions from CLEAN-02 and CLEAN-03, with characterization checks and no visual change. Feature work can then proceed through the established FND/CTX/LIFE seams. The later cleanup elective may deepen Modules where repeated changes show poor Locality; it should be cancelled when a proposed extraction only moves lines or increases indirection.

Parallel lanes are frontend (CLEAN-02), Herdr Adapter (CLEAN-03), and guide/test loop (CLEAN-04) after CLEAN-01. One integration owner updates protocol exports, generated TypeScript, client Interfaces, host/native composition, manifests, and lockfiles. Integration is complete only when typecheck, Rust checks, existing tests, characterization checks, and the disposable real smoke all pass.


## Concrete style and test cleanup

Audit `src/app/styles.css` against the actual component tree. Consolidate repeated typography/state/spacing definitions into existing semantic variables, group rules by current owning view, and remove a rule only after proving it unused by native and browser rendering. Do not re-theme the application during extraction. Keep the terminal canvas, image layering, cell metrics, hover borders, and focused-pane geometry unchanged in before/after screenshots.

Review tests that assert implementation-shaped exports from `App.tsx`. Move their imports to the owning module and retain behavioral assertions. Replace brittle source-string checks only when a behavior-level test covers the same regression. Do not dilute the existing protocol fixtures or remove difficult tests merely because extraction makes them inconvenient. No new general test harness is required.

A bounded priming target is CLEAN-01 plus focus/keymap/layout ownership from CLEAN-02, request/method mapping separation from CLEAN-03, and the code map/token audit from CLEAN-04. Timebox discovery to a focused increment, estimated 3-6 person-days including runtime proof. Defer extra terminal-wire decomposition until an actual feature or defect benefits from it. Re-estimate from the baseline rather than promising a line-count reduction.

## Personal tweak map to publish in CODE_GUIDE.md

| Desired change | Intended owning module | Short verification |
|---|---|---|
| Change a GUI shortcut or mouse action | `src/app/input/keymap.ts` and action dispatch | Keymap/focus tests plus one focused GUI/terminal check |
| Change when a pane gets a GUI replacement | `core/extensions/detect.rs` after PANE-01 | Detection fixtures; same-title normal terminal remains terminal |
| Change Markdown width or selected-line color | Shared semantic tokens plus Context style module | Normal/minimum-size screenshots |
| Change reference text format | `core/comments/format.rs` after REF-01 | Golden payload fixtures including Unicode/deleted lines |
| Change same-tab target default | Comment target policy module | Multi-agent/tab-move fixture; no broad UI edits |
| Change included local snapshot files | `core/context/snapshot.rs` policy | Dirty/untracked/ignored/isolation fixtures |
| Change source traversal defaults | `core/sources/hydration.rs` plus config schema | Bounded graph/cycle tests |
| Change Herdr operation mapping | `cockpit-herdr/operations.rs` | Request/response contract fixture |
| Change an error's classification | Owning core operation module | Native/browser error equivalence fixture |

The guide distinguishes current modules from planned ones until they exist. A contributor should not have to read every host and frontend view to change one policy. Module extraction is successful when a real representative tweak is local and behavior-preserving, not when a file falls below an arbitrary length.

## Optional follow-up cleanup

After M3 or the graphical review increment, inspect the diffs for repeated policy, awkward interfaces, and excessive caller knowledge. Select only concrete repeated pain points. Keep this as a separate cleanup story/commit rather than burying it inside a provider or UI feature. It may deepen file/review modules, consolidate duplicate adapters, or simplify operation state, but does not introduce a generic extension SDK or a product settings language.
