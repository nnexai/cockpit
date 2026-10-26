# Quick agent question ("Ask an agent…"): implementation plan

Companion UX spec: [ux-design.md](ux-design.md) (mock: `mocks/ask-composer.html`). Where this plan and the UX spec differ, this plan wins on delivery mechanics (§Decisions D1–D4). The UX spec still owns layout, copy, focus and accessibility, except for the deltas listed in Slice 5.

## Outcome

From the workbench, the user types one question, picks one of six agents (OMP, Pi, Claude Code, Copilot CLI, Codex CLI, OpenCode) and gets a new Herdr tab in the selected Space, in which that agent runs interactively and has received the question:

- **Literal:** the question bytes reach the agent's input unchanged, as if the user pasted them and pressed Enter.
- **Exactly once:** at most one submission. Cockpit never retries a submission whose outcome is uncertain.
- **Safe cancellation:** cancelling at any point before the single submission sends nothing.
- **Recoverable:** the tab is an ordinary shell tab. It outlives the dialog, the window, Cockpit restarts and agent exit.
- **No orchestration registration:** no task, run, credential, launch tag, inbox or orchestration-store write.

"Literal" is defined precisely: Cockpit delivers the exact UTF-8 text once through Herdr's stock `agent.prompt` (text plus Enter, using bracketed paste when the terminal has it enabled). After that, the harness interprets the message exactly as it would a message the user typed. For example, `@path` mentions keep their normal harness meaning. Cockpit refuses inputs it cannot deliver literally or that would switch the harness out of question mode (D5). It never alters the text.

## Evidence

**Herdr stock API (upstream docs, Herdr v0.9.5 is the latest release, 2026-10-14).**
- `agent.start {name, kind, pane_id, args?, timeout_ms?}` needs an existing pane at its interactive shell prompt. It starts the kind's canonical executable and "returns only after Herdr detects the expected agent in the same terminal and marks it ready for interactive input". If startup reports `blocked`, it returns `agent_not_ready`. The default wait is 30 s; `timeout_ms` must be >3000 and ≤300000. Supported kinds include `pi`, `claude`, `codex`, `omp`, `opencode` and `copilot`. Names must match `[a-z][a-z0-9_-]{0,31}` and be unique among live agents. Source: https://herdr.dev/docs/agent-automation/. Schema: `crates/cockpit-herdr/tests/fixtures/herdr-0.9.0-protocol-22-schema-1.json:1335-1368` (`AgentStartParams`); the result `agent_started {agent, argv}` is at `:9478-9500`.
- `agent.prompt {target, text, wait?}` "submits text plus encoded Enter and honors the terminal's live bracketed-paste mode". An already-`blocked` agent returns `agent_blocked` **without sending input**. "A timeout … does not prove that no input was sent. Read the agent before retrying." Agent input "rejects the operation if that agent no longer controls the pane". Sources: https://herdr.dev/docs/agent-automation/ and https://herdr.dev/docs/socket-api/. Schema: `:1225-1249`; the result `agent_prompted {agent}` is at `:9501-9516`.
- `agent.wait {target, until[], timeout_ms?}` is server-owned and pins the resolved occupant (socket-api docs). Schema: `:1667-1691`.
- `AgentInfo` has `agent`, `name`, `pane_id`, `terminal_id`, `tab_id`, `workspace_id`, `agent_status`, `launch_pending` and `interactive_ready` (`:6609-6733`).
- `tab.create {workspace_id?, cwd?, label?, focus?, env?}` creates a tab whose root pane is a shell (`:4339-4371`; agent-automation docs).
- Process env: Herdr injects `HERDR_SOCKET_PATH`, `HERDR_ENV`, `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID` and `HERDR_PANE_ID`. A caller's `env` can only add variables. Other variables are inherited from the server (socket-api docs, "Process-launching methods accept an env object").
- Server restart restores panes as shells in their saved cwd, or as native agent resumes such as `claude --resume <id>` or `omp --resume=<id>`. It **never reruns the original argv or input** (https://herdr.dev/docs/session-state/).

**Cockpit code.**
- Closed mutation set and Herdr mapping: `crates/cockpit-protocol/src/v1.rs:397-453`, `crates/cockpit-herdr/src/cli/operations.rs:121-206`. The mutating-method list (`operations.rs:6-33`) already contains `agent.prompt` but not `agent.start`.
- Existing identity-checked prompt path, orchestration-only and pinned to a `RunLocation`/launch tag: `crates/cockpit-herdr/src/cli/orchestration.rs:312-346` (target validation; currently rejects control characters including `\n`, ≤4096 bytes), `:370-387` (receipt), `:625-641` (single `agent.prompt`, "never wait, retry, or fall back to keys"). Outcome-unknown classification is at `:20-32`.
- Existing tab creation, snapshot binding and parse helpers: `orchestration.rs:549-593`, `crates/cockpit-herdr/src/cli/projects.rs:414-437` (`parse_tab_result`), `orchestration.rs:433-547` (`runtime()` → `RuntimePane{agent_name, agent_kind, agent_status, launch_pending, terminal_id,…}`, defined at `crates/cockpit-core/src/orchestration/herdr.rs:99-132`).
- `pane.process_info` → shell PID: `orchestration.rs:34-83`, `:643-661`.
- Per-call method gate: `projects.rs:60-67` (`project_methods` via `herdr api schema --json`). The global required set is `crates/cockpit-herdr/src/cli/capabilities.rs:3-25`.
- Response deadlines are per method: `crates/cockpit-herdr/src/cli/transport.rs:8-30`. `agent.prompt`, `agent.start` and `agent.wait` currently fall into the 2 s default.
- Server instance = first 8 bytes of SHA-256 of the endpoint identity, in hex: `crates/cockpit-herdr/src/cli.rs:367-369`, `cli/shell.rs:30`. The Space-tag fence precedent is `crates/cockpit-core/src/space_tags.rs:114-121`.
- Narrow capability-adapter precedent: `crates/cockpit-core/src/paste_adapter.rs:1-35`, `crates/cockpit-herdr/src/cli.rs:694-697`. It is composed at `crates/cockpit-host/src/bin/cockpit.rs:361-430` and `src-tauri/src/startup.rs:145-223`.
- Optional service accessor pattern: `crates/cockpit-core/src/lib.rs:314-322`, `:339-350`.
- Host routes and native commands: `crates/cockpit-host/src/transport/registry.rs:82-91` (session POST body), `:408-417` (session GET pair path); native allowlist in `src-tauri/build.rs:86-90`.
- Generated protocol: section macro `crates/cockpit-protocol/src/typescript/quota.rs:1-16`, section list in `crates/cockpit-protocol/src/typescript.rs:48-86`, exporter `crates/cockpit-protocol/src/bin/export-typescript.rs:7-45` (`--write|--check <path>`). Client parse policy precedent: `src/client/quotaProtocol.ts:1-34`; operation encoding: `src/client/operations.ts:890-900`, `:1012-1016`.
- Escape-injection rule for bracketed paste: `crates/cockpit-core/src/comments/paste/send.rs:137-147` rejects payloads containing ESC.
- Credential-bearing env keys that would give an agent orchestration authority: `crates/cockpit-host/src/cli_orchestration/caller.rs:60-64` (`COCKPIT_JOB_CREDENTIAL_FILE` → job actor); allowlist of orchestration env keys at `orchestration.rs:265-310`. The first credentialed call claims a worker generation (`docs/configuration.md:143`).
- Frontend surfaces: Commands `src/app/shell/commands.ts:48-142`; prefix registry `src/app/input/shortcuts.ts:30-38,97-164`, where lowercase `t` is unused and not Herdr-default (`src/app/input/shortcuts.test.ts:23-37`); dialog mounting `src/app/shell/WorkbenchContent.tsx:99-110`, `useWorkbenchUI.ts:39,90`, `useWorkbenchState.ts:48-50`, `Workbench.tsx:65-79`; notices `useWorkbenchUI.ts:22,54`.
- Disposable fixture isolates `HOME`/`XDG_*` and strips `HERDR_*`/`COCKPIT_*`: `scripts/verify/ui_polish_runtime.py:31-48`. Harnesses inside it are therefore unauthenticated (see Q1).

## Decisions

**D1 — Delivery: stock `tab.create` → `agent.start` → one `agent.prompt`.**
1. `tab.create` creates a new, dedicated, unfocused shell tab (one pane) in the chosen Space, with the resolved absolute `cwd` and label `Ask · <Agent> · <tag4>`. Each ask gets its own pane; existing panes are never targeted.
2. `agent.start` uses kind `<herdr kind>`, name `ask-<kind>-<tag8>`, `timeout_ms: 60000` and no `args`. Herdr owns readiness. `name` is a required `agent.start` parameter (`AgentStartParams.required`, schema `:1362-1366`). It is not a separate `agent.rename`, and it is never used as identity proof on its own (D10).
3. Exactly one `agent.prompt {target: pane_id, text}` follows, immediately after the D10 pre-send gate passes.
4. The composer keeps keyboard focus throughout. Cockpit does not activate or focus the new pane until delivery reaches a terminal phase. The one exception is a blocked startup, which needs the user's action in the pane (D3).

Rejected:
- **Launch argv prompt** (`initial_prompt_args`, `layout.apply` command, or `agent.start` args). This is not literal: omp/pi expand `@file`, leading `-` is parsed as an option, flags differ per harness (`copilot -i`, `opencode --prompt`), args typed by `agent.start` pass through shell parsing and history, and the question would appear in `/proc/*/cmdline`, `pane.process_info` and `layout.export`.
- **`layout.apply` direct argv without a prompt, plus Cockpit-observed readiness.** The prompt would then be literal, but Cockpit cannot inspect the environment before the harness exists. PATH comes from the server env rather than the user's shell. The pane's fate after the harness exits is unverified (UX OQ-2). Readiness would have to be re-implemented from snapshot fields. This is the **fallback** for a harness whose `agent.start` probe fails in Slice 0.
- **`pane.send_text` bracketed paste + `pane.send_keys enter`.** No occupant pin ("pane input addresses the terminal regardless of its current occupant"), no blocked check, and two effects, where losing the Enter leaves a draft.
- **`agent.prompt` with `wait`.** A stall or timeout is still ambiguous and adds latency. The plain acknowledgement is the receipt.
- **Non-interactive modes** (`claude -p`, `codex exec`). These give no recoverable conversation.
- **OMP hook `sendUserMessage`.** OMP only, so it breaks harness neutrality.
- **Plugin pane argv.** Argv is manifest-only.

**D2 — Exactly-once boundary.** Only the `agent.prompt` dispatch can submit. Every earlier failure, including an uncertain `tab.create` or `agent.start`, is reported with `delivery: not_sent`. A stray tab or agent may exist; the question was provably not submitted, so asking again is safe. `agent.prompt` failures are classified as follows:
- Herdr error `agent_blocked`: not sent, by Herdr's documented guarantee. Return to waiting.
- Explicit pre-dispatch refusals from Cockpit (stale identity, not idle): not sent.
- `request_outcome_unknown`, `response_timeout`, `malformed_*`, `session_mismatch`, disconnect, and any other Herdr error not proven pre-input: `delivery: unknown`. Terminal state, never retried.

Herdr error codes beyond `agent_blocked` default to *unknown* unless the Slice 0 probe shows they are returned before input.

**D3 — Readiness.**
- Ordinary path: `agent.start` success means the agent is ready, so Cockpit runs the D10 gate and **sends automatically**. No confirmation is needed.
- `agent_not_ready` (startup blocked, e.g. a trust or login dialog) means the agent is running but waiting on the user. Phase becomes `waiting_for_agent`; the composer offers **Open tab** so the user can answer there. Cockpit loops `agent.wait {target: pane_id, until: ["idle","done"], timeout_ms: 20000}` until the agent is ready, the user cancels, or 10 minutes pass.
- After an observed blocked phase, Cockpit does **not** auto-send. Phase becomes `ready_to_send` and the user must explicitly click **Send question**. The user has just interacted with that pane and may have left input there. The D10 gate still runs at Send, except for the focus check: the user's own focus during that phase is expected.
- An uncertain `agent.start` result (timeout or disconnect) is observed, never replayed. It enters `waiting_for_agent` against the pinned pane and name for up to 60 s. If the agent never appears, the result is `failed`/`not_sent`.

**D4 — No authority leakage.**
- Cockpit sends no `env` to `tab.create`.
- On Linux, before `agent.start`, Cockpit reads `/proc/<shell_pid>/environ` (same UID) of the new shell from `pane.process_info`. If any of `COCKPIT_JOB_CREDENTIAL_FILE`, `COCKPIT_RUN_ID`, `COCKPIT_ROOT_ID`, `COCKPIT_ATTACHMENT_ID`, `COCKPIT_ATTACHMENT_EPOCH`, `COCKPIT_LAUNCH_TAG` or `COCKPIT_INBOX_HOOK` is present, it refuses (`quick_ask_inherited_authority`, not sent) and leaves the shell tab. These would only be inherited from a Herdr server started inside a worker.
- A missing shell PID or an unreadable environ fails closed with `quick_ask_environment_unverified` (not sent, shell tab kept).
- Hosts without a readable `/proc/<pid>/environ` (macOS and other non-Linux) also fail closed with `quick_ask_environment_unverified`. Nothing is skipped. Quick ask is unavailable there until a verified per-process environment probe exists. The start response carries the refusal; the shared interface is unchanged.
- The label `Ask · …` cannot collide with orchestration launch tags (`ck-…`, `crates/cockpit-core/src/orchestration/dispatch.rs:1020`). Quick ask never touches `OrchestrationService` or its store.

**D5 — Input rules.** These are enforced in core; the client mirrors them for UX.
- 1–16384 UTF-8 bytes, not all whitespace.
- Refused: NUL, ESC, other C0 controls except `\n`, CR, DEL and C1 (U+0080–U+009F). ESC could terminate bracketed paste early and inject keystrokes; CR submits.
- Leading `/`, `!`, `@`, `-` and similar characters are delivered unchanged. Safe literal delivery does not require refusing them. The harness interprets them exactly as it would a typed message. The composer states this in its hint ("Sent exactly as typed; <Agent> treats it like a message you typed").
- `\n` is allowed only for harnesses whose Slice 0 probe shows a two-line question arrives as one submission. In the static table this is `multiline: bool`, default `false` until proven. A harness without bracketed paste would turn `\n` into extra submissions.
- No trimming, normalization or templating. Text is never logged or echoed in errors or status, and is dropped from memory at any terminal phase.

**D10 — Pre-send gate, stop rules and the residual limit.**
- Immediately before the single `agent.prompt`, `prompt_once` takes a fresh `session.snapshot` and requires all of the following:
  - The same endpoint identity.
  - Exactly one pane with the pinned `pane_id`, `tab_id` and `terminal_id`.
  - Agent `agent == kind` and `name == name`.
  - `agent_status` is `idle` or `done`, and `launch_pending == false`.
  - The agent's `state_change_seq` equals the value observed at readiness (from the `agent_started` receipt or the ready `agent.wait` receipt).
  - On the ordinary path, `focused == false`. Every pane was created unfocused and Cockpit never focused it, so focus shows that someone else activated it and may be typing.
- Any mismatch stops the operation with `failed`/`not_sent`, code `quick_ask_target_changed`. The pane is kept and the question stays in the composer. Recovery is manual: the user opens the tab, or asks again into a new dedicated pane. Cockpit never sends to that pane later.
- An unknown send outcome stops with `outcome_unknown`. The pane and the question are preserved, nothing is retried, and the composer offers no re-send for that request.
- **Residual limitation (documented, not solved):** Herdr has no compare-and-send. Between the gate snapshot and `agent.prompt` dispatch, another client (another Herdr TUI, a script using `pane.send_text`, a plugin) can still type into or replace the pane. Herdr's own occupant check (`agent.prompt` rejects a pane whose agent changed) covers replacement only. Unfocused external text input before the gate is also not observable through supported APIs, because there is no draft-read method. Cockpit protects existing panes and its own UI's input. It does not guarantee atomicity against arbitrary external mutation of the dedicated pane. This is recorded in `DECISIONS.md` and the composer docs (S6).

**D6 — State and API.**
- A run-local in-memory `QuickAskService` in core (like widget memory) holds operations keyed by a client-generated UUID `request_id`.
- Re-posting the same ID with the same payload joins the existing operation. A different payload is a conflict.
- Herdr-visible identity (tab label tag and agent name) is what survives a Cockpit restart; the in-memory registry does not.
- No durable store: the question may contain secrets, and the pane holds the real record (UX OQ-9).
- The client polls status every 400 ms while the phase is non-terminal.

**D7 — Placement and availability.**
- New tab in the selected Space. `cwd` follows the UX "Runs in" order: Space `git.checkout_path`, then the selected terminal's `cwd`, then a user-entered absolute path. Core requires an absolute path to an existing directory.
- Availability: core looks up the canonical executable on Cockpit's host PATH. A missing executable disables the agent with "Not found on PATH". The shell PATH may still differ; an `agent.start` timeout then reports `not_sent`.

**D8 — Entry.** Commands row `Ask an agent…` (group Navigate, primary) plus prefix `Ctrl+B t` (UX OQ-1). The tab-strip chip tracks a hidden in-flight request (UX §4.2).

**D9 — Deadlines** (`transport.rs::response_deadline`):
- `agent.start`: new `AGENT_START_RESPONSE_TIMEOUT` = 65 s (60 s server timeout plus margin).
- `agent.wait`: new `AGENT_WAIT_RESPONSE_TIMEOUT` = 25 s.
- `agent.prompt`: `PROCESS_MUTATION_RESPONSE_TIMEOUT` (10 s), instead of 2 s, to avoid false *unknown* results under load. This also applies to orchestration inbox hints; that is benign because they are never retried either.
- `agent.start` is added to `request_is_mutating`.

## Open questions

- **Q1 — Live harness authentication in the disposable fixture** (blocks Slice 0 and final acceptance). The fixture sets `HOME`/`XDG_*` to a temp root (`ui_polish_runtime.py:36-42`), so all six harnesses start unauthenticated or in onboarding. Options:
  - (a) A throwaway driver wraps `environment()` to bind each harness's real config/auth directory into the fixture HOME. It uses real credentials and writes transcripts to the user's harness history.
  - (b) Probe delivery only up to the harness's own login screen. That does not prove idle → prompt.
  - (c) A separate disposable named Herdr session with the real HOME. This conflicts with `.omp/RULES.md:8`.

  **Recommend (a) with explicit user approval**, using the trivial question `Reply with exactly: ok`. Exact per-harness dirs/env vars are `[INFERENCE]` until the probe.
- **Q2 — Multi-line questions.** Recommend the D5 rule (probe-gated per harness). The alternative, single-line only for everyone, is simpler but loses pasted code.
- **Q3 — resolved:** no leading-character refusal, because it is not technically required for literal delivery (see D5).
- **Q4 — `agent.start` extra args** (model flags). Recommend none in v1. A later `[quick_ask.<harness>] args` config would need Herdr's quoting of `args` verified first.

## Slices

Order: S0 → S1 → (S2 ∥ S4) → S3 → S5 → S6. Interfaces below are fixed up front; slices must not change them. A deviation is coordinated with Main first. Each slice's "Acceptance" commands are run once by Main after all slices land. Workers do not build or test mid-flight.

### Shared interface contracts

`crates/cockpit-protocol/src/quick_ask.rs` (new; `pub mod quick_ask;` in `lib.rs`; TS section `QUICK_ASK` in `typescript/quick_ask.rs` + `typescript.rs` list):

```rust
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)] #[serde(rename_all = "snake_case")]
pub enum QuickAskHarness { Omp, Pi, Claude, Copilot, Codex, Opencode } // herdr kind == serde name

#[derive(Clone, Debug, Serialize, Deserialize, TS)] #[serde(deny_unknown_fields)]
pub struct QuickAskStartRequest {
    pub request_id: String,       // lowercase hyphenated UUID, generated at first Ask press
    pub server_instance: String,  // from the client's snapshot; mismatch => quick_ask_target_changed
    pub space_id: String,
    pub cwd: String,              // absolute, existing directory
    pub harness: QuickAskHarness,
    pub prompt: String,           // D5
}
#[derive(Clone, Debug, Serialize, Deserialize, TS)] #[serde(deny_unknown_fields)]
pub struct QuickAskActionRequest { pub request_id: String, pub action: QuickAskAction }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, TS)] #[serde(rename_all = "snake_case")]
pub enum QuickAskAction { Cancel, Send }   // Send only valid in ready_to_send

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)] #[serde(rename_all = "snake_case")]
pub enum QuickAskPhase { CreatingTab, CheckingEnvironment, StartingAgent, WaitingForAgent,
    ReadyToSend, Sending, Submitted, Cancelled, Failed, OutcomeUnknown }
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)] #[serde(rename_all = "snake_case")]
pub enum QuickAskDelivery { NotSent, Submitted, Unknown }

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuickAskLocation { pub space_id: String, pub tab_id: String, pub pane_id: String,
    pub terminal_id: String, pub tab_label: String, pub agent_name: String }
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuickAskFailure { pub code: String, pub message: String } // never contains prompt text
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuickAskStatus { pub request_id: String, pub session_id: String, pub server_instance: String,
    pub harness: QuickAskHarness, pub phase: QuickAskPhase, pub delivery: QuickAskDelivery,
    pub cancellable: bool,                 // true iff phase < Sending and not terminal
    pub location: Option<QuickAskLocation>, pub error: Option<QuickAskFailure>, pub updated_at_ms: u64 }

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuickAskAgentAvailability { pub harness: QuickAskHarness, pub executable: String, pub found: bool }
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
pub struct QuickAskAvailability { pub agents: Vec<QuickAskAgentAvailability> } // fixed order, 6 entries
```

Invariants: `delivery == Submitted` ⇔ `phase == Submitted`. `delivery == Unknown` ⇔ `phase == OutcomeUnknown`. Every other phase is `NotSent`.

Routes (`registry.rs`, mirroring `space_tag_change` for POST bodies and `workspace_operation` for GET pair path). Native names are added to `src-tauri/build.rs`:

| id | HTTP | native |
|---|---|---|
| `quick_ask_availability` | `GET /api/v1/quick-ask/availability` | `cockpit_quick_ask_availability()` |
| `quick_ask_start` | `POST /api/v1/sessions/{session_id}/quick-ask` body `QuickAskStartRequest` → `QuickAskStatus` | `cockpit_quick_ask_start(session_id, request)` |
| `quick_ask_status` | `GET /api/v1/sessions/{session_id}/quick-ask/{request_id}` → `QuickAskStatus` | `cockpit_quick_ask_status(session_id, request_id)` |
| `quick_ask_action` | `POST /api/v1/sessions/{session_id}/quick-ask/actions` body `QuickAskActionRequest` → `QuickAskStatus` | `cockpit_quick_ask_action(session_id, request)` |

Core adapter trait `crates/cockpit-core/src/quick_ask/herdr.rs`:

```rust
#[async_trait]
pub trait QuickAskHerdr: Send + Sync {
    /// Fresh snapshot: endpoint server_instance == expected and space exists once; method gate
    /// (tab.create, agent.start, agent.wait, agent.prompt, pane.process_info). Read-only.
    async fn bind(&self, session_id: &str, server_instance: &str, space_id: &str) -> Result<QuickAskBinding, InspectionError>;
    /// tab.create {workspace_id, cwd, label, focus:false} (no env) + fresh snapshot: exactly one pane in tab.
    async fn create_tab(&self, b: &QuickAskBinding, cwd: &str, label: &str) -> Result<QuickAskPane, InspectionError>;
    async fn shell_pid(&self, b: &QuickAskBinding, pane: &QuickAskPane) -> Result<Option<u32>, InspectionError>;
    /// agent.start; Ok(Ready) on agent_started receipt matching pane/terminal/kind/name;
    /// Ok(Blocked) on agent_not_ready; Err(code="herdr_outcome_unknown") when uncertain.
    async fn start_agent(&self, b: &QuickAskBinding, pane: &QuickAskPane, kind: &str, name: &str) -> Result<AgentStart, InspectionError>;
    /// One agent.wait(until idle|done, timeout_ms) pinned to the pane; Ok(Some(state_change_seq)) when ready,
    /// Ok(None) on Herdr timeout. Receipt must match pane/terminal/kind/name.
    async fn wait_ready(&self, b: &QuickAskBinding, agent: &QuickAskAgent, timeout_ms: u64) -> Result<Option<u64>, InspectionError>;
    /// D10 fresh snapshot gate against `agent` (incl. ready_seq; focus check iff require_unfocused),
    /// then exactly one agent.prompt; validate agent_prompted receipt. Err distinguishes not-sent vs unknown (D2).
    async fn prompt_once(&self, b: &QuickAskBinding, agent: &QuickAskAgent, require_unfocused: bool, text: &str) -> Result<(), PromptError>;
}
pub struct QuickAskBinding { pub session_id: String, pub endpoint_identity: String, pub server_instance: String, pub space_id: String }
pub struct QuickAskPane { pub tab_id: String, pub pane_id: String, pub terminal_id: String }
pub struct QuickAskAgent { pub pane: QuickAskPane, pub kind: String, pub name: String,
    /// AgentInfo.state_change_seq observed when Herdr reported ready; None until ready.
    pub ready_seq: Option<u64> }
pub enum AgentStart { Ready(QuickAskAgent), Blocked(QuickAskAgent) }
pub enum PromptError { NotSent(InspectionError), Unknown(InspectionError) }
```

All Herdr calls after `bind` pass `Some(endpoint_identity)` to `socket_request_with_identity`.

### S0 — Live probe (no product code)

- **Goal:** confirm the assumptions that D1–D5 depend on, per harness, before coding.
- **Files:** throwaway driver under the fixture root only. The result goes in `docs/verification-log.md` in S6.
- **Steps** (disposable fixture, Q1 resolved), via `ui_polish_runtime.py rpc <root>`:
  1. `herdr api schema --json` on the installed Herdr: confirm `agent.start`/`agent.wait`/`agent.prompt`/`pane.process_info` and the shapes of `agent_started`/`agent_prompted`/`agent.wait` results.
  2. For each of the six kinds: `tab.create` → `agent.start {name, kind, pane_id, timeout_ms:60000}`. Record time to ready, `agent_not_ready` cases and the error codes for a busy shell, a missing binary and a duplicate name.
  3. `agent.prompt` with `Reply with exactly: ok`. Then a two-line question (`line one\nline two`) → `pane.read` to check whether it arrived as one message (sets `multiline`).
  4. Prompt a deliberately blocked agent and confirm `agent_blocked` leaves the input unchanged.
  5. Exit the harness and confirm the pane returns to the shell.
  6. Record the agent PID's `/proc/<pid>/environ` and confirm it contains no `COCKPIT_*` keys.
  7. Confirm the `session.snapshot` agent records carry `focused` and `state_change_seq`. Confirm `state_change_seq` stays unchanged while an unfocused agent sits idle for 30 s (no spurious bumps from title or "seen" changes), and that it bumps on a status transition. If either fails, D10 replaces the sequence check with the nearest stable supported field the probe identifies, recorded as a decision update.
- **Acceptance:** a table of 6 harnesses × {ready, blocked-start behavior, multiline, exit→shell}, plus the Herdr error-code list. Any harness failing `agent.start` switches to the D1 fallback for that harness only; record that as a decision update in this file.
- **Non-goals:** UI, product edits.

### S1 — Protocol + generated TS (owner: Protocol agent)

- **Goal:** add the DTOs above.
- **Files:** `crates/cockpit-protocol/src/quick_ask.rs` (new), `crates/cockpit-protocol/src/lib.rs`, `crates/cockpit-protocol/src/typescript/quick_ask.rs` (new), `crates/cockpit-protocol/src/typescript.rs`, regenerated `src/protocol/generated/v1.ts` and `src/protocol/generated/validate/{quick_ask.ts,policy_quick_ask.ts,index.ts}`.
- **Steps:**
  1. Add the DTOs.
  2. Register the section.
  3. Run `cargo run -p cockpit-protocol --bin export-typescript -- --write src/protocol/generated`. Confirm the path argument against the existing test in `crates/cockpit-protocol/tests/typescript.rs`.
- **Acceptance:** `cargo test -p cockpit-protocol`; `export-typescript --check` clean.
- **Non-goals:** behavior.

### S2 — Herdr adapter (`cockpit-herdr`)

- **Goal:** implement `QuickAskHerdr` for `HerdrCliAdapter`.
- **Files:** `crates/cockpit-herdr/src/cli/quick_ask.rs` (new; `mod quick_ask;` in `cli.rs`), `cli.rs` (`pub fn quick_ask_adapter(&self) -> Arc<dyn QuickAskHerdr>` next to `paste_adapter` at `:694`), `cli/operations.rs` (`agent.start` in `request_is_mutating`; test), `cli/transport.rs` (D9 deadlines; tests near `cli.rs:3297-3327`), `cli/shell.rs` (make `server_instance` `pub(super)` if needed; reuse it, no third copy).
- **Steps:**
  1. `bind`: `runtime()` + `project_methods()` + space count == 1 + `server_instance(endpoint) == expected` → else `quick_ask_target_changed` / `unsupported_capability`.
  2. `create_tab`: like `create_agent_tab` (`orchestration.rs:549-593`) without its launch-tag validation: `parse_tab_result`, then fresh `runtime()` with exactly one pane in the tab and a non-empty `terminal_id`. Map uncertainty with the existing `unknown()`.
  3. `shell_pid`: `pane.process_info`.
  4. `start_agent`: params `{name, kind, pane_id, timeout_ms: 60000}`. Validate the `agent_started` receipt fields. Map Herdr `agent_not_ready` → `Blocked`. Map S0-proven pre-start refusals → `quick_ask_agent_start_refused`. Everything else → `herdr_outcome_unknown`.
  5. `wait_ready`: one `agent.wait`. A Herdr timeout → `Ok(None)`. On ready → `Ok(Some(state_change_seq))` from the receipt agent, which must match pane/terminal/kind/name. `start_agent`'s `Ready` likewise fills `ready_seq` from the `agent_started` receipt.
  6. `prompt_once`:
     - D10 gate from a raw `session.snapshot` read. `RuntimePane` lacks `focused` and `state_change_seq`, so read those from the raw agent record, the same way `runtime()` reads `launch_pending` (`orchestration.rs:525-528`); do not widen `RuntimePane`. Any mismatch → `NotSent(quick_ask_target_changed)`. Non-idle → `NotSent(quick_ask_agent_busy)`.
     - Dispatch `agent.prompt`.
     - Herdr `agent_blocked` → `NotSent`.
     - S0-proven pre-input errors → `NotSent`.
     - Everything else, including an invalid receipt → `Unknown`.
     - Never wait, retry or fall back to keys.
- **Tests (fake socket, pattern at `orchestration.rs:1369-1373`):** one request per call; no second `agent.prompt` after a timeout; receipt mismatch → `Unknown`; `agent_blocked` → `NotSent`; params contain no `env`/`args`; identity is pinned.
- **Acceptance:** `cargo test -p cockpit-herdr`.
- **Non-goals:** orchestration paths. `prompt_agent` and `validate_prompt_target` stay unchanged.

### S3 — Core service

- **Goal:** state machine, validation, env preflight, availability.
- **Files:** `crates/cockpit-core/src/quick_ask.rs` + `quick_ask/herdr.rs` + `quick_ask/tests.rs` (new); `crates/cockpit-core/src/lib.rs` (`quick_ask: Option<Arc<QuickAskService>>`, `with_quick_ask`, `quick_ask()` → `quick_ask_unavailable`, per `lib.rs:314-350`).
- **Steps:**
  1. `const HARNESSES: [(QuickAskHarness, kind, executable, label, multiline); 6]`, with `multiline` from S0.
  2. `validate_prompt` (D5). Error code `quick_ask_invalid_prompt`; the message names the rule, never the text.
  3. `start`:
     - Validate the UUID, space ID (`validate_resource_id`), and that `cwd` is absolute and a directory.
     - Dedupe or conflict (`quick_ask_request_conflict`).
     - Bound: ≤4 non-terminal operations, else `quick_ask_busy`; retain ≤64 terminal statuses for 30 min.
     - `bind` synchronously and return its error directly (*refused*, nothing created).
     - Spawn the task and return `CreatingTab`.
  4. Task sequence:
     - `create_tab` → `CheckingEnvironment`.
     - `shell_pid` → read `/proc/<pid>/environ` and reject forbidden keys (D4). An unreadable environ, a missing PID or a non-Linux host fails closed with `quick_ask_environment_unverified`. There is no skip path.
     - `StartingAgent`.
     - `start_agent`:
       - `Ready` → send.
       - `Blocked`/unknown → `WaitingForAgent` loop with a deadline (D3), using `tokio::select!` with a cancellation `Notify`.
       - Ready after a blocked phase → `ReadyToSend`, waiting for `Send`.
     - Check the cancel flag and transition to `Sending` **under one mutex**.
     - `prompt_once` → `Submitted` | `Failed(NotSent)` | `OutcomeUnknown`.
     - Drop the prompt at every terminal phase.
  5. `action(Cancel)`:
     - Allowed while `cancellable`. Sets the flag and wakes waits. An in-flight `create_tab`/`start_agent` finishes, then the operation ends `Cancelled` with the tab kept.
     - In `Sending` or terminal phases: `quick_ask_cancel_too_late`, status unchanged.
  6. `action(Send)`: only in `ReadyToSend`, else `quick_ask_invalid_action`.
  7. `status` of an unknown ID → `quick_ask_request_unknown`.
  8. `availability()`: PATH lookup per executable, using the existing helper behind `config.rs:304-307` if reusable, otherwise a small local search over `PATH` entries for an executable regular file.
- **Tests (fake `QuickAskHerdr`):**
  - Every phase transition.
  - Exactly-once: `prompt_once` is called ≤1 time across cancel races, duplicate start, Send double-click and unknown outcomes.
  - Cancel at each phase never reaches `prompt_once`.
  - Ordinary ready auto-sends with `require_unfocused = true`, without any client action. `ready_to_send` sends only on `Send`, with `require_unfocused = false`.
  - A D10 gate failure ends `failed`/`not_sent`/`quick_ask_target_changed`, with no further `prompt_once` for that request.
  - A blocked startup requires Send.
  - Forbidden env → refused, and `start_agent` is never called.
  - Every D5 rule.
  - Prompt absent from errors and Debug output.
- **Acceptance:** `cargo test -p cockpit-core quick_ask`.
- **Non-goals:** orchestration, persistence, process killing, tab closing.

### S4 — Host + native wiring

- **Goal:** expose the operations on HTTP and IPC.
- **Files:** `crates/cockpit-host/src/transport/operations/quick_ask.rs` (new; `mod` in `operations/mod.rs`), `transport/registry.rs` (4 entries per the table; limit `MUTATION_BYTES`), `crates/cockpit-host/src/bin/cockpit.rs` (compose `QuickAskService::new(adapter.quick_ask_adapter())` beside `with_space_tags` at `:368`), `src-tauri/src/startup.rs` (same, beside `:220`), `src-tauri/build.rs` (4 command names).
- **Steps:** follow `session.rs:105-122` for handlers; the gateway validates the session ID like `mutate` (`session.rs:143-173`).
- **Acceptance:** `cargo test -p cockpit-host`; `cargo build -p cockpit-host --bin cockpit`; `cargo check -p <tauri crate>`.
- **Non-goals:** UI.

### S5 — Client + UI

- **Goal:** the composer per the UX spec, adapted to these phases.
- **Files:**
  - `src/client/CockpitClient.ts` (4 methods), `src/client/operations.ts` (encode like `:890-900`, `:1012-1016`), `src/client/quickAskProtocol.ts` (+ test; enforce the `delivery`/`phase` invariant and the 6-entry fixed order).
  - `src/app/quickAsk/QuickAskDialog.tsx`, `useQuickAsk.ts` (Workbench-level operation + 400 ms poll + chip state), `quickAsk.ts` (labels, D5 mirror, tag derivation), `quickAsk.css`, tests.
  - `src/app/input/shortcuts.ts` (`"ask-agent"` PrefixCommand, `k("t")`, group Navigate, `needs: "space"`, `primary: true`) and `shortcuts.test.ts`.
  - `src/app/shell/useWorkbenchInput.ts` (`runCommand` branch).
  - `src/app/shell/commands.ts` (`run.askAgent`, disabled reasons per UX §4.3).
  - `useWorkbenchUI.ts` / `useWorkbenchState.ts` (`askOpen`, included in `modalOpen`).
  - `Workbench.tsx` / `WorkbenchContent.tsx` (mount dialog + chip).
  - `src/app/input/modal.ts` (`FOCUSABLE` includes `textarea:not(:disabled)`, UX §5.5-1).
  - `src/app/shell/TabStrip.tsx` (chip).
  - Regenerate the `docs/keyboard-shortcuts.md` block via `renderShortcutDocs` (`shortcuts.ts:338`).
- **Deltas from the UX spec:**
  1. Phases as in `QuickAskPhase`. "Starting" covers `creating_tab`, `checking_environment` and `starting_agent`; the secondary button is **Don't send**, which is a real cancel.
  2. `waiting_for_agent` copy: "<Agent> needs your input in tab "Ask · … · 7f3a" before it can take the question." with **Open tab**. This is the only pre-delivery path that activates the pane, and only on the user's click.
  3. `ready_to_send`: primary **Send question**.
  4. Unknown applies only to the submission: "Cockpit couldn't confirm whether your question was sent. Open the tab to check before asking again." Buttons: **Open tab**, **Copy question**. No Check again. **Dismiss** returns to Editing with the draft, after the user has used Open tab or Copy.
  5. `failed`/`not_sent`: "<Agent> didn't get your question: <reason>. Your question is still here." A stray tab is mentioned when `location` is set.
  6. More → preview text: "Starts `<executable>` in a new shell tab, then submits your question once."
  7. Counter limit 16,384 bytes.
  8. Inline refusal for the D5 control-character rules and for newlines with non-multiline harnesses. No leading-character refusal.
  9. Keep focus in the composer through every pre-delivery phase. Never call focus on the new pane before a terminal phase. On `submitted` with Switch on and the dialog visible, call the existing `client.focus({kind:"pane", target_id: pane_id})`. On `failed` with `quick_ask_target_changed`, show: "The new tab changed before your question was sent, so Cockpit stopped. Nothing was sent. Your question is still here." with **Open tab**.
  10. Request ID: `crypto.randomUUID()` at the first press, reused for any re-POST.
  11. The draft lives in memory only.
  12. Last agent is a local UI preference.
- **Acceptance:** `bun run typecheck`; `bun run test` (dialog phase rendering, single request under repeated Ctrl+Enter, cancel calls action not start, Unknown has no retry path, invariant parsing).
- **Non-goals:** chat history, answer streaming, settings UI.

### S6 — Docs and live smoke

- **Docs:**
  - `DECISIONS.md`: one rule under harness/launch covering D1, D2, D4 and D5, with owner `crates/cockpit-core/src/quick_ask.rs`.
  - `CONTEXT.md` Local surfaces: one sentence.
  - `CODE_GUIDE.md`: owner rows for frontend, core, adapter and routes, plus a scenario-selection row.
  - `docs/supervisor-surfaces.md` if it lists Commands surfaces.
  - `docs/verification-log.md`: S0 and smoke observations.
- **Smoke (disposable fixture per Q1; browser build):**
  1. For each harness: `Ctrl+B t` → ask `Reply with exactly: ok` → exactly one new `Ask · … · xxxx` tab, one submission visible in `pane.read`, sidebar shows the agent, and the Supervisor Tasks/Overview and orchestration snapshot are unchanged.
  2. Literal: `a  b "q" $HOME '\x'` (plus the two-line variant for multiline harnesses) appears verbatim in the transcript.
  3. Cancel during `starting_agent` → no submission (`pane.read`), tab kept.
  4. Blocked startup (fresh fixture HOME trust dialog for Claude) → `waiting_for_agent` → user answers → `ready_to_send` → Send → one submission.
  5. Induced unknown: stop the Herdr server between `start_agent` and `prompt_once` via a fake delay hook in a test build only. Otherwise rely on the S3 unit test and claim only what was observed.
  6. Harness exit → pane returns to the shell.
- **Native:** one native run of a single harness through the composer (new IPC commands).
- **Acceptance:** the steps above are observed and recorded. Unverified items are stated, not claimed.

## Risks & verification

| Risk | Mitigation | Verified by |
|---|---|---|
| Duplicate submission | Single `prompt_once` call site; mutex-guarded `Sending` transition; request-ID dedupe; no retry on unknown; UI has no retry path | S2/S3 unit tests; S5 tests; smoke step 1 (one submission) |
| Lost or partial submission due to `\n` without bracketed paste | Per-harness `multiline` gate from probe evidence | S0 table; smoke step 2 |
| Keystroke injection through ESC in paste | D5 rejects ESC/C0/C1/CR/DEL | S3 tests |
| Harness-mode input (`/`, `!`) | Delivered as typed, by explicit decision; the hint states harness semantics apply | S5 copy test |
| Merging into a user draft or another pane | Dedicated new unfocused pane per ask; the composer keeps focus; D10 gate (identity, readiness, unchanged `state_change_seq`, unfocused) immediately before the single send; explicit Send after an observed blocked phase; stop-and-preserve on any mismatch | S2/S3 tests; smoke step 4 |
| External concurrent mutation between gate and send | Residual limitation, documented (D10); Herdr's occupant check covers replacement | `DECISIONS.md` text (S6) |
| Wrong target (pane replaced, Herdr restarted) | `server_instance` fence; endpoint-pinned requests; snapshot gate on terminal/kind/name; Herdr occupant pinning | S2 tests |
| Orchestration authority leak | No env sent; environ preflight fails closed everywhere, including non-Linux; no orchestration calls; non-`ck-` label | S3 tests (Linux forbidden key, unreadable environ, non-Linux refusal); S0 environ check; smoke orchestration snapshot unchanged |
| Secret exposure | Prompt never in argv, logs, errors, status or disk; dropped at terminal phase | S3 test (Debug/error strings) |
| Shell PATH differs from host PATH | Availability is advisory; `agent.start` timeout → `not_sent` | Smoke with a missing binary, if cheap |
| Herdr error codes undocumented | Unknown-by-default classification; S0 records codes | S0 |
| Unsupported Herdr | Per-call method gate → `unsupported_capability` | S2 test |
| Concurrent windows | Per-request UUID; core bound of 4 in flight | S3 test |
