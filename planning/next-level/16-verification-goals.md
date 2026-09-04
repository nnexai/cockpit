# Verification goals

Status: proposed acceptance plan, prepared 2026-09-04 against the next-level plan index. This file defines evidence to collect after implementation. It does not add application code, install tools, activate a live configuration, or authorize a user-session run.

The goals below follow the authoritative [execution timeline](15-execution-timeline.md), [quality gates](11-quality-gates.md), [viewer and reference comments](04-viewer-and-reference-comments.md), [extension panes](07-extension-panes.md), and [terminal stability](13-terminal-stability.md). The default verification scope is the main loop, `REV-01/02`, and `SRC-04/05`. `PANE-03` and `SRC-06` stay parked unless a later scope decision selects them. Optional `OPT-*` stories are excluded.

## Evidence rules

The orchestrator’s Herdr `default` session is protected. Every runtime script must verify its explicit session against the run-owned resource ledger and reject `default`, missing targets, and inherited ambiguous targets before effects. Never restart, upgrade/downgrade, close, or send test input to the protected session.

Every run records the commit, base ref, host, client type, Herdr executable path and hash, Herdr version, protocol/schema, generated client revision, renderer/addon versions, display scale, fixture revision, and exact argv. A disposable named Herdr session and temporary Git roots are mandatory for runtime checks. Automated mouse and keyboard input may target only that session. No test drives the user's active agent, terminal, or browser tab.

Each goal reports `PASS`, `FAIL`, or `INCONCLUSIVE`. `FAIL` means the required environment was present and an assertion failed. Missing native/WebKit support, a missing provider executable, unavailable process inspection, missing configured credentials, a missing compatible Herdr pair, or an interrupted run is `INCONCLUSIVE`, never `PASS`. A provider credential failure is recorded as an external block with the provider and operation named; it is not replaced by a fixture success.

The proposed command names in this file are future verification contracts. They are not claims that the scripts or binaries already exist. Implementations must use the repository's pinned tools and record their actual versions. A static HTML mock, helper-only unit test, CRAP score, or final screenshot cannot satisfy a runtime goal by itself.

## G00A. Read-only startup inventory

The user downgrades Herdr `default` before execution; Astra runs inside it. Record stable executable/server version, hash and schema through bounded read-only calls, repository/source baseline, protected user work, tool inventory, and run-owned session naming policy. The current frontend is expected to be incompatible. PASS requires complete inventory and guards that reject default/unspecified test sessions. No frontend smoke, old protocol-22 server, or default-session restart is a prerequisite. Missing stable identity is INCONCLUSIVE; mutating the protected session is FAIL.

## G00B. Stable protocol bootstrap and environment identity

**Inputs.** A fixed Cockpit commit, the verified stable Herdr client/server pair selected as the default target, one disposable repository, one disposable session, the native host where available, Chromium, and WebKit/Tauri where available. Record protocol/schema compatibility and retain the committed protocol-22 build (`7e8fe25`) as comparison history. Use a fixed fixture command that emits text and, only where the selected pair supports it, one stable image.

**Scenario.** After BOOT-01 changes the compatibility gate AND actual transport/decoder to the installed stable contract, a future `scripts/verify/startup-inventory` contract launches each available host against a disposable config, performs the protocol/client-shell handshake, opens the fixture pane, and writes the inventory before feature checks. It does not mutate the user's config or attach to an existing session.

**Pass/fail.** `PASS` requires one complete JSON inventory per host with all identity fields, capability states, fixture IDs, and session/resource IDs, plus successful snapshot retrieval, visible fixture output, and byte-exact basic input to a harmless capture program. The native and browser inventories must agree on protocol/schema and effective non-secret configuration, and must report mouse input, click/focus, app-mode pointer coordinates, wheel/scroll, and any graphics capability explicitly. A missing required pair or host is `INCONCLUSIVE`; an identity mismatch or silently disabled capability is `FAIL`.

**Artifacts.** Redacted inventory JSON, handshake transcript, capability report, disposable resource manifest, and cleanup result. Secrets and raw process arguments are omitted.

## G01. Temporal terminal stability and scrolling

**Inputs.** The exact stable default build pair from G00B, native and Chromium clients, a text-only control, and, when the pair advertises it, a graphics fixture that keeps a high-contrast marker and image stable while text updates. Run idle, text-only, image-plus-text, image replacement/removal, resize, focus change, two panes, agent-like output, and `/pets`-shaped bursts. Exercise mouse clicks, focus transitions, app-mode pointer coordinates, short wheel steps, and a sustained gesture while at tail and away from tail. Active Kitty TGP is a desired future capability and is not a requirement for this stable default.

**Scenario.** A future `scripts/verify/terminal-temporal` contract captures displayed frames or video from the real renderer at 60 displayed frames per second or the measured display refresh rate, with at least 30 seconds per workload and 100 scroll input samples per client/run, while also recording surface/graphics events, input timestamps, pointer coordinates, click/focus results, intended scroll position, and visible scroll position. Replay the same bounded fixture at least three times per client on the stable pair. Compare the stable default against the pre-protocol-22 mouse evidence from commits `34459ab` and `582792e` only as source/comparison evidence, not as a compatibility claim. Preserve protocol-22 behavior in `7e8fe25` as a separately labeled experimental comparison.

**Pass/fail.** `PASS` requires zero detected unintended blank/dark intervals lasting one sampled frame or longer, zero loss of the persistent marker, zero unexpected viewport movement while the user is away from tail, no automatic tail-follow after a deliberate scroll-back, and successful click/focus, pointer-coordinate, and wheel/scroll assertions in both native and browser clients. Input-to-visible-scroll p95 must be no worse than 1.25 times the paired text-only control and at most 100 ms, reported with sample count. These are initial acceptance budgets, not measured current performance. Changing a budget requires a recorded measurement-based decision, never merely a failing result. If TGP is unsupported by the stable pair, text-only stability and all advertised mouse/scroll capabilities still must pass, with `desired_kitty_tgp: parked` recorded. A run fails if any advertised control or supported graphics variant violates the rule. If displayed-frame capture or native compositor observation is unavailable, report `INCONCLUSIVE`. An old protocol-22 comparison pair is optional; its absence cannot block bootstrap or successful stable-target verification.

**Negative controls.** A final screenshot is invalid evidence. The detector must reject a fixture with a deliberately blanked stable-marker frame and a 100 ms blank interval. Record dropped capture frames; inadequate capture makes the temporal verdict inconclusive. The report must include successive-frame data, a stable-marker control, text-only replay, a known idle run, a click/focus control, and a wheel/scroll control. Canvas pixels alone do not count when compositor capture is available. Unsupported TGP must be labeled parked, never silently treated as passed graphics. The report also counts full resets, image deletions, unchanged image retransmissions, and viewport jumps, but these counters are diagnostic unless correlated with a visible failure.

**Artifacts.** Bounded video/frame samples, event trace, scroll trace, paired-control statistics, reset/image lifecycle counts, exact build identities, and a reproduction/minimization report. Do not include real user output.

## G02. Existing correctness repairs

**Inputs.** Fixtures for stale snapshots, generation transitions, out-of-order events, duplicate sequence numbers, pane attachment changes, reconnect, request timeout after dispatch, focus loss, and process replacement. Use the repaired code's characterization baseline from CLEAN-01.

**Scenario.** A future `scripts/verify/correctness-fixtures` contract feeds each event/request trace through the core reducer and transport coordinator, then runs the corresponding disposable-session smoke. Mutations are attempted only against resources created by the fixture.

**Pass/fail.** `PASS` requires stale or wrong-generation data to be rejected or marked stale, duplicate/out-of-order data to be rejected deterministically, attachment changes to invalidate old input targets, and post-dispatch timeout to become outcome-unknown without automatic mutation retry. Reconnect must invalidate stale live attachment/control evidence and require fresh identity. Durable drafts are added and tested later under G06, not a dependency of this pre-feature gate. Any accepted stale mutation, duplicate listener, blind retry, or stale attachment reuse is `FAIL`; unavailable runtime confirmation is `INCONCLUSIVE`.

**Artifacts.** Input traces, reducer transition logs, request/attachment state logs, expected/actual verdicts, and disposable-session cleanup report.

## G03. Cleanup and deterministic quality gate

**Inputs.** CLEAN-01 behavior baseline, fixed comparison base, changed tracked/untracked source, reviewed legacy baseline, pinned tool manifest, and deliberately bad fixtures for complexity, coverage, mapping, and mutation outcomes.

**Scenario.** Future `bun run quality:report`, `bun run quality:gate`, and `bun run quality:baseline` contracts are exercised only after their providers are probed. Run the ordinary repository checks and the changed-scope report. Verify report freshness by changing a scoped source or test hash between collection and gate.

**Pass/fail.** `PASS` requires ordinary checks to pass, new-function CRAP `<=8`, new logic coverage `>=90%`, no new surviving/no-coverage mutant, no changed legacy regression, and complete source-to-function joins. Exit `1` is a hard violation, `2` is strict inconclusive for missing mapping/provider evidence, and `3` is invalid setup/infrastructure. Deliberate bad fixtures must fail with the correct status. Missing quality providers are `INCONCLUSIVE`, never an empty green report.

**Artifacts.** Versioned JSON report, human report, baseline diff, command argv/status/duration, method and mutant rows, policy hash, source/test hashes, and bad-fixture results. No report may write the baseline without explicit `--write`.

## G04. Shared contracts, identity, and storage safety

**Inputs.** Generated Rust/TypeScript DTOs, malformed JSON, roots with spaces, absolute and traversal paths, symlink escapes, repository replacements, concurrent host writers, operation journals, and protocol compatibility fixtures.

**Scenario.** A future `scripts/verify/contracts` contract round-trips every DTO implemented in the current stage; repeat cumulatively for later main/REV/SRC04/05 DTO additions through browser and native adapters, then exercises containment, atomic replacement, compare-and-swap generations, lock contention, cancellation, reconnect, and malformed-input paths.

**Pass/fail.** `PASS` requires byte-equivalent valid round trips, identical error envelopes across hosts, rejection of unsafe paths and unknown fields where specified, no symlink traversal, no lost concurrent write, and explicit `accepted`, `rejected`, or `unknown` mutation outcomes. An operation journal must preserve completed effects and never authorize a foreign resource. Any unrestricted absolute read, silent coercion, or host-specific contract drift is `FAIL`; unavailable native adapter evidence is `INCONCLUSIVE`.

**Artifacts.** Protocol round-trip corpus, generated-file drift result, malformed-input matrix, filesystem tree before/after, lock/CAS trace, operation journal, and host comparison report.

## G05. Local setup, lifecycle, and Context loop

**Inputs.** Disposable local Git repository, optional typed issue URL, temporary configured roots, Herdr session, an authored Markdown/Mermaid file, an agent pane, and a second terminal pane. Provider availability is not required.

**Scenario.** A future `scripts/verify/local-context-loop` contract selects the repository, plans and creates a worktree, verifies the returned Herdr workspace through snapshot/list, creates the companion, creates a context-bearing pane with `COCKPIT_*` variables, browses bounded files, switches Context/Terminal/Context, reconnects, repeats open, and removes only test-owned resources. Use automated click, wheel, key, focus, and pane-move input only in the disposable same-tab session.

**Pass/fail.** `PASS` requires exact repository/worktree provenance, `already_open` reuse, durable partial recovery, safe teardown, readable original lines, Mermaid failure isolation, and environment visibility in Cockpit-created processes. The Herdr-created initial root pane must remain visible when env injection cannot retrofit it. Missing Herdr lifecycle capability or native runtime is `INCONCLUSIVE`; deleting a foreign/ambiguous resource, losing owned context, or claiming env inheritance for an existing process is `FAIL`.

**Artifacts.** Setup plan/result, Herdr snapshots/lists, operation journal, companion manifest/tree, screenshots or recordings of actual WebKit rendering, input/focus trace, and owned-resource cleanup proof.

## G06. Reference comments and paste-only delivery

**Inputs.** Two context files plus a checkout file, whole-file and selected-line comments, changed/deleted source revisions, multiline UTF-8 text, control-character and oversize payloads, two agents in one disposable tab, and a harmless input-capture program.

**Scenario.** A future `scripts/verify/reference-delivery` contract selects lines with mouse and keyboard, edits from file and overview views, reloads, changes focus and panes, reconnects, validates the same-tab target, and sends bracketed paste without Enter. It records the target pane, focus confirmation, bytes dispatched, and Herdr receipt.

**Pass/fail.** `PASS` requires exact path and numbered original excerpts, stable draft IDs, correct UTF-8 byte bounds, control/terminator rejection, no submission key or extra CR/LF after closing paste framing, while preserving source newlines inside the payload, accepted bytes removed from unsent drafts, rejection preserving drafts, and timeout/disconnect marked unknown without retry. Targets outside the actual tab or current agent set, and the hidden extension TUI, must each receive zero bytes. Missing paste acknowledgement or unsupported agent mode is `INCONCLUSIVE` with copy/preview retained; accidental submission, wrong-tab delivery, or silent sanitization is `FAIL`.

**Artifacts.** Draft and batch JSON before/after, byte-exact preview and capture output, focus/target trace, receipt/error classification, and no-Enter assertion. Never automate a live user session.

## G07. Source snapshots and provider hydration

**Inputs.** A configured read-only provider fixture for issue/comments, one or more local reference repositories, pinned revisions and dirty states, reflink-capable and copy-only roots, pagination, transient failure, auth-required, oversized, and edited-user-file cases.

**Scenario.** A future `scripts/verify/source-hydration` contract adds and refreshes sources, follows bounded pagination, materializes independent snapshots, resumes after interruption, and compares old/new cache generations. Run both hosts against hermetic fixtures without real credentials. Separately require one read-only hydration from the configured real provider; unavailable credentials leave that required live subcheck inconclusive.

**Pass/fail.** `PASS` requires provider identity to include instance and resource identity, bounded retries/cancellation, retained old snapshots on failure, explicit freshness, no hardlinks or Git alternates, copy isolation after source edits, and user files preserved across refresh. Missing provider executable or credentials is `INCONCLUSIVE` and names the blocked operation. A direct unconfigured credential path, silent truncation, shared writable inode, or overwritten user file is `FAIL`.

**Artifacts.** Provider capability/error report, request pagination trace with secrets redacted, manifests/cache hashes, copy-mode and inode/isolation evidence, freshness report, and failure/recovery journal.

## G08. Complete local graphical Review replacement

**Inputs.** Disposable Git fixture covering staged, unstaged, partially staged, untracked, renamed, deleted, binary, mode-only, submodule, detached, and empty-HEAD cases; a detected Reviewr pane; Context comment fixtures; and two terminal neighbors.

**Scenario.** A future `scripts/verify/review-replacement` contract detects Reviewr through verified process evidence or explicit pane override, renders the full local diff, expands bounded hunks, selects old/new/deleted lines, adds/edits/removes comments across three files, switches Terminal/Review/Terminal, moves/resizes the pane through Herdr, reconnects, and paste-delivers through G06 in the same tab.

**Pass/fail.** `PASS` requires correct immutable revisions and side-specific line numbers, unchanged Git/index contents, safe summaries for unsupported files, preserved drafts across renderer switches, no hidden xterm input path, no extension IPC, and exact paste-only delivery. A same-title ordinary terminal must remain terminal. Missing process inspection uses known-launch evidence or explicit override; the automatic pre-existing-pane subcheck is NOT_APPLICABLE with capability evidence, while the required safe replacement/false-match tests still run; wrong replacement, silently remapped anchors, or any Git mutation is `FAIL`.

**Artifacts.** Review snapshots and hashes, diff/anchor corpus, pane detection evidence, actual browser/native recordings, Herdr geometry/focus trace, draft/receipt records, and Git before/after hash report.

## G09. Review and wiki source imports

**Inputs.** Configured read-only fixtures for `SRC-04` review context and `SRC-05` wiki pages, including pagination, replies, deleted/edited content, unsupported markup, revision changes, auth-required, rate-limit, and unavailable-provider cases. Local repository/revision matching remains required.

**Scenario.** A future `scripts/verify/review-wiki-imports` contract resolves each typed source, checks its local repository/revision match, imports bounded normalized documents, preserves provenance and freshness, and exposes them through the Context viewer. Run the fixture corpus through native and browser adapters. Separately require one configured read-only real import for each selected provider operation; fixture success cannot replace missing live evidence.

**Pass/fail.** `PASS` requires separate source identities, deterministic ordering, preserved source URLs and revision metadata, bounded unsupported-content status, no remote mutation, and retained prior data after refresh failure. Missing credentials or provider capability is `INCONCLUSIVE`, with no fabricated import. A mismatched local revision used for anchors, silent dropped replies/pages, or a write request is `FAIL`.

**Artifacts.** Normalized import fixtures, provenance/freshness manifests, pagination/error logs, local-match decision, rendered source-line map, and host-equivalence report.

## G10. Release candidate readiness

**Inputs.** A fixed clean candidate commit, all required G00A/G00B and G01–G09 reports, the selected stable Herdr pair as the default, pinned toolchain, build outputs, generated protocol files, and a disposable installation/config/session. The committed protocol-22 build remains an explicitly labeled comparison/experimental artifact.

**Scenario.** A future `scripts/verify/release-candidate` contract checks version/changelog consistency, source tree and generated-file state, format/type/test/build gates, Tauri compile and native/browser runtime smokes, temporal evidence, cleanup, and report provenance. It packages to a temporary output directory only. Publishing, tagging, or changing the user's active installation requires a separate authorized release action.

**Pass/fail.** `PASS` requires every in-scope goal to be `PASS`, all selected required feature behaviors to pass on the stable target, using documented capability fallbacks where allowed, with no unresolved `FAIL`, no required goal hidden behind a skipped provider, reproducible candidate identity, clean owned resources, and explicit parked scope for active Kitty TGP, `PANE-03`, `SRC-06`, and `OPT-*`. Any required goal `INCONCLUSIVE` keeps the candidate `INCONCLUSIVE`; build success without native/runtime evidence cannot pass. Version or generated drift, residual test resources, or a stale report is `FAIL`.

**Artifacts.** Candidate manifest with hashes, version/changelog check, complete goal index, quality and temporal reports, build logs, browser/native smoke evidence, resource cleanup proof, and a short list of external blocks. The release-candidate report is the handoff artifact, not a publication record.
