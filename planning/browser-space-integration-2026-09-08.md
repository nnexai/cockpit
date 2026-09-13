# Space-associated browsers and visual feedback

Current direction: superseded by [the inline browser replacement](inline-space-browser-2026-09-13/README.md). The external extension design below is historical. The no-migration inline implementation has focused verification evidence; the broad A01–A25 matrix is not claimed.

Date: 2026-09-08

Status: historical product direction and implementation starting point. The inline replacement supersedes these external-window/extension proposals; the exact focused verification record is in the inline handoff.

## Decision

Use a dedicated external Chromium browser associated with a Cockpit Space. Require a configured, machine-installed Playwright CLI and let agents use its existing skills directly. Cockpit manages the association, a small amount of browser lifecycle, and browser annotations. It does not become a browser automation framework.

Collect drawings and comments directly on the live page through an extension, then capture the page and annotations together. Persist saved feedback in Cockpit. Agents can fetch and acknowledge it through the Cockpit CLI without a manual send; direct annotation-to-pane delivery remains an optional convenience. Both paths use the same pending feedback.

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
| Cockpit core | Resolve caller/Space, maintain browser association, own lifecycle policy, persist drafts/artifacts, expose feedback fetch/acknowledgement and reuse pane delivery |
| Cockpit native/browser clients | Space browser actions, live connection status, feedback overview and direct delivery without an agent picker |
| Playwright CLI adapter | Invoke the configured installed executable for session discovery, launch and scoped close; report capabilities and failures |
| Playwright CLI and upstream agent skills | Page inspection, snapshots, tab operations, navigation, clicks, typing and other browser automation |
| Annotation extension | Live-page drawing, element/region comments, combined capture, authenticated feedback submission and visible failure state |

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
cockpit browser open --current --url http://localhost:5173/checkout
cockpit browser status --current
cockpit browser close --current
```

`--current` resolves the calling Herdr pane to its current Space through supported caller context and fresh authoritative data. It never means the currently selected GUI Space or focused pane of another client. Inherited pane identifiers are lookup evidence, not permanent proof after a pane move.

Outside a resolvable Herdr pane, require an explicit endpoint/session and Space target. Missing or ambiguous identity produces an actionable error. Do not search for an arbitrary browser with a matching title or localhost URL.

`open` must create or reuse one associated live browser per Space, including simultaneous GUI and agent requests. Return a machine-readable association description and a concise normal Playwright invocation. With `--url`, open that URL in the newly launched browser or in a new tab of the existing browser, preserving existing tabs and page state. Without `--url`, do not navigate existing tabs. Return the opened tab's identifier when supported by verified CLI addressing. Browser creation is deduplicated; separate explicit URL-open requests may create separate tabs. Do not blindly retry an uncertain URL-open outcome.

`status` performs live lookup and returns the current session, connection state, and addressing instructions. Do not return stale credentials or claim that a saved association proves the browser is running.

`close` targets only the verified owned session. Stop the runtime without deleting its profile or feedback.

### Agents that are already running

When the user opens a browser from Cockpit:

1. Resolve the selected Space and create or reuse its browser association.
2. Offer **Send browser context** as a direct action without an agent picker.
3. Paste the current Playwright session and lookup instructions into the first eligible agent in the active tab of the active Space, using the delivery rules below.
4. Include the relevant browser context in later annotation batches.

Illustrative context, not a fixed message format:

```text
This Space has a browser available through Playwright CLI.
Session: cockpit-<association-key>
Use playwright-cli -s=cockpit-<association-key> for browser operations.
Use cockpit browser status --current to refresh the association.
Use cockpit browser feedback --current to read saved annotations and image paths.
```

An agent asked to open the browser runs the proposed `cockpit browser open --current`, optionally with `--url`, receives the association, then uses its ordinary Playwright skill. No agent restart or environment mutation is necessary. Opening the browser does not grant that agent exclusive ownership or bind feedback to it.

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

### Confirmed multi-client ownership

A browser has one explicit owning Cockpit runtime. Other clients discover that owner and route browser actions to it; they do not share ownership or automatically transfer it. A second runtime must not launch the same profile.

Closing the last native window shuts down its native-owned runtime and closes its browsers, even if another client is observing them. Closing an observing client does not stop another owner's browser. Closing a web-client tab does not close browsers; explicit shutdown of their owning gateway does.

Agent lifecycle commands contact a running Cockpit owner. If none is available, return an actionable startup error rather than silently starting a background daemon. Crash recovery verifies ownership before reconciling leftovers; immediate cleanup after a hard crash is not guaranteed. The exact owner-discovery transport remains an implementation detail to verify.

## Annotation capture and delivery

The extension submits annotations to a narrow authenticated Cockpit interface. Browser automation bypasses that interface and remains ordinary Playwright CLI usage.

Reuse the existing diff/file comment implementation for draft operations, batch overview, paste framing, and delivery outcomes, but not its agent picker. Optional direct browser feedback and browser context delivery target the first eligible agent in the active tab of the active Space, in authoritative tab pane order rather than attention-queue priority. Resolve and revalidate that destination at delivery time. Do not remember a selected recipient or search other tabs when no eligible agent exists; retain feedback and show an actionable no-agent result.

The browser and its feedback belong to the Space, not to whichever agent opened the browser or received an earlier batch. Deliver only when the active Space matches the feedback's associated Space; do not silently send another Space's feedback after selection changes. Saving a comment does not deliver it. Explicit delivery pastes without submitting Enter.

Rejected or uncertain delivery retains drafts, artifacts, and receipts. Do not automatically repeat a paste whose outcome is unknown. Successful delivery removes the batch from unsent feedback. This is a short annotate/send/continue loop, not a long-term feedback archive.

### Agent feedback retrieval

Proposed commands, not implemented syntax:

```bash
cockpit browser feedback --current
cockpit browser feedback ack --current --id <annotation-id>
```

Fetch returns pending saved annotations, stable IDs, historical page context, current browser status/addressing where available, and readable local image paths. The same authoritative caller-to-Space resolution applies as for lifecycle commands; GUI selection does not determine a CLI caller's target. Feedback remains readable when the associated browser is closed.

Reading does not consume feedback. After reading the comments and their images, the agent explicitly acknowledges the returned IDs. Acknowledgement clears only those annotations from pending feedback and starts their short artifact-retention window. Repeated acknowledgement is harmless; annotations saved after the fetch remain pending. Exact output and multi-ID syntax are implementation details.

CLI retrieval and direct send use the same pending/delivered state, not separate queues or agent-specific inboxes. Fetch does not reserve feedback for one agent. Preserve uncertain paste receipts even if a later CLI acknowledgement clears the corresponding pending annotations; do not claim exactly-once human/agent consumption.

The user can tell the agent to read saved annotations rather than manually pasting them. Include fetch and acknowledgement instructions in browser context. Do not add polling, watchers, automatic agent interruption, or terminal injection when the user saves a capture.

### Live-page annotation interaction

1. Start annotation mode from the extension toolbar, which shows the associated Space and connection status.
2. Draw directly over the live page and add comments anchored to elements or drawn regions. Edit or remove marks and comments in place.
3. Provide an explicit switch between drawing/picking and ordinary page interaction. Drawing mode intercepts pointer input for marks; it does not enforce a lock against Playwright.
4. Choose **Capture and save** to capture the visible page, drawings, and visible comments together. Keep extension tool controls out of the image while preserving the annotations.
5. Show capturing/saving, saved, and failed states. Failed capture or submission retains recoverable draft data; closing the toolbar popup must not discard it.

This is not a screenshot-first frozen-image editor. Capture produces the historical evidence only after the user has annotated the page. Save comment text and available element context as structured data alongside the combined image.

The initial capture covers the visible viewport. Verify scroll, resize, zoom, and page-layout changes so marks/comments cannot silently drift onto unrelated content; where correspondence cannot be preserved, require explicit review or recapture. Full-page stitching and full cross-frame element annotation are not prerequisites.

Before capture, bind unfinished marks/comments to their original tab/document. Reload or navigation must not silently reattach them to a new document or fabricate a screenshot of the lost page state. Retain recoverable draft data with an explicit stale-document state. Once pixels have been captured, navigation must not prevent saving that captured evidence.

### Evidence to capture

- User comment and annotation kind.
- Space/browser association and Playwright addressing context.
- URL, title, capture time and current tab/document identity where available.
- Element tag, visible text, accessible role/name where available, bounded locator candidates and a bounded DOM excerpt when useful.
- Viewport, scroll position, scale information and selected element bounds.
- Combined screenshot of the page with drawings and visible comments, plus drawing geometry mapped to the captured image coordinates. Do not require a separately captured clean screenshot that could represent a different page state.
- Proven local preview/worktree or source association where available; no fabricated source-file mapping.

Do not use transient Playwright snapshot references such as `e15` as durable element identifiers. The agent can take a fresh snapshot and resolve the comment using locator candidates, semantics and captured pixels.

Saved captures are historical evidence of the state the human meant. Preserve captured pixels and comments through reload, navigation, or tab/browser closure, including retrying submission after navigation. Do not require a live target to save already captured evidence or deliver it, silently retarget it, or imply that it describes the current page. Uncaptured live-page drafts follow the stale-document rules above. Historical evidence does not imply long-term retention.

### Images are part of the handoff

Deliver images as real readable local artifact paths alongside the comments. Verify that a supported receiving agent can open the image. Pasting a path is not native image attachment; remote-agent file transfer is outside this integration.

Do not paste image base64 into the terminal. Store artifacts under configured Cockpit-owned storage, not necessarily in a companion directory. Keep unsent and rejected/unknown deliveries recoverable. After successful direct delivery or explicit CLI acknowledgement, retain captures only for a short configurable grace period, then prune them automatically. A fetch alone does not start expiry, and paste acknowledgement must not delete images immediately. Choose and verify the concrete grace period and bounded capture/storage limits during implementation. Browser closure does not trigger artifact deletion, and profile retention is separate from feedback retention.

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

Direct delivery follows existing annotation batch conventions. CLI output additionally exposes stable IDs for acknowledgement. Neither path includes extension credentials or raw debugging endpoints.

## UI direction

Proposed placements, not production UI commitments:

- Space action/context menu: **Open browser**, **Show browser**, and scoped **Close browser**.
- Commands or equivalent existing action area: **Send browser context** and **Browser feedback** with a draft count.
- Browser feedback overview: reuse the existing annotation overview with a direct send action, no agent picker, no persistent history browser, and no new permanent bottom panel.
- Extension toolbar popup: associated Space, connection/error status, and entry into live-page annotation mode. Draw and edit comments on the page, then **Capture and save** the combined result; no frozen-image editor or agent picker.

Keep the external window association obvious without automatically stealing focus whenever the user selects a Space. Explicit **Show browser** is the initial behavior. Automatic window-following is optional and must be tested on the native window system before being promised.

Human and agent share browser control cooperatively. The intended loop is for the human to navigate to the relevant page, reproduce an error or reveal a state, draw and comment on that page, capture it, and ask the agent to fetch the saved context. Optional direct send supports the same conversation. Do not show an enforced "agent input blocked" guarantee or add a control-grant workflow. Direct CLI access bypasses Cockpit. Independent agent work can use separate browser sessions when interference would matter.

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

The spike did not prove installed `playwright-cli` session discovery, CLI-managed extension launch, production annotation-to-pane image delivery, production feedback fetch/acknowledgement, live-page drawing/comment composition, or production shutdown ownership. Its custom control-gate proof does not transfer to direct CLI access.

Two implementation lessons remain relevant: same-origin browser GET requests may omit `Origin`, and Chromium cached generated extension settings when the unpacked extension path was reused. The spike used fresh runtime extension paths; production should evaluate stable extension identity with explicit pairing and credential rotation rather than assume that workaround is the final design.

## Implementation sequence and acceptance

Each increment needs a separate verified commit. Exercise Herdr mutations and browser automation only in uniquely named disposable resources. Never use the user's active Space or personal browser for acceptance.

### 1. Verify the external tooling contract

Use the configured installed Playwright CLI to launch a named, headed Chromium session with the annotation extension and dedicated profile. Address it from different working directories and from an already-running terminal. Verify scoped close, profile reuse and reconnect. Record the supported CLI version and the exact session-addressing requirements. This is the first gate because the spike used a different automation entry point.

### 2. Integrate Space discovery and owned lifecycle

Implement the minimal open/status/close interface and GUI Space actions through the existing Cockpit core/client architecture. Verify GUI and agent-triggered open, URL-on-open for new and existing browsers, preservation of existing tabs/page state, simultaneous browser-open deduplication, rename, pane movement, missing browser, normal shutdown, multi-client ownership, crash recovery, and refusal to close unrelated sessions. Existing agent terminals must work without new browser environment variables.

### 3. Integrate live-page capture and feedback consumption

Connect the extension to Cockpit's durable draft/artifact model. In the real browser, draw and comment on the live page, edit/remove annotations, and capture the combined viewport with annotations visible and tool controls excluded. Verify scroll/resize/zoom alignment, layout changes, popup closure, duplicate URLs, reload/navigation before and after capture, browser closure, and failed capture/submission with retained drafts.

Verify a real agent fetches saved feedback through the CLI, reads the image artifact, acknowledges exact IDs, and uses ordinary Playwright CLI against the intended browser. Verify repeated fetch does not consume, repeated acknowledgement is safe, newer annotations survive acknowledgement of an older batch, closed-browser feedback is readable, and acknowledged artifacts survive until their configured expiry.

Also verify optional direct delivery to the first eligible agent in the active tab without a picker, the no-agent/wrong-Space failure states, failed/unknown paste retention, and shared pending state between direct delivery and CLI retrieval. Do not modify file/Review recipient selection in this increment.

### 4. Verify the daily-use interaction

Exercise the shared web UI and real native app where commands, window behavior or startup change. Demonstrate opening a browser after an agent is already running and asking that agent to open a specific URL. The human then navigates or reproduces an error, draws and comments on the page, and captures everything together. The agent fetches and reads that evidence without manual send, acknowledges it, and makes resulting page changes through ordinary Playwright CLI. Repeat with a fresh annotation batch and exercise optional direct send. Confirm closing the owning Cockpit runtime closes only owned sessions and preserves feedback/profile data according to the chosen policy.

Keep future exact schemas and file ownership in the implementation plan for each increment. Do not add unimplemented protocol methods or a generic plugin framework in advance.

## Alternatives and non-goals

- Pasting only a Playwright session name is useful for handoff but insufficient for agent-triggered creation and safe shutdown ownership.
- Explicitly adopting an independently created session is a possible later capability. It must distinguish borrowed access from permission to close it. Do not auto-adopt arbitrary matching sessions.
- Routing every browser action through Cockpit would restore enforced arbitration but contradicts the chosen loose coupling and duplicates existing tools.
- Embedded Tauri webviews, streamed browser views, personal-profile attachment, a browser-host migration, automatic Playwright installation, and full cross-frame annotation support are not prerequisites for this integration.
- Future improvement, explicitly outside this integration: replace the file/Review annotation recipient picker with a direct action and shortcut that sends to the first eligible agent in the active tab of the active Space. Preserve paste-without-submit and failed/unknown-delivery recovery. Do not change those existing workflows as part of browser integration.

## Primary references

- [Playwright CLI README](https://github.com/microsoft/playwright-cli): installation, upstream skills, named sessions, configuration and browser operations.
- [Playwright CLI session management](https://github.com/microsoft/playwright-cli/blob/main/skills/playwright-cli/references/session-management.md): session environment variable, persistent profiles, scoped lifecycle and attachment.
- [Playwright browser connections](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp): CDP attachment and its fidelity limitations.
- [Chrome extension debugger API](https://developer.chrome.com/docs/extensions/reference/api/debugger): target identity and debugger capability boundaries.

Upstream documentation establishes available design options, not verified behavior of the user's installed CLI. Pin the supported behavior through the first implementation gate.
