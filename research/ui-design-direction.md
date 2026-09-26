# Cockpit UI design direction

This document describes design intent: hierarchy, scan path, and semantic-token approach. It is not a pixel specification or a claim that every interaction below exists. Current visual values live in `src/app/styles.css` and `src/app/TerminalPane.tsx`; implementation and Herdr capability determine available behavior.

The protocol-22 client-shell behavior in `../DECISIONS.md` overrides historical ownership and renderer details below. Context/Review direction is in `../planning/next-level/08-ui-design.md`; current behavior may differ from those proposals.

## Decision

- Build Cockpit as a **graphite operations workbench**: a compact control rail beside a terminal-oriented work area. Prefer flat dark surfaces, crisp separators, restrained state color, and no decorative dashboard cards.

The first screen should make a Herdr session easy to supervise. The main region may also contain Cockpit browser, Context, or Review surfaces.

1. Which session and Space am I operating in?
2. Which agent needs attention next?
3. Which real terminal owns my keyboard now?

The intended hierarchy is:

```text
Cockpit window
  selected Herdr session
    Spaces tree
      selected Space
        tabs
          panes
            real terminal or Context viewer
    Agents attention queue
```

Herdr calls its API resources workspaces; the UI calls them Spaces, as required by [CONTEXT.md](../CONTEXT.md) and [DECISIONS.md](../DECISIONS.md). Do not invent project, room, task, or conversation as synonyms.

## Relationship to the Herdr TUI

Herdr TUI parity is the starting constraint, not Cockpit’s product destination.

- Copy semantics before appearance: resource hierarchy, authoritative focus, attention priority, terminal ownership, shortcuts, and mutation behavior should remain recognizable and correct.
- Treat Herdr's TUI and server behavior as references. Agents are ordered blocked, done, working, idle, unknown; newest state change first (whether Cockpit should follow Herdr's configured agent sort is an open question in DECISIONS.md).
- Depart deliberately where a desktop surface can provide better supervision: persistent overview, direct manipulation, visible ownership, larger readable type, pointer targeting, and resource-local recovery.
- Never trade correctness for graphical polish. A Cockpit interaction still resolves through Herdr and reconciles from authoritative state.
- Do not preserve TUI density when it harms legibility. Typography and geometry are semantic design tokens; tune those tokens as a system instead of scattering component-local overrides.
- Judge new behavior by the resulting work loop: identify attention, navigate to the right resource, understand keyboard ownership, act, and recover without losing context.

Future changes should state whether they preserve a Herdr behavior, expose it more clearly, or intentionally replace its interaction. Intentional replacements belong in `DECISIONS.md`.

For each intentional evolution, record:

1. the current Herdr/Cockpit behavior and concrete friction;
2. the desired user-visible behavior;
3. which Herdr authority and compatibility constraints remain fixed;
4. the observable acceptance scenario in a disposable real session.

## Reference study and what Cockpit takes from it

This is a synthesis, not a skin of another product.

| Reference | Verified pattern | Cockpit decision |
| --- | --- | --- |
| [Herdr README and official screenshot](https://github.com/SuperCodeAgents/herdr-terminal#readme) | Herdr puts Spaces and agent state in a narrow left column and uses thin pane borders around unmodified terminal programs. Its terms are workspaces, tabs, panes, blocked, working, done, and idle. | Preserve that scan path and those terms. Translate terminal-cell framing into flat desktop separators rather than recreating box-drawing characters. |
| [tmux getting started](https://github.com/tmux/tmux/wiki/Getting-Started#sessions-windows-and-panes) | tmux has a strict session, window, pane hierarchy. Exactly one pane is active and its border marks that fact. Processes persist behind attach and detach. | Make session, tab, and focused pane three visibly different levels. Mark the focused pane at its boundary, not with a large tinted background. Never imply that closing or hiding a renderer stops its process. |
| [Zellij basic functionality](https://zellij.dev/tutorials/basic-functionality/) | Zellij keeps session and tab identity in a compact top bar, frames tiled panes, and exposes immediate mode actions in a status bar. | Keep tab identity directly above panes and reveal pane actions in local chrome. Do not add a permanent global shortcut bar because it would spend vertical space and compete with terminal input. |
| [Warp split panes](https://docs.warp.dev/terminal/windows/split-panes) | A pane has local chrome, visible focus, mouse targeting, and resize/movement affordances. | Keep pane identity and focus clear. Drag targets and explicit drop zones are desired interactions, not yet implemented. |
| [VS Code interface](https://code.visualstudio.com/docs/editing/getting-started/userinterface) | VS Code separates the primary sidebar, tabbed editor groups, and movable split regions with compact borders. It supports tree context menus and drag/drop. | Use a resizable primary sidebar and tab strip, but omit the Activity Bar, secondary sidebar, global status bar, and bottom panel. Cockpit has one primary mode and does not need those extra layers. |
| [Zed Project Panel](https://zed.dev/docs/project-panel) | Zed's tree supports keyboard traversal, sticky ancestors, inline rename, context menus, filtering, and clear drag/drop behavior in a compact panel. | Give Spaces a true tree interaction model, sticky section header, inline rename, and precise insertion indicators. Do not turn each Space into a card. |
| [Zed getting started](https://zed.dev/docs/getting-started#panel-layout) | Zed's Agentic layout places agent controls next to the project tree rather than hiding them in a modal. | Keep Agents persistent below Spaces. Cockpit's queue is a glanceable operational list, not a chat panel or popup. |
| [Docker Desktop dashboard](https://docs.docker.com/desktop/use-desktop/) | Docker Desktop combines durable resource navigation, quick search, and persistent integrated terminals. | Borrow durable navigation and resource-local status. Reject its large dashboard rows, illustrations, and top-level product sections because Cockpit should open directly on work. |
| [lazygit primary screenshots](https://github.com/jesseduffield/lazygit#readme) | lazygit fits several operational lists and a detail area into one view through terse labels, strong selection, and almost no ornament. | Favor row density and immediate actions. Retain full words and accessible labels where a TUI can rely on learned single-letter commands. |

## First-screen composition

Design for a full-window desktop workbench with a flexible main area and a resizable sidebar. On narrow viewports, preserve usable terminal space through responsive navigation rather than forcing whole-page horizontal scrolling.

### Frame

- Fill the window. No centered container, page gutter, hero, title masthead, or footer.
- Use a two-column grid with a resizable sidebar and fluid main area.
- Keep the divider visually quiet with a usable resize target.
- Use square corners inside the workbench. A 4 px radius is reserved for menus, tooltips, buttons, and inline notices, never for primary regions.

### Sidebar proportions

The sidebar has three vertical regions:

1. Session selector.
2. Flexible Spaces region.
3. Agents attention queue.

Keep Spaces and Agents visually distinct. A draggable splitter between them is a desired interaction, not yet implemented.

#### Session selector

The selector is the topmost control because changing it replaces all state below it.

- Use a clear, compact session selector with selected session name and connection state.
- Opening it should provide a searchable session chooser. Switching must not present a half-old, half-new tree.

#### Spaces tree

- Use compact section headings and tree rows, with disclosure controls, labels, and any relevant status or count.
- Ancestor paths may use `text-secondary`; never dim the selected item's parents below readable contrast.
- Keep the `SPACES` header sticky while the tree scrolls.
- Inline rename replaces only the label region. Enter commits; Escape cancels. A mutation error stays directly below the row in a compact two-line notice.
- Right-click opens actions supported by the active Herdr schema. Unsupported actions are absent, not disabled promises.

Drag behavior must be unambiguous:

The desired drag behavior distinguishes reorder from reparent, previews the destination, and reconciles only after Herdr confirms. Generic drag reparenting, drop zones, auto-expansion/scrolling, and pending-operation indicators are not yet implemented.

#### Agents queue

Agents is an attention queue, not a directory and not a chat transcript.

- Show agent status, identity, and useful Space/tab context in a glanceable row.
- Order the queue blocked, done, working, idle, unknown; within a status, newest state change first.
- State is never a color-only dot. Use icon plus word: `! Blocked`, `↻ Working`, `✓ Done`, `· Idle`. Use a spinner only for a pending operation, not as the permanent Working icon.
- Clicking a row requests focus of its owning pane and attachment. The highlight moves only when Herdr acknowledges focus.
- Blocked and Done may receive stronger state emphasis; Working and Idle remain identifiable without relying on color.

### Main work area

#### Tab strip

- Place tabs directly above the pane layout. Tab sizing and strip dimensions follow current style tokens, not fixed values in this direction document.
- Tabs size to content within usable bounds. Do not reserve excessive space for short labels.
- Each tab shows a 14 px type icon, label, and close action on hover/focus. A small state mark may appear after the label if a child pane needs attention.
- The active tab has `surface-raised`, primary text, and a 2 px bottom accent. Inactive tabs use no pill or border radius.
- Context and Review are pane renderers, not guaranteed first-class tabs.
- Keep overflow usable and expose only Herdr-supported create/layout actions.

#### Pane layout and chrome

Render the exact Herdr layout. Do not rebalance it for aesthetic symmetry.

- Adjacent panes share clear separators; resize hit targets should be usable.
- Pane chrome should identify the pane and provide a local action surface. Header drag targets and pane drop zones are desired interactions, not yet implemented.
- Header order: terminal/process icon, pane label or short ID, process/agent state, flexible gap, attachment status, local overflow menu.
- The focused pane has a 2 px `focus-strong` outline drawn inside its bounds. Its header text becomes primary. Unfocused pane headers use neutral borders, never reduced terminal opacity.
- If a pane becomes too narrow for its header metadata, keep label and status icon; move the rest into the overflow menu.
- Double-clicking the header toggles pane maximize through Herdr if supported. Escape or the visible restore action exits maximize.
- During pane drag, directional drop zones and a tab-strip target are desired, not yet implemented.
- During resize, update the visual divider continuously, throttle Herdr resize requests, and reconcile to the confirmed layout when the drag ends.

#### Terminal treatment

The terminal is content, not a decorative widget.

- xterm.js fills the pane content area. Use the current terminal spacing and font settings in `src/app/TerminalPane.tsx` and `src/app/styles.css`; do not treat earlier numerical values as current.
- Preserve ANSI colors, cursor shape, alternate screen, selection, links, and terminal program mouse handling. Cockpit chrome must not reinterpret prompts or divide output into blocks.
- Identify focus through pane chrome and the real xterm cursor, not a second fake caret.
- Scrollback comes from Herdr's terminal semantics, not a browser-owned parallel history. An overlay scrollbar is not implemented.
- Do not animate terminal output, fade old lines, blur unfocused panes, or overlay agent prose on output.

#### Context tab

Context is a read-only work surface, not a document editor.

- The Context tree, local toolbar, and viewer should form a compact companion-file surface. Exact dimensions and typography follow the current implementation.
- Frontmatter starts collapsed behind `Metadata`, with canonical identifier and freshness summary visible in the header.
- Search opens a 32 px field in the toolbar and sends the narrow core-mediated search operation. Results appear as a compact list with file, line, and one bounded excerpt. Do not imply arbitrary shell access.
- In Context resources, show source titles first with provider, kind, freshness, and materialization status as compact chips. Keep full canonical IDs, revisions, URLs, hashes, and paths in a keyboard-accessible details disclosure; preserve inline failure and refresh actions.

## Token-level visual direction

### Typography

Prefer semantic font tokens and tune typography as a system. Current font families and sizes are defined in `src/app/styles.css` and `src/app/TerminalPane.tsx`; the values below are not a current token contract.

Use uppercase sparingly for sidebar section labels. Do not uppercase tabs, sessions, states, or errors.

### Spacing and geometry

Use a compact, consistent spacing and geometry vocabulary. Prefer shared semantic tokens over component-local values. Current token names and values live in `src/app/styles.css`; do not treat the earlier measurement table as a current specification.

### Dark palette

Dark is the first direction because real terminal programs and Herdr's primary screenshot are dark-first. Use semantic tokens rather than component-local literals. Current palette values live in `src/app/styles.css`, with terminal colors also configured in `src/app/TerminalPane.tsx`; older exact palette values are not current values. Do not remap application-supplied true color.

### Icons

Prefer a consistent outline-icon approach and accessible names for icon-only controls. Current icon choices, dimensions, tooltips, and hit areas follow the implementation.

## Interaction states

### Focus and keyboard ownership

Focus needs two visible levels:

- `focus-visible` on ordinary controls uses a 2 px `focus-strong` inset ring with 1 px offset from the component edge.
- Terminal focus uses the focused pane outline plus the real xterm cursor.

The active Space, tab, and pane remain visibly selected. Herdr's magic escape key should take priority over GUI shortcuts; this behavior is not yet implemented. Ordinary keys go to focused xterm. Cockpit must not intercept common terminal chords merely because a matching GUI action exists.

### Hover and pressed

- Rows use `surface-hover` after pointer entry, with no transition longer than 80 ms.
- Icon buttons use the same hover fill in a 28 px square.
- Pressed controls shift to `surface-selected`; do not scale, bounce, or glow.
- Pane dividers change from `border` to `border-strong` on hover and `accent` while dragging.

### Loading

Use loading only where work is pending:

- Initial connection: keep the entire frame and section headers stable. Show six neutral tree-row skeletons and two pane rectangles. Put `Connecting to <session>...` in the first pane header.
- Snapshot resync: preserve last-known content and add a slim inline `Resyncing...` strip. Do not replace usable terminals with skeletons.
- Terminal attach: keep pane chrome and show `Attaching to pane <id>...` centered over `terminal-bg`, with Cancel only if the protocol supports cancellation.
- A spinner is 12 px and rotates at 900 ms per turn. Respect reduced motion by showing a static progress glyph.

### Empty

Empty states stay local and compact:

- No sessions: full main area message `No Herdr sessions found`, one primary action `Retry`, and a monospace hint showing the documented Herdr session command. Do not offer Cockpit-owned session creation unless the supported schema provides it.
- Session has no Spaces: sidebar says `No Spaces in this session`; main says `Create a Space in Herdr to begin` with a schema-supported create action if available.
- Space has no tabs: main says `No tabs in <Space>` and exposes only supported create actions.
- Agents empty state should be local and compact; avoid celebratory inbox-zero messaging.
- Context empty: `No context files found for this Space`, then the companion path if safe to display and Refresh.

No illustrations, confetti, mascots, or oversized headings.

### Stale and disconnected

`Stale` means last-known data is visible but not current. `Disconnected` means the event/terminal connection is unavailable. Keep the distinction.

- Stale resource: amber outlined `STALE` badge in its header or row, last confirmed time, and `Resync` action. Content stays visible.
- Global disconnect and terminal notices should stay near the affected session or pane. Exact placement and size follow the current implementation.
- Never close a pane, clear output, or change agent state to Idle because transport failed.

### Inline errors

Errors attach to the resource or action that failed.

- Row mutation errors should appear near the affected row, with a concise explanation and available recovery action.
- Pane attach/input error: bottom overlay inside that pane. Keep the pane and last terminal frame visible.
- Context file error: viewer notice with file identity, refusal/failure reason, and allowed next action such as Refresh or Open externally.
- Session/schema error: persistent main-area notice because it affects the whole selected session. List the installed and required schema versions when known.
- A toast may echo an error only when the affected resource is offscreen. The inline copy remains the source of explanation and recovery.

Error copy says what failed and what remains safe. Example: `Rename was not applied. Herdr kept the Space as api-review.` Avoid `Something went wrong`.

## Herdr behavior constraints that the visual design must respect

These constraints come from the confirmed architecture in [CONTEXT.md](../CONTEXT.md#53-state-and-interaction), [DECISIONS.md](../DECISIONS.md#terminal-attachment), and Herdr's [direct attach documentation](https://github.com/SuperCodeAgents/herdr-terminal#direct-agent-attach).

1. Herdr owns sessions, Spaces, tabs, pane layout, PTYs, processes, focus, agent state, and terminal scrollback. Cockpit renders and requests; it does not manufacture a parallel truth.
2. Selecting a Space, tab, pane, or agent sends a Herdr focus operation. Selection chrome follows the acknowledged response or event. A pending click may show progress but not confirmed selection.
3. xterm.js renders Herdr's server-owned terminal. It does not start a replacement PTY. Attach receives current rendered state and then live ANSI frames where supported.
4. Only visible panes in the selected tab keep renderers/subscriptions. A hidden tab detaches its renderer while its Herdr process continues. UI copy must never equate hidden, detached, or disconnected with stopped.
5. One writable client owns terminal input and resize. The initially focused pane and explicit local selection/click may request takeover. After external focus or ownership loss, Cockpit observes without reclaiming until another local action. Never accept typing into a pane that has not confirmed ownership.
6. Herdr's magic escape key should win over GUI shortcuts. This priority behavior is not yet implemented.
7. A sequence gap, reconnect, or stale cache triggers resnapshot and resubscription. Preserve last-known content with a stale marker until replacement, rather than animating local guesses into place.
8. Attach failure leaves the resource visible with retry and resync. It must not silently close or clear the Herdr process.

## Compact text wireframe

```text
│ Session                [⌄]│ Tabs and selected pane layout                                      │
├──────────────────────────────┼──────────────────────────────────────────────────────────────────────┤
│ SPACES                  3 [+]│┌ [term] api-1   ↻ Working                    attached [···] ───────┐│
│ ▾ repo                       ││ $ pi                                                                ││
│   ▾ issue-248                ││ ...real Herdr terminal output...                                   ││
│     ▸ api-review         !   ││                                                                    ││
│     • docs                   │├────────────────────────────────────────────────────────────────────┤│
│ ▸ side-project              1││ [term] tests-2   ✓ Done                         attached [···]      ││
│                              ││ $ cargo test                                                       ││
│                              ││ ...                                                               ││
│                              │└────────────────────────────────────────────────────────────────────┘│
│                              ├──────────────────────────────────────────────────────────────────────┤
│                              │ focused pane is marked clearly; shared dividers remain unobtrusive    │
│ ! Claude             Blocked│                                                                      │
│   api-review · pane 1    12s │                                                                      │
│ ↻ Codex             Working │                                                                      │
│   api-review · pane 2     3s │                                                                      │
│ ✓ tests                 Done │                                                                      │
│   side-project          48s │                                                                      │
│ · shell                 Idle │                                                                      │
└──────────────────────────────┴──────────────────────────────────────────────────────────────────────┘
        resizable sidebar                                  fluid main area
```

This is a conceptual wireframe, not a prescribed measurement or current layout. The pane lines shown are illustrative Herdr layout, not a default.

## Explicit anti-patterns

- No landing-page hero, welcome marketing panel, gradient glow, glass blur, oversized logo, usage chart, or KPI card.
- No card per Space, agent, tab, or terminal. Rows and regions carry more information per pixel.
- Keep the Agents queue persistent and glanceable rather than turning it into a chat panel or popup.
- No separate inbox popup or Cockpit-managed acknowledgement state.
- No agent avatars or chat bubbles in the attention queue.
- No status encoded by color alone and no unlabeled rainbow-dot matrix.
- No optimistic reordering that temporarily disagrees with Herdr.
- No browser-style close button that implies a disconnected terminal process was killed.
- No custom terminal prompt, command blocks, output summaries, translucent terminal, or terminal text restyling.
- Keep pane headers where needed for focus and resource context. Exact geometry follows current styles; drag targeting is not yet implemented.
- No toast-only failure and no modal for recoverable resource errors.
- No permanent global status bar until the product has global information that cannot live beside its resource.
- No settings cog in the first milestone. Configuration is external by current decision.
- No unsupported disabled controls. Render actions only when the active Herdr schema supports them.
- No motion for live-state churn. Agent transitions change glyph, word, and timestamp without sliding rows around.

## First-screen acceptance checklist

### Structure

- [ ] Session selector, Spaces, Agents, tabs, pane chrome, and terminal content form a coherent first screen without forcing page scrolling at the intended desktop size.
- [ ] Sidebar remains resizable and the main area remains flexible.
- [ ] Spaces and Agents have distinct, usable regions; a draggable divider between them is not yet implemented.
- [ ] Main area renders the selected Space's tabs and Herdr pane layout.
- [ ] Context and Review presentation follows the implemented pane-renderer model rather than assuming first-class tabs.
- [ ] Narrow layouts preserve access to controls and terminal content without whole-page horizontal scrolling.

### Identity and hierarchy

- [ ] The selected session is always visible at the top left.
- [ ] Selected Space, active tab, and focused pane use distinct treatments and can be identified simultaneously.
- [ ] UI labels say Spaces, Agents, tabs, and panes. They do not leak API `workspace` wording.
- [ ] Long names truncate with full identifiers available by tooltip or accessible description.

### Terminal correctness

- [ ] The pane contains a real xterm.js renderer attached to Herdr, with no mocked prompt or rewritten command blocks.
- [ ] Focused xterm receives ordinary input only after writable ownership is confirmed.
- [ ] External ownership loss preserves observation without a reclaim loop; an explicit local action can request control again.
- [ ] Herdr's magic escape key has priority over GUI shortcuts. Not yet implemented.
- [ ] Hidden tabs detach renderers without any stopped-process visual.
- [ ] Fit precedes attachment; verify terminal dimensions and glyph continuity at zoom as requirements, not claims of tested guarantees.
- [ ] Disconnect preserves last rendered output, removes the live-cursor implication, and offers Retry attach and Resync.

### Spaces and Agents

- [ ] Space rows support keyboard navigation, expansion, context actions, and inline rename where available.
- [ ] Drag reparenting and destination drop zones are desired, not yet implemented.
- [ ] Agents appear in status order: blocked, done, working, idle, unknown; newest state change first.
- [ ] Every agent state has an icon, state word, and freshness. Color is supplemental.
- [ ] Clicking an agent requests focus of its owning pane and updates selection only after acknowledgement.

### Visual system

- [ ] Components use semantic color tokens; current values are defined in the styles and terminal implementation.
- [ ] Typography uses shared semantic font tokens and current project font configuration.
- [ ] Separators and focus indicators remain clear and consistent.
- [ ] No primary region is a rounded card and no workbench region uses a shadow.
- [ ] Hover, keyboard focus, pressed, drag, loading, stale, disconnected, and error treatments are visibly distinct.

### Empty, loading, and failure states

- [ ] Initial loading keeps the final frame geometry stable.
- [ ] Resync preserves last-known content and marks it stale.
- [ ] No-session, no-Space, no-tab, no-agent, and no-context states use the local copy and actions specified above.
- [ ] Errors sit on the affected row, pane, file, or session and include a concrete recovery action when one exists.
- [ ] No terminal or process disappears solely because attach, reconnect, resize, focus, or another transport operation failed.
- [ ] Incompatible Herdr schema errors name installed and required versions when available.

### Input and accessibility basics

- [ ] Every interactive item is reachable by keyboard and has a visible `focus-visible` state.
- [ ] Icon-only actions have accessible names and usable hit areas.
- [ ] Text and meaningful icons retain usable contrast against every specified state background.
- [ ] Reduced-motion mode replaces rotating progress with a static progress glyph.
- [ ] The focused terminal, not merely the selected tab, is unmistakably the keyboard destination.
