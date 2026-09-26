# Cockpit decisions in force

This file records current product and engineering rules, not the implementation diary. Herdr and the repository are sources of truth for their respective state; UI details may evolve without changing those ownership boundaries.

## Herdr authority & compatibility

- Herdr owns live sessions, Spaces/workspaces, tabs, panes, PTYs, processes, focus, layout, and agent state. Cockpit presents and operates on that state; it does not keep a competing live registry. Cockpit-owned companion associations are evidence about Cockpit's files, not another workspace authority.
- Use the supported Herdr socket/schema surface for persistent session state and terminal streams. Validate protocol, schema, and every required method before advertising compatibility; fail closed on mismatch. Display-version text is diagnostic and is not an exact patch-version allowlist. The adapter currently requires protocol 22, schema 1, and its required-method set.
- Build client state from an authoritative snapshot and ordered events. On gaps, identity changes, reconnect, or stale state, resnapshot and rebind rather than guessing. A mutation acknowledgement is not itself fresh focus or control authority; that comes from the ordered Herdr state.
- Preserve Herdr semantics and hierarchy while presenting Cockpit's graphical workbench. The Space tree, panes and layout remain Herdr-backed; semantic focus, DOM keyboard focus, and writable terminal ownership are separate state.
- The Agents list is ordered blocked, done, working, idle, unknown; newest state change first. Open question: whether Cockpit should instead follow the agent sort configured in Herdr.

## Terminal attachment & input

- Herdr owns the PTY and terminal model; xterm.js renders and captures input. Visible panes attach to Herdr's terminal stream, while hidden panes do not need a mounted renderer. Stable terminal frames and their sequence are authoritative; do not reset xterm on a full repaint.
- Keep ordinary terminal bytes on the attachment input path. Route wheel/page scrolling through the attachment's scroll operation and application mouse through the attachment's structured, cell-coordinate mouse operation when Herdr signals capture demand. Do not substitute raw SGR injection for structured mouse. Control intent, confirmed Herdr focus, attachment state, and process lifetime remain distinct.
- Attach, observe, resync, and retry failures must leave the Herdr resource intact and visible with its status and recovery action. A request with an uncertain mutating outcome is not safe to repeat automatically; recover by inspecting authoritative state.
- When investigating mouse or terminal input, name the exact input path tested: **physical attach, xterm wheel, structured AttachMouse, or CLI-injected SGR**. State only the acceptance dimension observed for that path; evidence for one path does not establish another.

## Workspace & filesystem ownership

- Setup opens a chosen directory by path or creates a worktree through Herdr's worktree API. Opening a checkout does not make it Cockpit-owned; an opened directory remains borrowed. Cockpit does not clone repositories or use display labels as filesystem authority.
- Cockpit owns its companion context and operation records. A companion manifest records association/provenance and must be checked against fresh Herdr evidence; it is not a live workspace registry. Publish companions without replacing an existing destination.
- Destructive teardown requires explicit ownership plus the exact creation receipt and fresh checks of linked-worktree, endpoint, and clean status. Missing or ambiguous evidence never grants deletion authority. Preserve operation receipts across closed dialogs and uncertain responses; reconcile the recorded operation instead of dispatching setup twice.
- Keep filesystem reads bounded, rooted, identity-checked, and no-follow where applicable. Revalidate roots/source identity before mutations or returning comments: a path, label, or prior snapshot alone is not authorization.
- Setup is plan-driven and path-only for Open. The plan is tied to its input; stale plans require a fresh user start. Provider links may identify/import artifacts but do not select or clone a repository on the user's behalf.

## Context & Review

- Herdr owns extension panes, process, tab/Space membership, focus, and layout. Cockpit may replace a verified extension renderer with its Context or Review UI, but does not scrape extension-private state or create synthetic Herdr panes/tabs. Detection uses supported launch/process evidence and verified roots, not titles alone; ambiguity keeps the terminal fallback.
- Context and Files use Cockpit's bounded filesystem model. Companion-specific operations require a verified companion; ordinary file browsing and Review can be rooted in a freshly verified local checkout without granting companion write authority. Unsafe, stale, oversized, or unsupported sources fail closed.
- Review is Cockpit's read-only local Git view plus shared comment capture. Comments retain immutable source identity/line anchors, and source state is re-evaluated before a successful save or delivery. Stale or uncertain deliveries retain recoverable drafts/receipts and never submit Enter to an agent.
- Pasting a prepared comment targets an explicitly selected eligible agent in the same actual tab. Preparation and send validate current payload/source; preview is optional, and sending does not submit the terminal command. Keep pending, rejected, and unknown outcomes distinct to avoid accidental duplicates.

## Inline browser

- The browser is a Cockpit-owned inline view scoped to the selected Space, not a Herdr pane or separate external-browser integration. Hiding it changes presentation; closing it is an explicit lifecycle action. Browser control remains scoped to its current target and valid frame/document identity.
- Unsent notes and annotations belong to the page/document they were created on. Navigation or reopening retires that page's drafts; drafts for other open tabs and drafts referenced by a prepared capture remain. Saved delivery/feedback receipts have a separate lifecycle.
- Preserve target, document, viewport, and ownership checks for input and captures. A capture must refer to the pinned frame and immutable submitted image; do not turn incomplete or lower-density live imagery into saved-frame evidence.
- **Accepted requirement, implementation/acceptance not verified:** attempt sharp browser images when safe and affordable, but allow lower resolution to preserve responsiveness during animation, hover, or scrolling; pursue affordable quality again as activity eases. Do not freeze older sharp imagery, poll continuously, or weaken frame/target/lease checks. Measure Chromium capture separately from Linux WebKit display/input. See [Live browser image quality](planning/stability-and-gitlab-2026-09-20/ACCEPTANCE.md#live-browser-image-quality).

## Providers & setup

- Provider adapters expose supported capabilities and explicit unsupported-operation errors; Cockpit reads/imports and validates provider data, while remote writes remain with provider CLIs or the user's usual tools. Provider metadata may inform a plan but does not override local repository choice or Herdr ownership.
- Current setup supports configured local repositories, path-only Open, Herdr worktree creation, and configured artifact lookup/import including Jira work items. Jira lookup uses the configured `jira` CLI/site; a Jira URL or ticket key never chooses or clones a repository. Linked imports are bounded and validated before setup.
- Prefer explicit, bounded hydration and canonical snapshots with source identity/revision where available. Freshness checks use provider metadata when available and canonical content otherwise. A failed optional import should not erase successful assets or silently become a successful import.
- Configuration comes from supported file, environment, and invocation options. Do not persist or export secrets through context snapshots or generated environment values; external tools own credential handling.

## Deferred scope

- Automatic OMP setup/launch; Cockpit-managed credentials/secrets; remote browser access and multi-user authorization.
- Agent history and resumable conversations; a separate inbox popup; a Cockpit-owned settings screen.
- Provider-specific mutations and a complete cross-provider Forge/Issue Tracker/Wiki feature matrix. Existing Jira lookup/import and supported provider read slices are not a claim of a complete provider matrix.
- Unbounded previews, arbitrary shell execution, and raw socket forwarding remain outside the product contract.
