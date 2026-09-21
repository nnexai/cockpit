# WEB-07 — browser launch prerequisites

## Accepted implementation plan

- Run: `run-20260920-a3e9b950`; orchestrator/integration owner: Main (Astra). Accepted 2026-09-21 against `6593cf4ecba87687c809ee9f3d2a295c757db787`.
- RUN-01 is complete with the mandatory guarded-HOME correction. OBS-008 restoration/retention is still unresolved and is not permission to touch protected files.
- Locks: config-host and browser-runtime-launch. Concurrent SYNC-01 owns frontend/client cancellation and host stream registry, not these launch files. Main preserves the unfinished VIEW-01 changes.

## Observable outcome and design

Browser launch diagnoses the specific missing, invalid, incompatible or unimportable prerequisite without breaking ordinary terminal use. The packaged helper works from a clean owned installed prefix. Existing ownership/profile/endpoint/origin/sandbox checks remain authoritative; never launch another browser or use a fallback owner to hide an error.

Inspected baseline already accepts CLI 0.1.x >=0.1.5 in `compatible_playwright_cli_version`; preserve it, do not gratuitously rewrite it. `resolve_executable` currently collapses missing and non-executable cases, paired-core validation checks only package.json existence, helper spawn/materialization share generic errors, stderr is discarded, and the helper imports the selected package at attach. Tighten these existing boundaries rather than add a generic dependency resolver.

Use the existing environment/TOML configuration precedence. Browser-only lazy preflight should name the exact setting and remedy, distinguish missing/path type/permission/wrong package/unsupported version or capability, and remain outside global terminal/startup validation. Resolve configured Node and helper paths through existing path validation; preserve the paired-package relationship to the selected CLI. Validate package identity/entry points rather than accepting any package.json. Existing but unimportable package gets its own bounded structured failure, not a missing-package diagnosis. Helper materialization, spawn/readiness failure and runtime crash remain distinguishable.

Keep the embedded helper materialization path deterministic under the owned host state root. Resolve helper sibling assets and paired modules without source-tree cwd assumptions. Preserve cleanup on every partial launch failure. Use existing structured helper error/host InspectionError boundaries for actionable browser-local codes/messages. Expose bounded provenance through the existing diagnostic/evidence channel; never add an unrestricted debug dump or leak profile/CDP/socket/ticket/credential data. Protocol/capability negotiation, not patch number, determines facility support. No package manager, automatic install, alternate owner or extension route.

## Exclusive writing contract

Worker owns `crates/cockpit-core/src/config.rs` browser-setting validation only, `crates/cockpit-core/src/browser.rs` dependency/path/version/attachment launch sections, `crates/cockpit-host/src/browser_helper.rs` dependency launch/materialization/readiness/cleanup sections, `browser-runtime/browser-helper.mjs` import/startup/capability errors, and directly affected existing focused tests. Leave unrelated browser input/rendering/feedback behavior unchanged. Existing source docs may be updated for configuration remedies; no unrelated new documentation. Do not edit frontend/client/App, shared generated protocol, native stream registry, resource_guard, task ledger or other run plans. Shared-interface changes outside this ownership need Main integration. Rust LSP is unavailable in this session; use scoped reference searches where necessary.

## Bounded recipe and gates

1. Read current config precedence, packaging handoff and affected launch/error paths. Preserve currently correct range/ownership behavior.
2. Add precise validation/import/materialization/crash diagnostics and repair source-tree assumptions. Add only behavior regressions for genuinely uncertain boundaries; no tests pinning incidental prose.
3. Worker skips all builds, tests, formatters, services, runtime mutation and commits. Return changed files, compatibility/provenance decisions and proposed smallest checks.
4. Required read-only packaging/configuration review covers terminal independence, clean-prefix behavior, cleanup, redaction and version/capability errors.
5. Main integrates the writing wave once. Per OBS-011, use one brief browser-first launch/failure smoke during implementation; defer exhaustive config/platform matrix to the end-of-wave/final campaign acceptance. Do not claim full acceptance from the brief smoke or a source-tree build.

## Original criterion coverage (unchanged)

1. Isolated omitted/invalid CLI, Node, paired core, helper, executable and permission configurations each show an exact actionable browser diagnostic; positive terminal typing remains usable.
2. Clean owned installed prefix with packaged helper/modules reaches Live browser view in gateway and Linux Tauri, without repository imports.
3. CLI 0.1.5, current supported bugfix and deliberate protocol/capability mismatch follow range/capability policy; no exact-patch rejection.
4. Negative launches leave no partial helper/socket, second Chromium/profile, alert storm or orphan; record owned process cleanup.
5. Browser absent configuration leaves Herdr/terminal startup and typing operational; repair and retry browser only.
6. Diagnostics redact credentials, tickets, private profile/socket data and bound subprocess details.
7. Distinguish missing package versus failed import, materialization failure versus runtime crash.
8. Record stable packaged helper path/content from source and installed-prefix launches, not source import inference.
9. Scoped retry preserves Herdr state, terminal scroll/control and unrelated panes.
10. Record bounded resolved CLI/core/helper provenance with safe package/version/path metadata.
11. Unsupported capability names the facility and preserves explicit cancel/retry without downgrade or owner substitution.

## Resources, authorization and stop boundaries

Main owns all runtime and creates isolated config/install-case roots under `/tmp/csg-a3e9b950` only, with guarded HOME. Never modify global Node/CLI packages, installed binaries, user profiles or default Herdr. Read installed package metadata when necessary, but copy required packaged artifacts into the owned test prefix for proof. CLI on this host is currently 0.1.19; its presence is not packaged-launch acceptance. No remote installation, provider writes or protected branch changes. Any helper security/ownership relaxation or required shared DTO change is an escalation, not worker discretion.

## Status

Plan accepted; implementation and all runtime acceptance remain unverified. Sole task status is `tasks.json`. No completion or commit claim.
