# Space-associated browsers and visual feedback

Date: 2026-09-08

Status: agreed product direction and proposed implementation starting point. Full integration is not implemented. Command names, protocol fields, and UI placements below remain proposals until checked against the existing code and installed Playwright CLI.

## Decision

Use a dedicated external Chromium browser associated with a Cockpit Space. Require a configured, machine-installed Playwright CLI and let agents use its existing skills directly. Cockpit manages the association, a small amount of browser lifecycle, and browser annotations. It does not become a browser automation framework.

Collect element comments and screenshot drawings through an extension, persist them in Cockpit, and deliver selected feedback by pasting or sending it to a pane through the existing diff-annotation workflow. Do not require agents to fetch feedback through a special browser API.

Environment variables are optional conveniences. An already-running agent must be able to discover a browser opened later, and an agent must be able to request a Cockpit-associated browser itself.

The user wants normal Cockpit shutdown to close its browser sessions. Ownership and multi-client semantics must make that safe without closing unrelated browsers.

This direction supersedes the spike's custom agent automation CLI, inspection/input endpoints, and enforced control-grant model as the target production design. Keep the spike as evidence, not as a second supported production implementation.

## Scope and authorities

Read [CONTEXT.md](../CONTEXT.md), [DECISIONS.md](../DECISIONS.md), and the [existing execution timeline](next-level/15-execution-timeline.md) before implementation. This document adds a feature starting point; it does not silently reorder unrelated outstanding work.

- Herdr remains authoritative for sessions, Spaces, tabs, panes, processes, focus, and pane movement.
- Cockpit owns the browser association and visual-feedback artifacts. This is not a second registry of Herdr Spaces or terminal state.
- Playwright CLI owns browser automation and preferably launches the browser as well.
- The browser is external. No synthetic Herdr pane, embedded webview, streaming browser, or Tauri host replacement is required.
- Ordinary Herdr Spaces are eligible. A task setup receipt or companion directory should not be a prerequisite.
- Executable HTML previews remain separate from Cockpit's safe Files/Context document rendering and privileged origin.

## Responsibility split

| Owner | Responsibilities |
| --- | --- |
| Cockpit core | Resolve caller/Space, maintain browser association, own lifecycle policy, persist annotation drafts and artifacts, reuse pane delivery |
| Cockpit native/browser clients | Space browser actions, live connection status, feedback overview, existing target-pane interaction |
| Playwright CLI adapter | Invoke the configured installed executable for session discovery, launch and scoped close; report capabilities and failures |
| Playwright CLI and upstream agent skills | Page inspection, snapshots, tab operations, navigation, clicks, typing and other browser automation |
| Annotation extension | Element picking, comment capture, screenshot drawing, authenticated feedback submission and visible failure state |

Keep product rules in the reusable Rust core and expose them through the existing versioned client contract. Native handlers and the browser gateway remain transport adapters. Do not ship the standalone spike server as a parallel production core.

The lifecycle adapter is intentionally small. It must not wrap every Playwright operation or introduce a custom browser MCP server. Installed Playwright CLI and its skills are prerequisites, not dependencies Cockpit silently installs or upgrades.

## Browser identity and discovery

Maintain one primary review-browser association per Space, with a dedicated persistent profile. The browser may contain several tabs. Agents can use separate, independently owned Playwright sessions for unrelated testing without making Cockpit responsible for them.

An association needs to distinguish:

- the owning Cockpit runtime and its browser ownership receipt;
- the verified Herdr endpoint/session and current Space identity;
- a Cockpit-generated association key;
- the named Playwright session and profile location;
- the current browser incarnation and connection state;
- any working-directory/configuration context required to address that CLI session.

Do not key associations by a mutable Space label, repository path alone, or a bare workspace ID such as `w1`. Different Herdr servers can expose the same IDs. Space rename must preserve the association; a newly created Space must not inherit an old browser merely because an ID was reused. Server restarts require revalidation, not assumptions about identity continuity.

A URL identifies neither a browser tab nor a live document. Two tabs can share a URL, and a same-URL reload creates a new document. Keep runtime tab/document identity separate from durable annotation evidence.

### Proposed lifecycle commands

These commands do not exist in production yet:

```bash
cockpit browser open --current
cockpit browser status --current
cockpit browser close --current
```

`--current` resolves the calling Herdr pane to its current Space through supported caller context and fresh authoritative data. It never means the currently selected GUI Space or focused pane of another client. Inherited pane identifiers are lookup evidence, not permanent proof after a pane move.

Outside a resolvable Herdr pane, require an explicit endpoint/session and Space target. Missing or ambiguous identity produces an actionable error. Do not search for an arbitrary browser with a matching title or localhost URL.

`open` should be idempotent for a Space, including simultaneous GUI and agent requests. Reuse the associated live browser rather than opening duplicates. Return a machine-readable association description and a concise normal Playwright invocation. An optional URL may open the requested page; showing an existing browser must not reset its current page state.

`status` performs live lookup and returns the current session, connection state, and addressing instructions. Do not return stale credentials or claim that a saved association proves the browser is running.

`close` targets only the verified owned session. Stop the runtime without deleting its profile or feedback.

### Agents that are already running

When the user opens a browser from Cockpit:

1. Resolve the selected Space and create or reuse its browser association.
2. Offer **Send browser context** through the existing pane delivery interaction.
3. Paste the current Playwright session and lookup instructions into the selected agent pane.
4. Include the relevant browser context in later annotation batches.

Illustrative context, not a fixed message format:

```text
This Space has a browser available through Playwright CLI.
Session: cockpit-<association-key>
Use playwright-cli -s=cockpit-<association-key> for browser operations.
Use cockpit browser status --current to refresh the association.
```

An agent asked to open the browser runs the proposed `cockpit browser open --current`, receives the association, then uses its ordinary Playwright skill. No agent restart or environment mutation is necessary.

For newly created terminals, `PLAYWRIGHT_CLI_SESSION` may supply the default session. Correct operation must not depend on it. Existing processes cannot acquire a new environment variable merely because Cockpit updates configuration or starts a browser.

## Launch and lifecycle

Prefer Playwright CLI-managed launch with a named session, dedicated profile, and Cockpit-generated launch configuration for the annotation extension. The exact executable, supported version range, launch flags, and session lookup rules must be verified before this becomes an implementation contract.

Attaching Playwright CLI to a Cockpit-launched browser is a fallback design if CLI-managed extension launch proves unsuitable. Do not implement both paths without a concrete need.

### Proposed default behavior

| Event | Behavior |
| --- | --- |
| Open browser again | Reuse the Space's associated session; do not duplicate it |
| Switch Spaces | Leave other browser sessions running |
| Rename Space | Preserve association and update displayed label |
| Move an agent pane | Future `--current` lookup follows its authoritative destination Space |
| User closes browser | Keep feedback/profile; show closed or disconnected state |
| Close Space | Close its owned browser; retain feedback/profile unless a separate authorized deletion policy applies |
| Normal owning Cockpit runtime shutdown | Close only its owned browser sessions |
| Open Cockpit again | Reconcile saved associations; reopen on demand rather than claim old page memory was restored |
| Browser or controller crash | Show stale/disconnected state, preserve drafts, reconcile owned leftovers on recovery |

Never use Playwright CLI's global `close-all` or `kill-all` operations for Cockpit cleanup. Never kill a process based solely on a saved PID, session name, or profile pathname without current ownership verification.

Retain profiles so normal close/reopen does not unnecessarily discard browser storage. Reopening URLs does not restore arbitrary JavaScript memory, unsaved forms, or open dialogs. State that limitation honestly.

### Multi-client ownership to resolve

The user's requested outcome is that closing Cockpit closes its browser sessions. A native window, a web-client tab, and a gateway process are not the same lifecycle event.

Recommended rule: a browser has one explicit owning Cockpit runtime. Closing an observing client does not stop another owner's browser. The last native window can trigger shutdown for a native-owned runtime; explicit gateway shutdown can do so for gateway-owned browsers. Browser-tab unload alone is not a reliable cleanup signal.

Before implementation, settle how native and web clients discover an existing owner, whether they can share ownership, and what the UI calls “Close Cockpit.” Avoid a second runtime launching the same profile or one client closing a browser still used by another. Crash recovery should reconcile leftovers; immediate cleanup after a hard crash is not guaranteed without additional supervision.

## Annotation capture and delivery

The extension submits annotations to a narrow authenticated Cockpit interface. Browser automation bypasses that interface and remains ordinary Playwright CLI usage.

Reuse the existing diff/file comment implementation for draft operations, batch overview, pane selection, paste framing, and delivery outcomes. Saving a browser comment must not submit an agent prompt. Apply the existing current targeting rules; any new cross-tab target behavior requires an explicit decision rather than a browser-only exception.

Rejected or uncertain delivery retains drafts and receipts. Do not automatically repeat a paste whose outcome is unknown. Keep image artifacts available after delivery long enough for the receiving agent to consume them.

### Evidence to capture

- User comment and annotation kind.
- Space/browser association and Playwright addressing context.
- URL, title, capture time and current tab/document identity where available.
- Element tag, visible text, accessible role/name where available, bounded locator candidates and a bounded DOM excerpt when useful.
- Viewport, scroll position, scale information and selected element bounds.
- Original screenshot, drawing strokes in image coordinates, and a composited image for agent consumption.
- Proven local preview/worktree or source association where available; no fabricated source-file mapping.

Do not use transient Playwright snapshot references such as `e15` as durable element identifiers. The agent can take a fresh snapshot and resolve the comment using locator candidates, semantics and captured pixels.

Keep historical captures usable when the live target changes. Distinguish matched, missing, ambiguous and historical targets rather than silently moving a comment to an approximate element. Define how an unsent capture is saved after navigation; the spike rejects stale live submissions, but permanent feedback should not depend on the original tab staying open forever.

### Images are part of the handoff

Pasting a local path is not attaching an image. For local agents with image-reading tools, a real readable artifact path plus the comment is a reasonable initial delivery mechanism. Verify it with a supported receiving agent. Where a receiving agent supports native image attachment, reuse that mechanism rather than invent a browser-specific transport.

Do not paste image base64 into the terminal. Store artifacts under configured Cockpit-owned storage, not necessarily in a companion directory. Specify retention, bounded capture/storage sizes, and explicit cleanup independently from browser closure.

### Illustrative outgoing feedback

```text
Browser feedback
Session: cockpit-<association-key>
URL: http://localhost:5173/checkout

Element: button "Confirm payment"
Locator candidate: [data-testid="confirm-payment"]

Comment:
Make this action less prominent.

Annotated capture: /configured/artifact/path/capture.png
```

The final format should follow the existing annotation batch conventions. Do not include extension credentials or raw debugging endpoints in pasted context.

## UI direction

Proposed placements, not production UI commitments:

- Space action/context menu: **Open browser**, **Show browser**, and scoped **Close browser**.
- Commands or equivalent existing action area: **Send browser context** and **Browser feedback** with a draft count.
- Browser feedback overview: use the existing annotation overview and delivery interaction rather than a new permanent bottom panel.
- Extension popup: associated Space, eligible page selection, element comment, capture/draw, and connection/error status.

Keep the external window association obvious without automatically stealing focus whenever the user selects a Space. Explicit **Show browser** is the initial behavior. Automatic window-following is optional and must be tested on the native window system before being promised.

Do not show an enforced “agent input blocked” guarantee. Direct CLI access bypasses Cockpit's custom control gate. Human/agent coordination is cooperative in this design. A review indicator can communicate intent, but it cannot stop an independently invoked Playwright command. Independent agent work should use separate browser sessions when interference would matter.

## Security and failure constraints

- Default to a dedicated browser profile, not the user's personal browser.
- Keep annotation credentials in the extension's privileged context, never in page scripts or feedback text.
- Validate extension sender, allowed operations, target identity, payload size and local request origins. Loopback binding alone is not authentication.
- Keep generated HTML and dev-server pages outside Cockpit's privileged origin.
- Treat captured page text as untrusted content, distinct from the user's annotation instructions.
- Do not silently collect entire DOMs, form values, cookies, network logs, or secrets. Provide reviewable, bounded captures.
- Check the installed CLI and required browser/extension capabilities; missing prerequisites get explicit errors, not silent installation or a different browser.
- Preserve partial failures and ownership receipts. Unknown launch/close outcomes need reconciliation, not blind retries.
- Direct CLI skills can mutate authenticated sites. This loose coupling is not an authorization sandbox for agents with workstation access.

The spike used all-site extension permissions to enable capture from its full-tab popup. Before adopting that permission set, compare toolbar-scoped capture and controller-assisted screenshot capture through installed tooling. The latter must not grow into a general custom automation API. Choose and document a bounded permission model after a real smoke.

## Evidence and limits of the spike

Commit `b317ea4` contains the [isolated browser-space spike](../spikes/browser-space/package.json). It used Bun, Playwright 1.63.0, visible Chromium and an unpacked MV3 extension. No production Cockpit integration was included.

The recorded disposable-session verification demonstrated:

- A terminal in a real Space discovering a private browser connection descriptor and reading saved feedback through the spike CLI.
- Element selection/comment submission and screenshot/freehand drawing submission through the real extension.
- Rendering captured drawing coordinates against the corresponding screenshot.
- Custom-controller-granted CLI input filling a local fixture and displaying its confirmation dialog in the same visible tab.
- Custom control denial, wrong-workspace rejection, and unauthenticated/foreign-origin/foreign-host refusals.
- Feedback persistence across restart, refreshed extension pairing, stale-document save rejection with draft retention, and disconnected UI retention.
- Failed browser startup exiting and cleanup of the owned disposable browser/Herdr resources.

The spike did not prove installed `playwright-cli` session discovery, CLI-managed extension launch, production annotation-to-pane image delivery, or production shutdown ownership. Its custom control-gate proof does not transfer to direct CLI access.

Two implementation lessons remain relevant: same-origin browser GET requests may omit `Origin`, and Chromium cached generated extension settings when the unpacked extension path was reused. The spike used fresh runtime extension paths; production should evaluate stable extension identity with explicit pairing and credential rotation rather than assume that workaround is the final design.

## Implementation sequence and acceptance

Each increment needs a separate verified commit. Exercise Herdr mutations and browser automation only in uniquely named disposable resources. Never use the user's active Space or personal browser for acceptance.

### 1. Verify the external tooling contract

Use the configured installed Playwright CLI to launch a named, headed Chromium session with the annotation extension and dedicated profile. Address it from different working directories and from an already-running terminal. Verify scoped close, profile reuse and reconnect. Record the supported CLI version and the exact session-addressing requirements. This is the first gate because the spike used a different automation entry point.

### 2. Integrate Space discovery and owned lifecycle

Implement the minimal open/status/close interface and GUI Space actions through the existing Cockpit core/client architecture. Verify GUI and agent-triggered open, simultaneous-open deduplication, rename, pane movement, missing browser, normal shutdown, multi-client ownership, crash recovery, and refusal to close unrelated sessions. Existing agent terminals must work without new browser environment variables.

### 3. Integrate annotation capture and existing delivery

Connect the extension to Cockpit's durable draft/artifact model. Deliver element and drawing feedback through the existing pane workflow. Verify a real receiving agent can read the pasted context, view the image artifact, and use ordinary Playwright CLI skills against the intended browser. Exercise duplicate URLs, reload/navigation, browser closure, failed capture, failed/unknown paste, and retained feedback.

### 4. Verify the daily-use interaction

Exercise the shared web UI and real native app where commands, window behavior or startup change. Demonstrate opening a browser after an agent is already running, asking that agent to open a browser, selecting feedback, and reviewing its resulting page changes. Confirm closing the owning Cockpit runtime closes only owned sessions and preserves feedback/profile data according to the chosen policy.

Keep future exact schemas and file ownership in the implementation plan for each increment. Do not add unimplemented protocol methods or a generic plugin framework in advance.

## Alternatives and non-goals

- Pasting only a Playwright session name is useful for handoff but insufficient for agent-triggered creation and safe shutdown ownership.
- Explicitly adopting an independently created session is a possible later capability. It must distinguish borrowed access from permission to close it. Do not auto-adopt arbitrary matching sessions.
- Routing every browser action through Cockpit would restore enforced arbitration but contradicts the chosen loose coupling and duplicates existing tools.
- Embedded Tauri webviews, streamed browser views, personal-profile attachment, a browser-host migration, automatic Playwright installation, and full cross-frame annotation support are not prerequisites for this integration.

## Primary references

- [Playwright CLI README](https://github.com/microsoft/playwright-cli): installation, upstream skills, named sessions, configuration and browser operations.
- [Playwright CLI session management](https://github.com/microsoft/playwright-cli/blob/main/skills/playwright-cli/references/session-management.md): session environment variable, persistent profiles, scoped lifecycle and attachment.
- [Playwright browser connections](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp): CDP attachment and its fidelity limitations.
- [Chrome extension debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger): target identity and debugger capability boundaries.

Upstream documentation establishes available design options, not verified behavior of the user's installed CLI. Pin the supported behavior through the first implementation gate.
