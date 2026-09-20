# WEB-07 — Make browser dependency failures actionable

## Outcome

Make missing Node, Playwright CLI, paired `playwright-core`, helper module, executable, version, or capability prerequisites fail with precise actionable diagnostics before/at launch, while a correctly packaged helper works outside the source tree. Apply the issue #3 compatibility policy: accept Playwright CLI 0.1.x bugfix releases at or above 0.1.5 without imposing an exact patch gate or conflating protocol/schema capabilities. Ordinary terminal startup and use remain unaffected when browser prerequisites are absent.

## Evidence and starting points

- Baseline: `6f6222b74e4f552ce697e61364cf653f4b6be29f`; issue #9 finding 6 is a historical disposable configuration report.
- Issues: https://github.com/nnexai/cockpit/issues/9 and https://github.com/nnexai/cockpit/issues/3.
- Read `planning/inline-space-browser-2026-09-13/HANDOFF.md` limits and architecture feasibility/packaging sections; do not restore old extension runtime or developer-machine assumptions.
- Source starts: `crates/cockpit-core/src/config.rs` browser path fields, environment/TOML precedence, and validation; `crates/cockpit-core/src/browser.rs` required CLI version, executable/core/helper resolution, bounded CLI commands, and attachment identity.
- Source starts: `crates/cockpit-host/src/browser_helper.rs` paired-core/node/helper launch, packaged materialization, readiness, and `browser_helper_unavailable` errors.
- Source starts: `browser-runtime/browser-helper.mjs` `importPlaywright`, `importWebSocket`, capability reporting, and startup command handling.
- Issue #3's version policy is advisory history; verify current protocol/schema and capability negotiation rather than copy a patch-version gate.
- Dependencies/locks and status remain in `planning/stability-and-gitlab-2026-09-20/tasks.json`; this task owns `config-host` and `browser-runtime-launch` only.

## Changes

1. Validate configured CLI, Node, paired core, helper module, and executable paths with safe existence/type/permission diagnostics. Name the exact missing setting and supported configuration source without exposing secrets.
2. Distinguish “not configured,” “not executable,” “wrong package,” “unsupported capability/version,” and “helper startup/crash.” Return a browser-local actionable state while leaving the rest of Cockpit and ordinary terminal attach usable.
3. Verify Playwright CLI version as a supported 0.1.x bugfix range at or above 0.1.5; do not reject a newer bugfix solely for patch mismatch. Keep protocol/schema/capability negotiation authoritative for incompatible behavior.
4. Make packaged helper and paired modules discoverable from a clean run-owned install/config root, not only repository source paths or a developer’s global Node installation.
5. Keep browser launch ownership/profile/endpoint verification intact; a diagnostic must not silently launch a second browser or bypass sandbox/origin policy.
6. Add a required packaging/configuration review covering ordinary terminal unaffected, clean-prefix helper availability, and version/capability error wording.

## Non-goals

- No new package manager, credential store, remote install, generic dependency resolver, or extension runtime.
- No exact stale patch-version requirement, protocol downgrade, or automatic browser fallback that changes ownership.
- No changes to terminal behavior when the browser is disabled or unconfigured.
- No claim that a successful source-tree launch proves packaged availability.

## Acceptance

1. With each browser prerequisite omitted or invalid (CLI, Node, paired core, helper, executable/permission), opening browser shows a specific actionable diagnostic naming the missing capability and configured remedy; terminal open/typing still works.
2. With a clean run-owned installation/config root containing the packaged helper and paired modules, browser gateway and Linux Tauri reach `Live browser view` on a disposable profile without source-tree imports.
3. CLI 0.1.5, current supported 0.1.x bugfix, and a capability/protocol mismatch produce the intended accept/refuse diagnostics; a newer bugfix is not rejected by an exact patch gate.
4. Missing browser prerequisites do not create a partial helper, stale socket, second Chromium/profile, terminal alert storm, or orphaned process; resource cleanup is recorded.
5. Ordinary terminal and Herdr session startup remain usable with browser configuration absent, while the browser pane offers retry after repair without restarting unrelated panes.
6. Diagnostics do not print credentials, profile secrets, frame tickets, raw socket paths, or unbounded subprocess output.

7. Launch diagnostics must distinguish a missing paired package from a package that exists but cannot be imported, and distinguish helper materialization failure from helper runtime crash.
8. Verify the packaged helper path is stable across source-tree and installed-prefix launches; a successful source-tree import is not evidence for the installed artifact.
9. Keep browser retry scoped to the browser association. Repairing a missing dependency must not reset Herdr session state, terminal scroll, terminal ownership, or unrelated panes.
10. Report the resolved CLI/core/helper provenance in bounded diagnostic metadata suitable for evidence, while redacting credentials and profile/ticket material.
11. Capability refusal must name the unsupported facility and preserve an explicit cancel/retry path; it must not silently downgrade to a different browser owner or old extension route.

The compatibility decision must be recorded against the current protocol/schema and capability exchange. Historical issue patches and exact patch-version assumptions are evidence to review, not instructions to replay.

## Verification

The integration owner must test a matrix of isolated config roots and executable/package states on browser gateway and Linux-native Tauri, plus an ordinary terminal control run. Record versions, capability negotiation, resolved package/helper provenance, exact user-visible diagnostics, process/socket cleanup, and clean-prefix paths. Use `node --check`/build/type checks only as supporting gates; packaged launch and unaffected terminal behavior are the acceptance proof.

The evidence record must include:

- each missing/invalid dependency configuration, resolved source, exact diagnostic, and ordinary-terminal control result;
- clean-prefix packaged helper/core provenance, CLI version/capability exchange, and browser/native launch result;
- process, socket, profile, and partial-start cleanup for every negative case;
- confirmation that diagnostics redact credentials, tickets, profile secrets, and unbounded subprocess output.

Do not mark packaging complete from a source-tree run or a type check; the helper must be discoverable and runnable from the intended installed/configured surface.

Record the ordinary terminal control before and after browser prerequisite failures so unaffected behavior is observable, not inferred.

The evidence must preserve the exact installed-prefix paths and package versions without including private home-directory secrets.

## Handoff

Return changed paths, prerequisite matrix, compatibility decision, and unresolved packaging/platform risks. The integration owner must write durable `runs/<run-id>/WEB-07.md` evidence and land a real commit. Keep `config-host` and `browser-runtime-launch` serialized with any shared launch changes; status/dependencies remain solely in `../tasks.json`.
