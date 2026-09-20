# RUN-01 — isolated campaign baseline

## Identity and ownership

Run `run-20260920-a3e9b950`; integration/planning owner Main (Astra, openai-codex/gpt-6-astra). Source baseline `a3e9b9504339f128e14c21a8a24b40a7b8d79953`. Initial `git status --short --branch` reported clean main, three commits ahead of origin/main. No pre-existing dirty files. Product baseline in the original inventory remains `6f6222b74e4f552ce697e61364cf653f4b6be29f`; planning commits are not product proof. The matching native goal is active. No running/idle peer orchestrator was discovered. `campaign.py check` passed with 27 records; 25 required pending, two deferred.

## Accepted just-in-time task plan

ACCEPTED by Main/Astra on 2026-09-20 before execution. RUN-01 is the sole dependency root and acquires `verification-runtime`. No product implementation or worker dispatch is authorized by this plan.

- Coverage: one no-code increment covers original criteria 1–6: reproducible runtime identity; guarded ownership and cleanup; platform/fixture discovery; distinct compatibility/capability/unrun findings; GitLab identity/read-only policy; terminal oracle and frozen measurement protocol.
- Authorities inspected: CONTEXT, DECISIONS, CODE_GUIDE, campaign graph/acceptance/observations/inventory/orchestrator/autorun, RUN-01 brief, current inline HANDOFF, resource_guard.py validation/dispatch schema, ui_polish_runtime.py startup. Existing historical fixture launcher is not invoked: it spawns unsupervised services and lacks the current guard ledger. Use resource_guard's existing validated argv/environment contract and Hub-managed processes, without adding a framework.
- Writable scope: this evidence record, run-owned compact resource/evidence files, and tasks.json status fields. Runtime artifacts live under `/tmp/csg-a3e9b950`; never reuse an existing root. Named Herdr session `csg-a3e9b950`, configuration `/tmp/csg-a3e9b950/config/herdr/config.toml`, state `/tmp/csg-a3e9b950/state`, expected socket `/tmp/csg-a3e9b950/config/herdr/sessions/csg-a3e9b950/herdr.sock`; repositories, browser profile and artifacts below the same owned root. Allocate unused loopback ports before future consumers. No live installation or shared configuration edits.
- Recipe: record executable paths/versions and bundled Herdr schema without secrets; discover macOS via configured SSH capabilities/aliases and available runner configuration; GET only the designated GitLab issue and MR list; prepare isolated fixture repositories/config/guard ledger; verify guard positive target plus default/implicit-target refusal; launch guarded headless Herdr then its TUI via Hub; observe a harmless fixture action and authoritative snapshot; stop both owned processes or explicitly retain for TERM-01. Commit compact evidence then checkpoint its SHA in tasks.json.
- Positive checks: owned CLI snapshot and TUI render/command; fixture identity and marker match API. Negative checks: guard refuses default, unknown session and missing selector; source/runtime incompatibility is recorded rather than repaired or hidden. Authentication/API failure must be recorded without tokens. Discovering no macOS runner or MR fixture blocks affected downstream proof, not this inventory task.
- Resources/permissions: only local disposable resources and authenticated GETs to project 86672117. Existing authorized issue 1 marker `cockpit-glab-2026-09-20-a17b` is retained unchanged. No branch/MR creation, pushes, tracker comments/closure or protected main changes. Default/active Herdr, personal browsers, installed apps and pre-existing gateways are protected. An unfamiliar existing runtime root, changed executable hash, ambiguous process ownership or unintended endpoint stops execution and returns to planning.
- Worker choice: inline Astra execution because this is a single bounded inventory task with a shared resource ownership boundary; no padding delegation. Main independently checks actual output against all six criteria. No separate review is required for no product/API change; resource guard is the safety gate. Revalidate source, ownership and resources on recovery; no stale plan dispatch.

## Frozen performance protocol

Before repairs: browser desktop 1440×900 and minimum 1024×640; native actual viewport and scale recorded separately, scale 1.0 baseline. Terminal workload: 10 tabs, 60 switches; 10,000 numbered output lines at 50 lines/second for five minutes including deliberate scroll-away and return-to-tail. Context/Review: 5,000-line bounded document and 200 changed files; resource list 200 entries. Inline page: 2,400 CSS-pixel document, nested 600-pixel scroll region, counter/input, text/region/element targets; DPR 1 and 2, zoom 100% and 125%. Fixed fixtures/build hashes must accompany each measurement.

Warm up 30 seconds; collect 100 click/input samples and 100 metadata samples with monotonic host/browser clocks and explicit clock alignment (do not subtract unrelated clocks). Targets: click-to-visible p95 ≤150ms, metadata p95 ≤250ms; no whole-view blanking or scroll jumps, no hidden terminal subscriptions, no unsolicited hover flood. Sample CPU and PSS once per second during five-minute idle and active runs, report p50/p95, range and least-squares PSS slope; resource counts after warmup and teardown must return to stable baseline rather than grow per switch. Browser-specific existing performance/security matrix remains mandatory; these definitions cannot weaken it. PERF-01 owns final settled-code measurement and must preserve failing samples. No performance result is claimed here.

## Acceptance results

| Criterion | Surface | Result | Observed evidence |
| --- | --- | --- | --- |
| 1 | Local CLI/filesystem | PASS | Exact source/runtime/config/socket identities in this record and baseline.json. Herdr 0.9.1, protocol 22/schema 1; executable pinned by SHA-256. |
| 2 | Guard and owned runtime | PASS | Guard CLI returned ok for csg-a3e9b950; default, empty and unrecorded selectors refused. Hub stopped both owned processes with exit 0. |
| 3 | Platform/provider discovery | PASS | No SSH capability hosts, only gitea SSH alias, no saved Herdr machines or CI runners. macOS unavailable: NATIVE-01/02 and SETUP-02 need an authorized host. MR list empty: real-MR proof requires designated MR or explicit branch/MR fixture authorization. |
| 4 | Real CLI | PASS | Owned-server Cockpit status returned incompatible/version_mismatch: expected Herdr 0.9.0. Live/bundled protocol 22, schema 1. Browser/native interaction and full terminal matrix unrun. |
| 5 | GitLab GET | PASS | Issue id 203687939, iid 1, project 86672117, issue_type issue, opened, canonical work_items/1 URL and matching a17b marker. No mutation or credentials in evidence. |
| 6 | TUI and measurement protocol | PASS | Guarded create returned focused w1/w1:t1/w1:p1 at exact fixture cwd. TUI rendered Campaign oracle, main, tab 1, fish and welcome UI. Runnable oracle established, not full input/mouse/scroll proof. Workload protocol above predates repairs. |

## Checks and diagnosis

Executed installed CLI help/version/schema probes; `herdr api schema --json`; guarded `api snapshot` (initial empty snapshot, protocol 22/version 0.9.1); guarded `workspace create --cwd /tmp/csg-a3e9b950/repositories/sample --label 'Campaign oracle' --focus`; `python3 scripts/verify/resource_guard.py --ledger /tmp/csg-a3e9b950/resources.json --run-id run-20260920-a3e9b950 --session csg-a3e9b950`; `target/debug/cockpit status --herdr /home/linuxbrew/.linuxbrew/Cellar/herdr/0.9.1/bin/herdr --herdr-session csg-a3e9b950 --herdr-socket /tmp/csg-a3e9b950/config/herdr/sessions/csg-a3e9b950/herdr.sock --json`; `herdr machine list --json` returned [].

Authenticated `glab api projects/86672117/issues/1` and `glab api 'projects/86672117/merge_requests?state=all&per_page=20'` succeeded with selected output piped to jq. Installed glab rejected initial --jq; corrected without mutation. Initial guard calls incorrectly passed a path rather than loaded ledger and used unsupported `api snapshot --json`; both failed before dispatch and were corrected to existing contracts.

TUI initially refused nested execution. Clearing inherited selectors did not resolve it; the documented `[experimental] allow_nested = true` was enabled only in the disposable config. TUI then rendered. Enter and harmless printf were sent, but captured output did not confirm the marker; no byte-exact input claim. TERM-03 owns that proof.

Linux-native artifact exists with unknown source provenance until rebuilt. DISPLAY/WAYLAND_DISPLAY unset; /run/user/1000/wayland-1 is an actual socket. No Xvfb/xdotool. Native interaction is unrun. Paired Playwright/playwright-core packages are 1.63.0-alpha-2026-08-31 under CLI 0.1.19. No tool updates or installation.

## Cleanup and restart recipe

Hub stopped csg-tui PID 218407 and csg-herdr PID 215312, both exit 0. Failed TUI launches already exited. No user/default Herdr session or personal browser was mutated; the original assertion about installed files was invalidated by OBS-008 below. Retain owned /tmp/csg-a3e9b950 repositories/config/resources.json/artifacts for TERM-01; Main/ACCEPT-01 owns final cleanup. GitLab issue remains open. Local GitLab fixture has local initial commit and matching origin only; no remote clone/push.

Before relaunch, revalidate executable hash and absence of a live recorded socket owner. Set owned resource status planned, use resource_guard.prepare_subprocess(load_ledger(...), run_id, session, ['server']) and Hub with the complete returned argv/environment and fixture cwd. Every guarded invocation now has HOME=/tmp/csg-a3e9b950/home; inspect the actual launched server environment rather than trusting a prepared mapping. After snapshot readiness, set status running. TUI adds TERM=xterm-256color; nested opt-in is run-local. Recheck loopback ports before gateway launch; 37619/34585 were free candidates, not held reservations.

Raw artifacts under /tmp/csg-a3e9b950/artifacts have hashes in baseline.json. Compact observations remain durable here. No product code change or project-wide suite; separate code review unnecessary for this no-code inventory.

## Safety correction — OBS-008

Discovered 2026-09-20 during TERM-02 oracle capture. Accumulated Hub TUI output reports installation of pi, omp, Claude and opencode integrations into the real home. The six paths below have modification timestamps at 09:51:52Z during this baseline probe. The earlier Enter/printf attempt was not a harmless proven shell action: it reached the first-run integration UI. Server HOME was inherited as `/home/nnex`, despite isolated XDG roots and the TUI's owned HOME. Original containment claims do not cover these unintended writes.

- `/home/nnex/.pi/agent/extensions/herdr-agent-state.ts`
- `/home/nnex/.omp/agent/extensions/herdr-omp-agent-state.ts`
- `/home/nnex/.claude/hooks/herdr-agent-state.sh`
- `/home/nnex/.claude/settings.json`
- `/home/nnex/.config/opencode/plugins/herdr-agent-state.js`
- `/home/nnex/.config/opencode/herdr-tui-session.js`

Exact-name backup globs found no copies for the initially identified five paths; the sixth was resolved through a narrow opencode filename lookup. Current hashes/timestamps, not contents or credentials, are in [RUN-01-safety.json](RUN-01-safety.json). No pre-probe hashes/copies exist, so exact changes cannot be reconstructed from timestamps alone. Main disclosed this to the user and stopped TUI, server and gateway. Never delete or overwrite these user files speculatively. ACCEPT-01 retains reconciliation as an explicit blocker.

### Accepted corrective plan

ACCEPTED Main/Astra, 2026-09-20. This is a bounded correction to RUN-01's fixture safety requirement, not new product scope. Own only `scripts/verify/resource_guard.py`, its existing tests, this evidence, OBSERVATIONS and ledger notes. Python LSP is unavailable. Locate references before editing. Set every guarded process HOME to a validated `resource_root/home` rather than preserving ambient HOME; remove the TUI-only special case. Keep exact session/config/socket/executable validation unchanged. Reject an owned-home symlink escaping the root. Existing ambient HOME behavior assertions must become containment assertions; preserve real behavior tests, not incidental implementation expectations.

Main implements inline; current TERM-02 and VIEW-01 writers have no conflicting paths or runtime permissions. Focused guard checks run after the concurrent writing wave settles. Review lifecycle and safety independently; restart owned Herdr with the returned isolated environment and verify its actual process HOME and absence of changed protected-file hashes. No integration installation is needed for that proof. Resume product verification only with the corrected environment. This containment repair cannot resolve the earlier user-file disposition: retain OBS-008 for the grouped prerequisite decision.

### Corrective verification and cleanup

The new server/TUI HOME and escaping-symlink regression fails against c360e68's guard (exit 1) and the corrected guard/startup-inventory suite passes 32 tests. FixtureSafetyReview found no evidence-backed defect in this bounded repair; preserving other non-Herdr environment variables is not a general filesystem sandbox. The actual launched server PID 378954 reports owned HOME/config/socket through `/proc`, and its authoritative snapshot reports all ten owned tabs. All six protected-file hashes remain unchanged across this restart; no integration installation was attempted. See [RUN-01-safety.json](RUN-01-safety.json) for exact hashes and limits.

Removed the owned throwaway guard-red code directory after proof. Retain the corrected owned server and private native compositor for TERM-02 under Main's runtime lock; gateway/TUI remain stopped until their next explicit proof step. This fixes future HOME containment, not the earlier incident disposition.

## Delivery checkpoint

Original inventory evidence remains available, with its containment error explicitly superseded by the correction above. Current guarded runtime is ready for TERM-02/VIEW-01 verification. macOS/MR prerequisites and OBS-008 reconciliation remain unresolved; the umbrella campaign cannot complete until their required criteria pass.
