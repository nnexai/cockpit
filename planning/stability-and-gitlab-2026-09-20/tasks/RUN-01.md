# RUN-01 — Establish safe runtime and evidence baseline

Status, dependencies, owner, locks and completion proof: [task ledger](../tasks.json). Follow the [orchestrator contract](../ORCHESTRATOR.md).

## Outcome

A reproducible, isolated campaign baseline with exact runtime identities, protected resources, platform/fixture availability and an evidence destination. No compatibility fix is required to finish this inventory task.

## Evidence and starting points

Inventory baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`, initially clean. Read [INVENTORY](../INVENTORY.md), repository authorities and the latest inline handoff. Existing helpers: `scripts/verify/resource_guard.py`, `startup_inventory.py`, `terminal_temporal.py`, `temporal_detector.py`, and `ui_polish_runtime.py`. Inspect their current CLI/schema before use: historical harnesses can encode obsolete runtime/native packaging assumptions.

## Changes

1. Record current commit, protected pre-existing changes, executable paths/versions, Herdr protocol/schema/capabilities, Bun/Node/glab/Playwright configuration and host/native build provenance. Never capture tokens or whole credential files.
2. Create a unique run ID, a run-owned evidence directory, and a resource ledger. Allocate explicit named Herdr session, config/state roots, fixture Git repositories, socket, ports and browser profile; inspect installed CLI help before creation. Refuse default/active sessions.
3. Record Linux browser/native and macOS access independently. Discover available authorized macOS execution capabilities; do not infer one exists or silently omit macOS tasks.
4. Read GitLab fixture issue #1 in project 86672117 and record identity/type/marker without changing it. Record MR fixture absence or an explicitly authorized MR. Obtain separate permission before creating/pushing a branch or MR.
5. Establish a disposable Herdr TUI oracle for selection, ordering, focus, mouse and scrolling. A Cockpit version rejection is a baseline finding for TERM-01, not a reason to downgrade Herdr.
6. Freeze fixture sizes, sample definitions and performance budgets for PERF-01 before repair measurements. Preserve observed limitations rather than calling them pass. Record safe launch/cleanup commands and which artifacts must survive teardown.

## Non-goals

No product repairs, live installation, user-session mutation, credential management, remote branch/MR writes, or new verification framework. Do not rerun user failures merely to decide whether they are real. Historical custom graphics migration is not part of this campaign.

## Acceptance

1. A second orchestrator can identify the exact source/runtime and launch a separate fixture without guessing session/config paths.
2. Resource guard proves each test target is owned and not the protected session; cleanup ownership is explicit.
3. Every missing platform/fixture has a precise discoverable prerequisite and affected task, not a global stop.
4. Compatibility rejection, unsupported capabilities and unrun scenarios are recorded distinctly.
5. GitLab fixture identity and production read-only policy are recorded; no credentials are in evidence.
6. Terminal oracle, workloads, sampling method and proposed performance budgets are recorded before repair work.

## Verification

Run bounded read-only CLI/schema/config probes and exercise only the created disposable TUI resources. Verify one harmless fixture command cannot address default implicitly. Inspect evidence for secrets and stale hard-coded historical paths. Reuse helpers only where their contracts match the current build. No project-wide suite is needed for an inventory-only task.

## Handoff

Commit a compact `runs/<run-id>/RUN-01.md` with resource/evidence identities, baseline limitations, performance protocol and platform/fixture blockers. Record its commit and evidence path in the central ledger. Clean up short-lived probe resources or explicitly assign retained fixtures to the next task with a final cleanup owner.
