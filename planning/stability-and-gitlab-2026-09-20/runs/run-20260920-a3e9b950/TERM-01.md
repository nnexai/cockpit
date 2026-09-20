# TERM-01 — protocol-based Herdr compatibility

## Accepted just-in-time plan

ACCEPTED Main/Astra (openai-codex/gpt-6-astra), 2026-09-20. Dependency RUN-01 evidence commit 1412c7cbef1315c9ab2474a87d7c34531b7f2dfc; subsequent status checkpoint b5188bc. Baseline product unchanged. Only ledger completion checkpoint changed since RUN-01; no foreign edits. Lock herdr-adapter; Main owns integration/runtime. Read all original TERM-01 requirements, authorities and RUN-01 evidence. No implementation workers: the predicate is a tiny repair, not a standalone delegated slice.

Observed baseline: installed Herdr 0.9.1 advertises live protocol 22 and bundled schema 1; Cockpit returns version_mismatch before inspecting schema. cli.rs::inspect_status exact REQUIRED_VERSION equality causes rejection. Status travels through CockpitService::status to browser /api/v1/status and native cockpit_status, both parsed by parseStatusResponse; App gates session work on compatible. LSP references attempted but rust-analyzer exits because the pinned Rust toolchain lacks that binary. Fallback repository search found REQUIRED_VERSION only in declaration and exact predicate. No generated DTO change needed.

Design: remove display-version equality and obsolete exported constant. Retain display version as diagnostic identity. Require protocol exactly 22, schema exactly 1, and existing 24 REQUIRED_METHODS in cli/capabilities.rs (ping, session.snapshot, events.subscribe, worktree.list, pane.read, workspace/tab/pane/agent focus, workspace create/rename/move_block/close, tab create/rename/move/close, pane split/resize/rename/swap/move/zoom/close). Unsupported/malformed remain incompatible/unavailable respectively with existing codes. No version allowlist, semver framework, protocol downgrade or broader browser/provider changes.

Mouse: protocol-22 structured AttachMouse is the supported wire facility; each attachment's MouseCapture signal is the live optional application demand. Core currently advertises mouse even for incompatible/unavailable normal mode. Gate this capability on successful compatible protocol/schema inspection; preserve TerminalPane's attachment-local MouseCapture checks. No new inferred optional feature. Keep installation/session generation caching and fresh inspection of incompatible results unchanged.

Owned files: crates/cockpit-herdr/src/cli.rs; crates/cockpit-herdr/tests/compatibility.rs; crates/cockpit-core/src/lib.rs status capability expression; directly relevant status tests; this evidence; tasks.json; focused current-authority wording only after proof. All other changes out of scope. Existing tests matching implementation-only exact version policy will be replaced by behavior coverage for later patch acceptance and protocol/schema/method rejection. Preserve malformed/unavailable and session-isolation regressions.

Recipe: change predicate and core capability expression; adapt narrow behavioral cases; build host/native frontend after edits settle; run adapter compatibility and core status gates once. Review protocol predicate independently before runtime proof. Relaunch guarded RUN-01 fixture with explicit session/config, exercise real 0.9.1 browser and Linux-native surfaces. Negative CLI fixtures report protocol mismatch, schema mismatch, missing method, missing executable and malformed JSON; they never target user resources. Compare actual rendered code/message across browser/native. Retry after repaired fixture must reach real session without stale incompatibility. Record exact required/optional capabilities and no-session-work negative evidence.

Coverage: criterion 1 real later patch normal UI; 2 three incompatibilities with no session work; 3 unavailable versus malformed distinction; 4 honest mouse gating; 5 equivalent browser/native identity/message; 6 existing disposable TUI/schema authority. Extra handoff checks: malformed JSON separate, fresh retry, session-specific cache coherence, no remaining exact display-version comparison. Static tests support but never replace surfaces.

Resource identities: retain /tmp/csg-a3e9b950 and csg-a3e9b950 only. New fixtures and captures below that root. Allocate unused loopback gateway port. Linux display capability discovery may use read-only compositor introspection; mutate only owned app window, never user focus/layout. Missing native tooling requires further discovery, not inferred pass. No macOS criterion in this task. Stop on unexpected shared-source change, executable hash drift or ownership ambiguity; replan before expanding APIs. Main owns final acceptance, cleanup, commit and ledger update. This accepted plan is not completion evidence.

## Integrated checks and browser proof

Changed only the fixed display-version predicate, compatible-status mouse capability gating and directly related regressions. Read-only CompatibilityReview found no blocking issue: protocol/schema/24 required methods remain fail-closed, incompatible retry freshly inspects, generation guards unchanged, MouseCapture remains independently required. Dynamic version equality elsewhere binds live session/stream identity; it is not a fixed release gate.

Executed: focused rustfmt; `cargo test -p cockpit-herdr --test compatibility` (18 passed); `cargo test -p cockpit-core --test status` (21 passed); `cargo build -p cockpit-host --bin cockpit`; `cargo build -p cockpit-tauri`; `bun run build` (TypeScript and Vite passed). Builds emitted existing ts-rs/unused browser/helper warnings and Vite chunk-size warning; these are not suppressed or counted as runtime proof.

Gateway runs at 127.0.0.1:37619 against the explicit owned socket, current rebuilt binary and dist. Real 0.9.1 browser status now reports compatible, protocol 22/schema 1, mouse facility true. Browser rendered named csg-a3e9b950 session, Campaign oracle Space, Tab 1 and live fish terminal at 1440×900 and 1024×640. This directly changes RUN-01's version_mismatch baseline.

A run-owned Python executable delegates to pinned real Herdr except injected status/schema faults. Each gateway restart then real App load produced:

| Injected condition | Classification/code | Visible message | Mouse / xterm |
| --- | --- | --- | --- |
| Protocol 23 | incompatible / protocol_mismatch | expected Herdr protocol 22 | false / 0 |
| Schema 2 | incompatible / schema_version_mismatch | expected schema version 1 | false / 0 |
| Missing pane.focus | incompatible / missing_methods | required Herdr methods are missing: pane.focus | false / 0 |
| Invalid JSON | unavailable / malformed_json | Herdr returned invalid JSON: expected ident at line 1 column 2 | false / 0 |
| Executable exits 1 | unavailable / execution_failed | Herdr command failed with status exit status: 1 | false / 0 |

Recorded wrapper argv for all five cases contains only status/schema inspection, no session-list/snapshot/terminal work. Restoring real mode and clicking the actual Retry status button without restarting the gateway restored normal live session UI. Compact JSON captures: /tmp/csg-a3e9b950/artifacts/term01-browser-negative.json. Actual screenshots: term01-browser-retry.png and term01-browser-minimum.png in that directory. These are browser observations, not native proof.

## Linux-native proof and diagnosis

Built the actual packaged native surface with `cargo build -p cockpit-tauri --features tauri/custom-protocol`. Plain cargo debug build initially loaded the development URL without a Vite server; it was not counted as product proof. The packaged build loaded tauri://localhost with real Tauri commands.

An isolated GNOME headless compositor ran under dbus-run-session with HOME/config/cache/data/runtime roots below /tmp/csg-a3e9b950, Wayland socket native-runtime/csg-native and 1440×900 virtual monitor. No user's window or compositor layout was used. WebKitWebDriver listened only on loopback port 34585 and launched the rebuilt binary with explicit COCKPIT_CONFIG/HERDR_EXECUTABLE/HERDR_SESSION/HERDR_SOCKET plus TAURI_WEBVIEW_AUTOMATION=true. Installed tauri-runtime-wry source and NativeProofFacts independently identify that exact variable; the initial TAURI_AUTOMATION spelling was ineffective and the owned failed attempts were stopped.

POST /session used webkitgtk:browserOptions.binary with no browserName request; WebKit negotiated wry 0.55.1/linux. Inspected the actual native DOM and cockpit_status invocation, not a browser substitute. Real 0.9.1 rendered Campaign oracle, tab 1 and the fish terminal; screenshot term01-native-compatible.png was visually inspected. All five fault cases above rendered their matching native notice, and complete native/browser status payloads were asserted equal. Wrapper command logs remained inspection-only in every rejected mode.

Native WebDriver's element click returned unsupported operation. Executed the actual Retry status button's DOM click handler instead, then observed one xterm and settled real fish output without restarting the app. This proves the status retry/action path, not physical native pointer injection. Native screenshots and negative JSON are hashed in TERM-01-artifacts.json.

## Acceptance results

| Criterion | Result | Current evidence |
| --- | --- | --- |
| 1 | PASS | Actual installed 0.9.1 reaches normal browser and packaged Linux-native session UI. |
| 2 | PASS | Protocol 23, schema 2 and missing pane.focus produce distinct incompatible codes/notices in both hosts, zero browser terminals and inspection-only wrapper calls. |
| 3 | PASS | Executable failure remains execution_failed/unavailable; malformed JSON separately remains malformed_json/unavailable; both hosts render actionable Retry status. |
| 4 | PASS | Rejected/unavailable status advertises mouse false, compatible true; independent review confirms unchanged per-attachment MouseCapture gating and no new optional capability inference. |
| 5 | PASS | All five complete browser/native status payloads equal; real compatible identity 0.9.1/22/1 equal; screenshots show actual respective hosts. |
| 6 | PASS | Same guarded RUN-01 TUI/session and live schema authority; normal UI identity comes from real server, not forged positive fixture. |

Extra handoff checks: incompatible retry performs fresh inspection in both hosts; session-specific and stale cache generation regressions passed in the 21-test core status suite. No exact fixed Herdr display-version predicate remains. Optional mouse application demand remains unchanged. No remote issue was closed.

## Cleanup and delivery checkpoint

Deleted native automation session and stopped Hub-owned WebDriver, headless compositor/private D-Bus tree, gateway and Herdr. No active probe service remains. Raw artifacts/config/fixture repositories retained for TERM-02 and final ACCEPT-01 cleanup; throwaway fault wrapper removed after proof. Reuse the recorded resource guard and allocate/check ports anew. Main owns final cleanup.

The first rustfmt invocation recursively formatted unrelated modules; those formatter-only changes were removed using the clean pre-edit baseline. No unrelated source edits retained. Focused formatting uses skip_children thereafter. Static/runtime evidence above covers the semantic repair; no historical test totals reused. Commit only four source/test files, two current authority files and this task's evidence/ledger. Record full resulting SHA in tasks.json after commit. Next selection: TERM-02, with VIEW-01 independently available if its paths/resources are disjoint. Umbrella goal remains active.
