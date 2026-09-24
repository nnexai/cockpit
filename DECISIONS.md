# Cockpit Architecture Decision Snapshot

Status: confirmed design decisions. Current runtime and planned features are distinguished in `CONTEXT.md` and `planning/next-level/README.md`.

This records the decisions made during the architecture refinement interview. It intentionally avoids example filesystem locations; all roots and endpoints are configurable.

## Product shape

- Cockpit remains the umbrella product.
- Forge, Issue Tracker, and Wiki are provider-neutral domains inside Cockpit, not separate products.
- The repository contains at least two major client-side/backend concerns:
  - a Rust Cockpit core and CLI;
  - a Tauri v2 Herdr client with a shared web frontend.
- Provider implementations are staged. Broad capability-based interfaces come first; unsupported capabilities must be explicit rather than silently approximated.

## Runtime and platform

- Linux is the first native target.
- Native distribution uses Tauri v2.
- Browser distribution is a separate deployment of the same static frontend, not a Tauri browser target.
- Native and browser clients share a versioned protocol and injected client interface.
- The native client uses Tauri IPC/channels.
- The browser client uses HTTP/WebSocket through a standalone local gateway.
- Browser/native behavior is verified through protocol contract tests plus a real native smoke test.
- Browser/test operation has an explicit deterministic test mode; normal startup uses Herdr’s default session with a selector.

## Shared core and package boundaries

- The core is reusable and written in Rust.
- Package boundaries are:
  - versioned protocol types;
  - application/core services;
  - Herdr and provider adapters;
  - CLI and standalone gateway hosts;
  - Tauri/native and browser client adapters.
- Tauri handlers remain thin transport adapters; business rules do not live in the UI host.
- The browser gateway is provided by a dedicated `cockpit serve` process, binds to loopback, and initially has no separate browser authentication mechanism. Remote access is out of scope.
- The gateway exposes task-level operations, not raw shell or raw socket forwarding.

## Herdr authority and transport

- Herdr-server is authoritative for sessions, Spaces/workspaces, tabs, panes, PTYs, processes, agent state, and Herdr-owned metadata.
- Cockpit has no separate session/workspace registry. Herdr metadata and provenance are authoritative.
- Cockpit uses Herdr’s documented newline-delimited JSON socket protocol for persistent runtime interaction and long-lived event subscriptions.
- Herdr CLI wrappers remain available for simple one-shot operations, human debugging, setup, and portability-sensitive cases.
- The installed Herdr schema/version is checked at startup. The adapter is schema-gated and maintains explicit capability/compatibility handling.
- Blind undocumented protocol probing is not the design. Runtime discovery means reading the supported schema/capability surface and validating required operations.
- The documented setup command for native OMP integration is a separate bootstrap concern, not part of each workspace creation.
- Herdr named sessions are supported through a top-level session selector. Only one session is selected at a time.
- Switching sessions detaches old streams, connects to the new session, loads an authoritative snapshot, subscribes to events, and discards stale selection state.
- Client state uses an authoritative snapshot plus ordered events. Sequence gaps, reconnects, or stale state trigger resnapshot/resubscription.

## Herdr client UI

### Primary surface

- The first screen uses the Herdr TUI as its semantic and operational baseline, not as a skin to reproduce indefinitely.
- Preserve Herdr’s resource model, hierarchy, focus semantics, attention ordering, terminal behavior, and supported operations before introducing intentional Cockpit departures.
- Cockpit may depart from the TUI where a graphical desktop workbench improves supervision, discoverability, direct manipulation, or ownership clarity. A departure must not create a second authority for Herdr-owned state.
- The sidebar has:
  - scrollable hierarchical Spaces at the top;
  - an Agents section below, acting as Herdr’s attention queue.
- Space rows show the branch and its upstream position (`main ↑14`) like Herdr's TUI. Herdr's API does not expose the counts, so Cockpit reads them from Git for the checkout paths in Herdr's snapshot only, and the frontend polls every 15 seconds while visible because commits produce no Herdr event.
- The main area shows tabs for the selected Space and fills available space.
- Spaces and agent/process entries use Herdr-native terminology and identifiers at the UI level where practical.
- The initial Cockpit foundation is the real Herdr mirror plus terminal attach/input, Herdr-semantic focus/layout controls, schema-gated socket access, and a working `cockpit serve` browser gateway.

### State and inbox

- Spaces and agents mirror Herdr entities directly.
- Agent state is Herdr-authoritative: blocked, working, done, idle, and related Herdr states where available.
- State indicators include transition detail/freshness where available and do not rely on color alone.
- The persistent Agents sidebar follows Herdr priority ordering: blocked, done, working, idle, then unknown; equal-priority entries use newest state transition first.
- There is no separate inbox popup in the initial scope.
- Agent history/resumable closed conversations are deferred.
- Inbox/triage mutations are performed through Herdr; Cockpit does not persist local acknowledgement state.

### Navigation and interaction

- Semantic focus, DOM keyboard focus, and writable terminal ownership are distinct state. Cockpit synchronizes them deliberately rather than inferring one from another.
- Clicking a Space, tab, pane, or agent sends the corresponding Herdr focus operation. Confirmed selection follows Herdr’s response/event; agent selection resolves to its owning pane.
- A local pane selection or terminal click records local control intent. An authoritative focus change or ownership loss from another client clears that intent.
- Space hierarchy supports direct tree operations, including selection, expand/collapse, create, rename, reparent, reorder, close, context menus, and drag/drop where Herdr supports them.
- Tab/pane layout editing supports Herdr-backed core operations.
- The interaction authority is Herdr semantics with GUI presentation. Space tree operations support native Herdr mouse behavior where available; pane/layout actions are primarily shortcut-driven.
- GUI controls remain discoverable, but the first shortcut set is limited mainly to Herdr-style navigation. The Herdr magic escape key has priority over GUI shortcuts.

### Terminal attachment

- Herdr owns the PTY, process, and terminal model. xterm.js owns rendering and input capture only.
- The supported contract is Herdr protocol 22, schema 1, and the adapter's required methods. The display version is diagnostic identity, not a patch-release allowlist. Each visible pane uses a direct ANSI `TerminalHello` stream with `ControlTerminal` or `ObserveTerminal`.
- Stable `TerminalFrame` ANSI bytes are authoritative. The first frame is full; every later sequence must be consecutive. Full repaints never reset xterm.
- The JSON API remains authoritative for hierarchy, focus, and layout. One bounded reader owns each framed terminal socket without cancellation between its header and payload.
- xterm.js renderers are mounted only for panes visible in the selected tab. Hidden tabs detach renderers/subscriptions while Herdr processes continue running.
- Cockpit fits each pane before attachment and sends character dimensions and measured cell pixels.
- Text/binary input uses stable raw `Input`; wheel/page scrolling uses `AttachScroll`, gated by local control intent and attachment state. Normal xterm.js panes attached to Herdr are also observed to receive wheel/scroll events.
- Application mouse demand follows Herdr's per-attachment `MouseCapture` signal. Cockpit sends structured `AttachMouse` cell coordinates rather than raw SGR injection. Herdr owns the application encoding and tracking checks. Mode-off and Shift-drag preserve text selection; click-to-focus remains available.
- Terminal graphics are parked and the image addon is removed. Known auxiliary/graphics traffic is bounded and consumed without disconnecting text terminals. Enhanced keyboard reporting is a separate capability to revalidate.
- Control and observe requests use stable attach modes. Semantic focus, local control intent, attachment state, and process closure remain independent.
- Attach/reconnect failure leaves the resource visible with stale/disconnected status, retry, and resync; the Herdr process is not silently closed.
- Errors are inline on the affected resource. Toasts may supplement but are not the only error surface.

### Context and review replacements

- Detect supported Herdr extension panes and replace the renderer with a complete Cockpit GUI. Context/files replace `herdr-file-viewer`; local review replaces `persiyanov.reviewr`.
- Herdr owns the actual pane, process, tab/Space membership, focus, and layout. The user moves/resizes it through normal Herdr operations. No separate dock layout or synthetic Context tab is introduced.
- Cockpit owns the GUI file/review model and drafts. It does not communicate with, scrape, or synchronize the extension's private state. No extension or Herdr-server change is required.
- Use plugin-open provenance and bounded current process evidence for automatic detection. Ambiguous panes retain terminal rendering with an explicit per-pane renderer choice. Titles alone do not decide replacement.
- The original extension keeps running independently. Terminal fallback reveals its own state; GUI drafts remain in Cockpit. A future TUI backport is a separate reusable-core story.
- Context renders Markdown with Mermaid, frontmatter, bounded source/text/logs, and safe images. It discovers user files, watches changes, and uses bounded core-mediated ripgrep.
- Whole-file and selected-line comments can be collected across files, edited, removed, and previewed as one batch. Full-file comments carry path/comment; line comments also carry exact original lines and line numbers.
- Paste targets an explicitly selected agent in the same actual tab. It never submits Enter. A dedicated acknowledged task uses the existing public Herdr byte-write capability with proven paste framing; ordinary terminal input stays on the stable per-pane attach path. Pending, rejected, and unknown outcomes retain drafts/receipts and never cause automatic duplicate retries.
- Full local graphical review is a separately selectable feature using Cockpit's own read-only Git model and shared comment behavior. Review/MR URLs are setup inputs, not Build/Review workbench modes.

### Accessibility and settings

- No settings UI is planned initially. Configuration is file/environment/launch-option driven.
- Accessibility is currently a best-effort goal for this personal proof of concept rather than an explicit release gate for broader distribution.
- The design should still preserve keyboard access, visible focus, usable labels, non-color-only state, and actionable inline errors where practical.

## Workspace and filesystem lifecycle

- Workspace creation starts by enumerating repositories under a configured default root.
- Creation resolves a typed task/artifact, asks Herdr’s worktree API to create/open the worktree workspace, creates the Cockpit-owned companion context, records verified provenance in its owned manifest, passes context env to explicit new Cockpit tabs/panes, and returns the selected session/resource.
- Workspace lifecycle uses Herdr’s worktree API for creation/open/removal and Herdr worktree provenance for mapping.
- The companion resource is Cockpit-owned. Its manifest records context ownership and verified Herdr/repository provenance; it is not a second registry of live Herdr workspaces. Display metadata is not used as arbitrary durable context storage.
- Partial provisioning artifacts are allowed. Destroying a newly created space/workspace must also remove its owned companion context.
- Branch and workspace-location templates are configurable and use task artifact metadata where available. A review artifact may already identify its source branch.
- Configuration uses a durable config file with environment-variable overrides and one-off command options.
- Reflink copies are preferred for independent local repository snapshots. Normal copying is a correctness-preserving fallback and reports its copy mode. Hardlinks and Git alternates are prohibited for writable context snapshots.
- Central normalized cache assets are replicated into companion contexts. Sync replaces changed copies atomically.

## Terminal environment

- Worktree create/open has no env argument in the inspected version. Context env is passed explicitly to subsequent Cockpit-created tabs/panes; the initial root pane, existing processes, and direct TUI-created terminals are not guaranteed to inherit it.
- Automatic OMP setup/launch is out of scope for now.
- Initial workspace setup creates/registers a Herdr session only; it does not require a predefined three-surface layout.
- Cockpit does not mutate Herdr metadata for bookkeeping. Existing Herdr-provided metadata is inherited; Cockpit adds only approved context/workspace environment to its explicit process launches. Renderer detection uses supported Herdr inspection and launch receipts, not an extension handshake.
- Credentials are not abstracted or managed by the client initially. External wrappers/tools retain responsibility for credential handling. Cockpit must not persist or export secrets through snapshots or generated environment values.

## Context ingestion and snapshots

- Creation hydration is explicit, with bounded typed dependency expansion.
- Automatic expansion may include an immediate parent and recognized local/provider references in descriptions/comments, with deduplication, cycle detection, and depth/count limits.
- Cockpit pulls and validates freshness. Users and agents perform remote writes through provider CLIs or their normal tools.
- Provider adapters decide what to download and how to transform it.
- Normalized snapshots use a versioned canonical Markdown/frontmatter envelope.
- Required frontmatter covers schema version, provider, resource type, canonical identifier, source URL when available, fetched time, and source revision/hash metadata when available.
- Freshness prefers adapter-provided source version/ETag/revision/hash metadata, then canonical content hash as appropriate.
- The current context browser discovers the companion tree plus frontmatter. There is no generated `INDEX.md` or scratchpad in the current scope.
- The first concrete adapter is Gitea issue context via a configurable CLI, with `tea` as the default when available. It includes issue metadata, description, labels, milestones, assignees, comments, timestamps, source identifiers, and recognized bounded references.
- Gitea provider interfaces are capability-detected and return explicit unsupported-operation errors.
- Provider failures leave successful assets in place, record per-asset failure status/retry information, and allow the workspace/session to proceed when the primary resource is available.

### CLI surface

- The initial user-facing CLI provides repository discovery, Herdr session connectivity/mirroring, workspace create/destroy, context hydration hooks, and `serve` for the browser gateway.
- `cockpit serve` is an explicit foreground process serving the static browser frontend plus the versioned HTTP/WebSocket API from one loopback origin, with explicit test mode. Native Tauri access uses Tauri capabilities; separate browser authentication is deferred.
- Provider-specific mutation commands are deferred.

## Deferred scope

- Automatic OMP setup and launch.
- Provider implementations beyond the first Gitea issue pull/freshness slice.
- Full Forge review-assistant ingestion for GitLab/GitHub/Gitea.
- Full Issue Tracker and Wiki provider matrix implementations.
- Agent history and resumable conversations.
- Separate inbox popup/views and advanced inbox title/history behavior.
- Cockpit-managed secrets, passkey integration, and key-store injection.
- Remote browser access and multi-user authorization.
- A Cockpit-owned settings screen.
- Unbounded file previews, arbitrary shell execution, raw socket forwarding, and custom search implementation.

## Known risks

1. **Compatibility is a hard runtime boundary.** Protocol 22/schema 1 and all required methods are checked before attachment; compatible patch releases are accepted without exact display-version matching. The historical protocol-22 client-shell/graphics implementation remains separate.
2. **Mouse autodetection is verified; broader terminal acceptance remains scoped.** Browser and Linux-native mode transitions, press/drag/release, wheel, text selection, and ownership handoff are verified. Exact pixel coordinates, idle hover forwarding, Kitty graphics, and the full G01 temporal matrix are not part of this migration.
3. **Accessibility is intentionally best effort for this personal proof of concept, not a gate for broader distribution.**
4. **No index/scratchpad means context membership is derived from the companion tree and frontmatter; human-created files are displayed but not managed by Cockpit.**
5. **Herdr “Space” UI labels map to Herdr API “workspace” resources.** The adapter must keep this translation explicit.

## Evidence used during refinement

- `CONTEXT.md` in this repository: initial architecture and workflow proposal.
- Tauri v2 documentation: https://v2.tauri.app/start/ and related architecture/configuration/security pages.
- Herdr socket API: https://herdr.dev/docs/socket-api/
- Herdr source repository: https://github.com/SuperCodeAgents/herdr-terminal
- Herdr Agent Inbox source: https://github.com/douglascorrea/herdr-agent-inbox

## Personal workflow and maintainability, 2026-09-04

- This is the user's primary local project workbench, not a product targeted at other users. Code changes should be the easy way to tweak behavior.
- Add a separate maintainability pass before the new features: preserve current behavior, identify owning modules, simplify focus/input/lifecycle seams, consolidate styling, and document where to change common behavior.
- Keep broader follow-up cleanup selectable after features. Avoid a wholesale rewrite, arbitrary file-size goals, a generic workflow engine, or a dynamically loaded frontend plugin framework.
- Plan every missing/deferred capability as a separate story, but do not make distribution, multi-user access, credentials infrastructure, or provider breadth prerequisites for the local main loop.
- A local primary repository is required for setup; an issue/review URL is optional. Additional already-discovered local repositories can be explicitly snapshotted into context. Arbitrary URL downloading and remote cloning remain outside the current main scope.
- The selected UI supersedes the first HTML dock study. Final plans, dependencies, code cleanup, and graphical pane mocks are in `planning/next-level/README.md`.

## Agent quality feedback, 2026-09-04

- The maintainability pass must establish deterministic test and metric feedback for later code-writing agents, including bounded Luna-high implementation work when contracts and patterns are clear. This task plans that infrastructure only.
- CRAP 6–8 is the user-proposed local target, not a quoted industry standard. Proposed enforcement: new functions at most 8, preferred target 6; touched legacy code must not regress against an explicit reviewed baseline. Missing per-function coverage mapping is inconclusive.
- Add mutation testing alongside complexity and coverage. Preserve outcome categories and expose survivors, uncovered mutations, timeouts, invalid mutants, skips, and tool failures. Do not count every non-survivor as a successful kill.
- Pin tools and inputs, provide actionable stable reports, and keep baseline/exception changes reviewable. Metrics support behavioral tests and design review; they do not certify maintainability by themselves. See `planning/next-level/11-quality-gates.md`.
- The second workflow mock proposes direct explicit paste to the visible same-tab agent, with optional preview/always-preview. This remains a discussion proposal until the interaction checkpoint resolves it.

## Comment interaction checkpoint, 2026-09-04

- Retain the quick comment interactions. Remove the persistent comments bottom panel: show an unsent-comment count in the Context/Reviewr pane header and open the batch overview only on click or a GUI shortcut.
- Render unsent selected-line comments inline; whole-file comments or rendered-document comments may appear below the file. These views and the overview share draft state. Sent comments leave the unsent views; rejected/unknown delivery retains them.
- Pane/ellipsis controls and source setup need later refinement. Their current mock appearance is not a chosen final design.

## Reopened terminal stability decision, 2026-09-04

The user reports repeated whole-view dark frames during redraw, especially active agent output and Codex `/pets`, with Kitty images making the regression prominent. Protocol 22 and the current renderer describe the installed implementation; they are no longer assumed to be the correct stable baseline for future work. Preserve Herdr authority, but compare a bounded presentation repair with a verified stable Herdr-compatible path. Keep Kitty graphics as a goal without requiring a flickering daily-use default. No running server or installed binary is changed by this planning decision. The evidence and acceptance gate are in `planning/next-level/13-terminal-stability.md`.

The final code review also calls for separately verified state-ordering, stream-transition, input/attachment, and request-lifetime repairs before behavior-preserving cleanup. Future quality metrics must exercise production state paths, not duplicate unused reducers. See `research/next-level-existing-code-review.md`.

Scrolling is included in that first stability gate: viewport continuity, scroll position during incoming output, responsiveness, and flicker with and without images. It is not deferred as UI polish.

## Selected stable Herdr target, 2026-09-04

The user chose to give up the custom protocol-22 path for now, preserving it in committed history, and explicitly requires retaining mouse click handling. Stable Herdr is the default target. Commit `7e8fe25546ce5fa9364cab100522af6d63343e4a` preserves protocol-22/TGP work; `34459ab` and `582792e` contain pre-22 input/mouse work to evaluate and retain. Verify stable capabilities instead of assuming a historical protocol version. Required migration gates distinguish CLI-injected SGR and normal xterm.js wheel delivery from native/browser pane clicks/focus, application-mode mouse events, and scrolling quality. Terminal TGP may remain parked; Context GUI Markdown/Mermaid/images remain in scope. No current installation or running server is changed during planning.

## Orchestrator startup constraint

The user will downgrade the Herdr default session to stable before starting the implementation orchestrator. Astra runs inside that default session; it must never restart, upgrade/downgrade, close, or send test input to it. The current frontend will initially be incompatible, so BOOT-01 restores the actual stable protocol/transport first, then runtime smokes run only in explicit disposable sessions. A current-frontend or old protocol-22 smoke is not a bootstrap prerequisite.

## Stable transport bootstrap

The implementation run `run-20260904T214621Z` pins the installed Homebrew Herdr and preserves the protected default session. The shadowing local executable was archived only at the user's explicit request; neither the default server nor shared live configuration was changed.

Browser and native AppImage smokes rendered the stable ANSI fixture and delivered byte-exact input to their recorded disposable panes. Source review corrected partial-read cancellation, repaint resets, nonconsecutive full-frame acceptance, and auxiliary-message disconnects. These checks establish bootstrap behavior, not full migration or release acceptance.

The user selected WebUI-first behavioral verification and native AppImage startup/simple compatibility checks for this run. Required browser temporal, scrolling, focus, and application-mouse assertions remain in scope. The old “missing stable mouse API” wording is superseded: the direct-attach physical-pointer path is still unproven, but stable Herdr 0.8.2 SGR delivery is demonstrated through `pane.send-keys`, and normal xterm.js wheel delivery is observed. A remaining app-mode or native pointer gap is an acceptance gap, not proof that SGR input is impossible.

On this Fedora host, linuxdeploy's bundled `strip` cannot parse `.relr.dyn`. The supported `NO_STRIP=1` packaging option preserves the ELF data instead of applying the incompatible rewrite; it does not bypass compilation or runtime smoke. The resulting AppImage is stored only with run-owned artifacts, not installed.

## Finite Herdr request recovery

Finite socket requests have separate 500ms connect/write limits and a 2s response limit. Commands have a 5s total limit; output overflow stops both capture paths and kills/reaps the owned process group. Live terminal streams retain their separate lifetime.

A mutating request that may have reached Herdr is not safe to retry automatically. The browser offers Resync instead of Retry for `request_outcome_unknown`. Cancellation does not imply rollback. Dropping a Rust future triggers resource cleanup but cannot return an error to that dropped caller; durable cancellation/dispatch reporting remains an explicit open contract.

## Ordered recovery and terminal input

Mutation responses acknowledge the operation; only a newly ordered session stream grants fresh focus/control authority. Session changes, removed-session fallback, and newer list refreshes invalidate older callbacks. Browser and native session adapters share one stream-order policy and fail closed on gaps.

Workbench prefix commands are consumed before xterm input. Normal xterm pointer events remain available when Cockpit's separate structured-mouse capability is disabled, restoring text selection; wheel/scroll events are observed through the normal xterm.js pane path. Input during reattachment is not yet guaranteed; attachment cancellation and queued-input ownership remain REPAIR-03 work.

Installation and per-session compatibility probes commit only within their captured invalidation generations. Event subscriptions retain their initial identity across reconnects and terminate on an identity mismatch so a new subscription must re-inspect it.

The stock Herdr 0.8.2 source review says that the physical host-mouse path used by `herdr terminal attach` captures wheel for `AttachScroll` but drops non-wheel reports. This remains a scoped explanation of the Ghostty/X11 experiment. It does not cover `pane.send-keys`, `pane.send-text`, or the normal xterm.js wheel path, and it does not prove a blanket Herdr/PTy inability to carry SGR bytes.

On 2026-09-06, `scripts/verify/send_mouse_sgr.py` invoked `herdr pane send-keys` against the running `MOUSE-FEEDBACK-READY` pane with `ESC[<0;30;9M` and `ESC[<0;30;9m`; the pane reported `mouse 2: release button=0 x=30 y=9 wheel=False`. The user also reports working wheel/scroll delivery through ordinary xterm.js panes attached to Herdr.

The application-mouse decision is therefore revised: remove the categorical upstream block. First investigate Cockpit/native pointer interception, focus, ownership, and coordinate routing. If physical forwarding remains unavailable, an explicit ownership-gated emulator using `pane.send-keys`/`pane.send-text` is a demonstrated fallback. Neither workaround marks structured app-mode coordinates, drag/release, native/browser parity, or G01 complete.

## Mouse-input correction, 2026-09-06

The stable input matrix now records separate paths: physical host mouse through direct attach (historical non-wheel negative), one-shot CLI SGR injection (verified), normal xterm.js wheel/scroll delivery (user-observed), and Cockpit structured app-mode pointer routing (still to trace). Future plans must name the path under test instead of collapsing all of them into “mouse unsupported.”

Prioritize repository/worktree setup and graphical Context, then the remaining selected reference, source, and review workflows. Outstanding terminal/repair gates remain recorded; they do not authorize calling the release complete, but they no longer hold independent feature implementation behind additional terminal experiments.

## Reviewed repository and worktree setup (historical)

The path-only opening and repository-action policy below supersede this increment's branch-based Open selector and per-operation consent. Its evidence describes the implementation at that time.

The `task…` entry point discovers only configured local repositories, reviews exact create/open effects, then delegates worktree mutations to stock Herdr. Open resolves exactly one branch or checkout-path selector from fresh inventory. Cockpit's repository ID is a filesystem-bound catalog identity; Herdr's repository key is the canonical Git common directory. They are not interchangeable.

Inspect effective configuration with `cockpit configuration --config /absolute/cockpit.toml --repository-root /absolute/repositories`. The same options are accepted by `cockpit serve`. Configuration precedence is invocation, environment, versioned TOML, then defaults. `COCKPIT_REPOSITORY_ROOTS`, `COCKPIT_WORKTREE_ROOT`, `COCKPIT_COMPANION_ROOT`, and `COCKPIT_STATE_ROOT` select local roots; no repository is cloned.

Stock Herdr cannot suppress configured repository actions, so planning requires explicit per-operation consent. The reviewed endpoint identity binds the socket's connected process identity, not merely its pathname. Unknown dispatch outcomes are not automatically retried. Journals use descriptor-owned execution leases and transactional updates; companion publication is no-replace and currently supported on GNU/Linux.

Worktree creation returns a root pane, not a context-bearing process. Only a separate successful terminal creation supplies the operation's context tab/pane receipt. That new terminal receives allowlisted `COCKPIT_*` variables; the original root pane remains visible and unchanged. Opened checkouts remain borrowed even when Cockpit creates a new companion and context tab.

Browser acceptance created a linked checkout, reopened that exact checkout without creating another workspace, and resumed a companion-write failure after restarting the gateway. The terminal fixture recorded the expected context environment; its root-pane control recorded none. Teardown/removal, graphical Context rendering, and the remaining source/reference/review workflows are not covered by this increment.

## Graphical Context

The pane and command menus expose **Open Context right/below** from a pane already at its verified companion directory. Open uses the installed `herdr-file-viewer` entrypoint through stock Herdr; it does not install plugins, create a synthetic dock, or inject input into the existing terminal. Detection and replacement eligibility are separate: a file viewer becomes graphical only when its proven browsing root is exactly the verified companion. Repository-root, merely associated, and unknown-root viewers remain terminals; **Render as Context** cannot bypass this check. Verified Reviewr selects Review without a companion restriction; the full Review UI remains a separate increment.

Stock Herdr 0.8.2 starts the plugin's relative command from its install directory and supplies file context from the target pane. Split requests therefore omit both `cwd` and `workspace_id`. Context detection requires verified executable/process-generation evidence or a generation-pinned launch receipt. On Linux, executable matching uses `/proc/<pid>/exe`; bounded inspection reads only the documented `HERDR_PLUGIN_CONTEXT_JSON` entry and pins its process generation. The root follows the installed viewer's Git-toplevel-or-cwd rule, never an inferred workspace checkout. Unavailable metadata fails closed; pre-existing viewer adoption on other platforms remains unsupported. Graphical file selection is independent of the TUI's private state.

Context exposes the configured checkout and freshly verified associated companion, including user-created files. Directory and document reads are descriptor-relative and do not follow symlinks. Git internals, companion metadata/staging files, special files, binary/non-UTF-8 content, stale revisions, and oversized previews receive explicit refusals. JSON paths address literal filenames; percent sequences are not decoded.

The TOML `[limits]` keys are `context_preview_bytes` (1048576), `context_preview_lines` (5000), `context_directory_entries` (1000), and `context_tree_depth` (32). Source preserves original lines and frontmatter; derived GFM rendering maps block selection back to source lines. Raw HTML does not execute, images do not fetch automatically, and unsafe links are inert. Manual Refresh rereads files; renderer detection polling does not. Terminal/GUI switching retains bounded per-file view state while identity changes invalidate stale requests.

Browser proof exercised real plugin splits, repository/companion browsing, source-to-Markdown line mapping, refusal states, literal filenames, and manual refresh. Repository-root viewers stayed terminals and could not bypass the companion gate. A companion-root viewer was adopted after gateway restart without a launch receipt; removing and restoring its owned companion caused terminal fallback and graphical recovery. A real Linux Tauri window verified the same root policy and source/GFM document commands. The disposable session and clients were stopped and their fixture root removed; default Herdr and the manual gateway were untouched. This does not complete image/Mermaid rendering, file watching/search, reference comments/paste, source hydration, snapshots, or graphical review.

## Local Review and action placement clarification, 2026-09-05

GUI Review is available for any local Git checkout, including ordinary Herdr workspaces and linked worktrees outside the configured task catalog. The checkout comes from fresh authoritative pane working-directory evidence, with filesystem-bound repository identity and revalidation. Task associations add context; they are not a prerequisite for reviewing staged, unstaged, untracked, or branch changes. This does not add PR/MR operations or broaden companion write authorization.

Following the workflow mock and the user's explicit placement choice, Pane and Commands actions belong on the right of the tab bar, including when a single pane has no label. Task setup uses the plus button in the Spaces heading. Herdr still owns pane creation, focus, and layout; the controls expose those operations without requiring a visible pane header.

## Review interaction and ordinary file viewers, 2026-09-05

Review keeps old/new and expanded-source modes, with visible line-range selection, keyboard file/hunk navigation, and an inline comment editor beside the selected lines. The comments overview remains available. The extra explanatory/TUI-switch header is removed; terminal fallback stays in the Pane menu. Paste preselects the first eligible agent returned for the current tab and retains a valid explicit choice. Saving a comment does not paste or submit it.

Review's bottom status bar shows the active side/range, comment shortcuts, and the comments-overview button. The former top comment toolbar is removed for Review. Mouse selection and keyboard hunk/line movement update one active range; C opens a line comment and Shift+C a whole-file comment. Comment actions are also clickable from the status bar.

Comment capture and source-state checks reuse fresh Review evidence within one request, then revalidate it before saving. Evidence is not cached across requests. Extension inspection reads endpoint-pinned pane structure without requesting optional Git summaries for unrelated Spaces.

Verified `herdr-file-viewer` panes may render any safe browsing folder without task setup. The root follows the viewer's proven browsing cwd and Git-toplevel-or-cwd behavior, never the plugin install directory. Ordinary Folder roots use the existing bounded read and no-symlink protections. Open Context still targets the companion; companion comments, imports, and other companion-specific actions retain their existing authorization.

- Review comments use the bottom status bar for line/file actions and the overview count. The overview keeps its sidebar placement, uses compact sans-serif controls, and puts recoverable batches in a collapsed section with explicit discard. Exported comments follow Reviewr's concise path/range, source excerpt, and message format; capture revisions and IDs remain internal. Discard uses generation checks and durable deleted-ID markers so stale editors cannot recreate removed batches.
- Changed-file trees compress single-child directory chains, indent branches, fold single-file paths into leaves, and preserve filename endings. Per-file additions/deletions replace repeated scope labels; index/working labels remain where a path has distinct staged and unstaged entries.

- File-viewer comments use the same inline editor, bottom status actions, and recovery/paste workflow as Review. A verified ordinary browsing folder can own comments without a task companion. Companion ownership identities remain compatible; ordinary folders use the verified filesystem root identity, and mutations revalidate that identity. Companion-only import/snapshot operations remain separate.
- Comment editors keep Enter for newlines and use Ctrl+Enter or Cmd+Enter to save. Pane keyboard navigation follows visible tree order and source lines; text editors retain their own keys. Task setup autofocus runs on opening, not on parent status updates.
- Markdown uses the available document width. Saved scroll positions restore when entering a document, not on every scroll-state update. HTML preview is static and isolated: styles render in a sandboxed iframe, scripts/forms/navigation are disabled, and external resource requests are blocked.

- Pane and Commands menus expose Open files right/below beside Review. Files launch the installed file viewer at the selected pane's verified Git root or working directory without task setup. Open Context retains its companion-only launch meaning; both actions use Herdr's existing plugin pane operation and Cockpit's bounded root checks.

- The native window defaults to WebView scale 1.0 with toolkit decorations enabled on every platform. Shared Cockpit configuration accepts `[window]` overrides for `scale_factor` and `decorations`; personal Linux preferences belong in the user's configuration rather than platform defaults.


## 2026-09-06: daily-use input, file navigation, and native installation

Ctrl+B stays armed across modifier-only keydowns. This fixes Shift cancelling the prefix before a shifted command; there is no prefix timeout. Desktop additions include numbered tabs, pane cycling/directional focus, and local file navigation. Pane destinations derive from the latest Herdr layout; selection still requires Herdr confirmation.

Focus requests are serialized per session and coalesce to the latest queued click. A delayed older request must finish before a newer request for the same session is sent. Different sessions have independent in-flight requests. Reset invalidates queued intent while preserving ordering for an outstanding request to that session.

Files and Review own their document/tree focus and fuzzy picker. Files indexes bounded directory listings within its authorized root and cancels on dismissal or root change; Review searches the current comparison. New renderable files open in preview with one source toggle. Existing unsent line comments remain visible in Markdown preview. Repository-discovery warnings have one location in Files and name the numeric configurable budget.

The native installer builds a standalone Tauri binary and writes a stable user desktop launcher. Updates atomically replace the executable without stopping running processes. Installation ownership is recorded for scoped update/uninstall. This workflow is verified in a disposable prefix; the user's running installation and Herdr default session were not changed.

Verification and limits are recorded in `planning/daily-use-2026-09-06.md`.

## 2026-09-08: bounded session refresh and terminal activity

Session refresh reads worktree provenance from `session.snapshot`, without issuing `worktree.list` for every Space. Missing or malformed optional Git decoration does not invalidate pane structure or extension inspection. Explicit worktree inventory and mutations keep their existing validation and endpoint pinning. Stock Herdr 0.8.2 snapshots have no branch field; Cockpit retains repository grouping and uses the Space label when branch data is absent. A branch is displayed only when actually supplied.

Automatic session recovery makes at most three attempts per outage, after 250, 500, and 1000 milliseconds. One second of live state in the same ordered stream generation renews the budget. Manual Resync and session changes reset it; obsolete timers are cancelled. Last-known GUI content and binding-local renderer choices survive resync. Only an actual binding or session change invalidates those choices.

Terminal fitting remains immediate before attachment. Subsequent geometry changes use a 100-millisecond trailing debounce, cancelled before renderer disposal. Cockpit no longer sends structured idle pointer motion; controlled press, drag, release, cancellation, wheel, and keyboard paths remain. This does not add application-hover support.

Issue #1 was verified rather than applied verbatim. A three-Space refresh fell from one snapshot plus three inventory requests to one snapshot only. A browser resize burst fell from fourteen resize commands to one, with no idle mouse reports. Real PTY captures verified pointer, wheel, and keyboard input. Ten browser and four Linux-native Files GUI/terminal cycles retained the selected document; an outage retained it through manual recovery. Native-to-Herdr-TUI-to-native handoff delivered input in each client. These checks used only disposable session `ci1-0908`. macOS runtime behavior was not retested on this Linux host.

## 2026-09-08: Herdr 0.9.0 compatibility and mouse autodetection

The adapter now targets [Herdr 0.9.0](https://github.com/herdrdev/herdr/releases/tag/v0.9.0), protocol 22/schema 1. Direct-terminal message tags and Hello/Resize payloads follow the [tagged wire contract](https://github.com/herdrdev/herdr/blob/v0.9.0/src/protocol/wire.rs). This retains per-pane ANSI rendering, not the earlier custom client-shell or graphics path.

Herdr's dynamic `MouseCapture` signal controls application pointer interception. Cockpit forwards structured cell-coordinate `AttachMouse` messages; Herdr selects SGR, legacy, or other application encoding from the live terminal mode. Wheel input retains `AttachScroll` routing. Mode-off and Shift-drag permit local text selection. Mouse state is attachment-local, defaults off, and is cleared on failure, mode exit, or ownership loss. A control conflict or takeover switches to observation; only a new local action requests takeover. Failed control requests discard queued input.

Lifecycle subscriptions are live-only in this release. Cockpit uses the initial snapshot to discover pane-specific subscriptions, then reads an authoritative snapshot after the subscription acknowledgement. A changed pane set forces a rebind before readiness. Browser/native relays read their published baseline after readiness, preserving live agent-status subscriptions without relying on retained history.
Every established-stream rebind also invalidates the published snapshot after acknowledgement, including when only agent status or labels changed during the gap. Bootstrap rebinds do not enqueue notifications before a consumer exists. A socket regression verifies recovery without a subsequent live event.

Project capability inspection now includes workspace close and worktree removal. Normal close still omits `close_group`, allowing Herdr to reject accidental group closure. Existing repository-action consent does not silently grant the new Git `trust_repository` override; that optional upstream flag remains unset.

Verification used only disposable session `cockpit-h090-0908`. Browser and Linux Tauri rendered real protocol-22 frames and delivered keyboard, mouse press/drag/release, and wheel input to a capturing PTY. Enabling/disabling application tracking switched pointer handling without reconnecting; disabled-mode and Shift selections emitted no pointer input. Native/browser takeover preserved observer output and allowed explicit control recovery. Browser rendering was checked at 1440×900 and 1024×640. Exact pixel targeting, idle hover, graphics, and macOS runtime behavior were not tested.

The same browser run created and removed a real linked worktree and launched Files against its checkout, displaying `sample.txt` from the correct plugin context. Final gates passed 151 frontend tests and 113 Rust adapter/protocol/host tests, plus browser and native builds. The named session, clients, display server, and disposable fixture resources were removed; captured input and browser/native evidence remain under `/tmp/cockpit-h090-proof`. The user's active Herdr session and installed Cockpit executable were not replaced.

## 2026-09-08: Space-associated browser direction

Use dedicated external Chromium associated with a Space, with machine-installed Playwright CLI and its existing agent skills for page automation. Cockpit owns the association, scoped lifecycle, annotation collection, and delivery through the existing diff/file annotation-to-pane workflow. It does not proxy every browser action or require a custom agent browser API.

Discovery must support already-running agents and agent-requested browser creation. Browser environment variables are optional; a proposed minimal open/status/close interface resolves the caller's current Herdr Space without relying on GUI focus. Normal Cockpit shutdown should close only its owned browser sessions. Exact multi-client ownership, CLI session lookup, extension launch/pairing, and artifact delivery remain implementation gates.

The starting point is [Space-associated browsers and visual feedback](planning/browser-space-integration-2026-09-08.md). It distinguishes the agreed direction, proposed interfaces, and acceptance work. Spike commit `b317ea4` demonstrates browser/annotation exchange but does not implement this production design. Its custom automation CLI and enforced control gate are not the target integration; direct Playwright CLI use provides cooperative rather than enforced input coordination.

## 2026-09-08: native window configuration and macOS plugin launches

Native window presentation uses the shared Cockpit TOML discovery path. Every platform defaults to scale 1.0 with decorations enabled; `[window]` overrides apply before the window is shown. Scale must be finite and within 0.2 through 10.0. The source icon is 8-bit RGBA because the macOS bundler rejects its former 16-bit RGBA encoding.

macOS process-generation evidence uses Darwin's microsecond start timestamp through the target-specific `libproc` safe wrapper, not parsed `ps` or `sysctl` output. Executable matching prefers the kernel executable path; a metadata fallback examines only executable-name slots, never later command arguments. After an authorized plugin launch and pane/process verification, macOS may retain the approved launch root when injected plugin context is unavailable. This does not permit adoption of pre-existing macOS file viewers without a launch receipt. Linux retains its strict process-generation and injected-context checks.

An uncertain extension mutation triggers an authoritative resync without automatically repeating the launch. A browser smoke against disposable session `cockpit-patch-0908` opened a real Files pane, replaced its successful confirmation with `mutation_applied_snapshot_failed`, and verified the graphical file content, retained inline error, and exactly one launch. Linux-native smoke compared default scale 1.0 with the personal scale 1.25 override and verified the undecorated window hint. Tauri's icon generator produced an ICNS from the corrected source.

macOS runtime behavior was not exercised on this Linux host. The attempted Darwin cross-check is blocked by `libproc` generating its bindings only on a macOS build host; it fails on Linux with a missing `osx_libproc_bindings.rs`. Native macOS compilation and runtime verification remain unverified, not covered by the Linux smoke.

## 2026-09-13: Directory opening and compact task setup

The [approved UI polish](planning/product-atlas-2026-09-12/polish/implementation-plan.md) replaces branch-based Open with a path-only operation. Herdr `workspace.create` opens the validated directory exactly as supplied, including plain directories and directories nested within Git checkouts. Git discovery adds metadata; it does not initialize Git, switch branches, or confer deletion ownership. New worktrees continue through Herdr's worktree API.

Ownership is explicit in setup plans. Opened directories remain borrowed. Teardown offers worktree removal only for an owned Create with an exact creation receipt and fresh linked-worktree, endpoint, and clean-status checks. Legacy records remain readable; unproven legacy Creates require review and cannot resume provisioning. Missing ownership metadata never grants deletion authority.

Configured repository actions run without per-operation consent. The optional upstream Git `trust_repository` flag remains unset. The compact form obtains provider metadata through the shared Rust service, matches provider identity to configured local repositories, preserves explicit edits, and ignores superseded lookups. Ambiguous matching requires selection. Review defaults use the provider's actual source branch. Open paths and new-worktree destinations have separate form state.

Every submission gets a fresh plan. Once dispatched, its receipt survives a closed dialog or a lost response; Cockpit reads and reconciles that operation rather than submitting a second setup. Browser and native adapters share the same contract. See the [execution record](planning/product-atlas-2026-09-12/polish/execution.md) for verification and platform limits.

### 2026-09-13: optional comment message preview

The user's targeted polish review removes the manual preview prerequisite for pasting comments to an agent. Preparation supplies the current payload hash and eligible target; sending still revalidates the source and payload, preserves stale-source and uncertain-delivery handling, and pastes without submitting. Preview remains an optional disclosure. Draft saves commit the captured comment without rereading the entire batch; freshness is checked on batch inspection and delivery preparation.

The 2026-09-23 FLOW-01 real Review mutation found that prose-only edits could return and persist a previously captured draft as `current` after its staged source changed, placing an old line comment on the new line until refresh. This supersedes only the no-reread part of the preceding draft-save decision: keep captured file identity/anchor immutable and generation checks strict, but re-evaluate bounded batch source states before a successful upsert commits and returns them. Review deduplicates source references with at most four concurrent reads across a 64-draft batch. This is source truthfulness for Review/Context comments, not browser-page annotation retention: page-scoped browser annotations follow the separate user navigation policy recorded in OBS-030.

## 2026-09-13: Inline Space browser replacement

The user authorized the complete inline browser plan and the no-migration cutover. This supersedes the September 8 external-window and annotation-extension choices. The existing single browser owner, named Playwright CLI session, persistent profile, and agent discovery remain authoritative. The view uses the selected raw JPEG screencast transport. Focused implementation and runtime verification passed on 2026-09-13; the broad acceptance matrix is not claimed.

The browser is a Space-scoped Cockpit split beside the unchanged Herdr layout. It does not add a Herdr pane, tab, or PTY. Clicking it suspends local terminal writable intent without fabricating Herdr focus. Returning to a terminal uses the existing confirmed focus path. Hide affects presentation; Close remains an explicit browser lifecycle action.

Cockpit owns annotation overlays and durable revisioned drafts. PNG capture retains exact retry bytes and existing feedback/paste receipt semantics. The user explicitly excluded legacy migration. The inline runtime directly replaces the extension and external-window integration without an import layer. Existing saved feedback remains readable. The focused pass covered browser open/frame, region draft persistence, hide/show, URL navigation, static gates, and Linux Tauri startup; full security, performance, native input/decode, and A01–A25 evidence remains unrun.

## 2026-09-20: compatible Herdr patch releases

Removed the exact 0.9.0 display-version gate while retaining protocol 22, schema 1 and all 24 required methods. Incompatible/unavailable status does not advertise terminal mouse support; compatible streams still require their own MouseCapture application demand. Installation/session cache generations remain unchanged, and rejected status is freshly inspected on retry.

Real Herdr 0.9.1 reached the session/Space/tab/live terminal UI in browser and Linux Tauri. Protocol/schema/method faults, malformed JSON and executable failure produced identical browser/native status payloads and visible notices without session work. Retry after fixture repair restored the live session. Evidence: `planning/stability-and-gitlab-2026-09-20/runs/run-20260920-a3e9b950/TERM-01.md`. This does not claim arbitrary protocol support, macOS proof or the remaining campaign.

## 2026-09-24: Adaptive live browser image quality

The user replaces unconditional live-frame sharpness with: **attempt to have a sharp image; allow lower resolution to keep up performance**. Real pages may animate continuously or react to hover and scrolling without becoming quiet. Preserve current eligible page updates and responsive input rather than freezing an older sharp image or waiting for a DPR-perfect first frame.

Attempt higher-quality capture when safe and affordable, including after activity eases. A lower-density frame alone does not establish stale content, incorrect geometry, or an unavailable prerequisite. This does not authorize permanently ignoring affordable quality improvements, continuous idle screenshot polling, enlarged queues, stale-frame promotion, or weakening target/document/viewport/lease checks. CSS viewport, page DPR, encoded raster dimensions, and displayed size remain distinct. Immutable saved PNGs must still faithfully represent the pinned frame and annotations.

Chromium is the capture producer; Linux-native WebKit is a display/input consumer. Measure their behavior separately. [Live browser image quality](planning/stability-and-gitlab-2026-09-20/ACCEPTANCE.md#live-browser-image-quality) defines the current quiet, animation, hover, scroll, and recovery scenarios. Historical density-only failures remain evidence, not retroactive passes or standing blockers under the revised requirement. This decision changes acceptance; it does not claim an adaptive capture implementation or completed browser/native acceptance.

## 2026-09-24: One-click Space setup and Jira work items

The user found the compact setup form unusable (choosing a repository left its list open and showed no selection) and chose: no separate confirm step, a file-picker style repository field, and a link-first flow for the GitLab MRs and Jira tickets they set up several times a day. This supersedes the "Review setup" then "Start setup" steps of the 2026-09-13 compact form. Path-only Open, ownership, receipts and uncertain-outcome rules are unchanged.

The form prepares the authoritative plan while the user types and shows it as a one-line summary. **Create Space** (or Enter) starts the plan for the current input; if that plan is still being prepared it waits for it, and it never starts a plan prepared for different input. A plan that the backend reports as stale is shown again and needs another press. The dialog closes when the Space is ready; failures stay in the dialog with their recovery action. Plans expire after an hour and never-started plans are deleted instead of accumulating, and startup no longer turns an unstarted plan into a resumable partial setup.

Jira work items are read through the owner's `jira` CLI (ankitpokhrel/jira-cli) with `executable = "jira"`. They name no repository, so their source authority is the configured Jira site and the user chooses the repository; a Jira link never selects or clones one. An MR or issue that mentions a Jira URL on a configured site, or a key such as `SCRUM-5` in its title, branch or description that the site confirms, offers that work item as an additional import (on by default, bounded to four). Linked items are validated before setup starts and imported into the same companion; a failed import leaves the Space and offers the existing source retry.

Setup reads the same artifact several times within seconds: the link lookup, the plan, the check before start and the import. Within one setup these reads may reuse a provider result from the last two minutes. An explicit Context import or refresh always asks the provider. When the provider's metadata names the canonical URL (GitLab, Jira), the plan uses metadata and starts the full read in the background. The check before start waits for that read, and does a fresh one if it failed, so a full read that fails still stops setup before anything is created. A plan older than two minutes is checked against the provider again.

With no Space checkout to go on, the dialog preselects the repository containing the focused pane's current folder (the innermost repository for nested checkouts). It re-reads the folders when it opens, because changing folders sends no Herdr event. This follows the current Space only while the repository is still the dialog's own guess; a repository the user chose, or one taken from a link, is kept.
