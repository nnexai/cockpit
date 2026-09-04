# Cockpit UI design direction

Current-authority note, 2026-09-04: protocol-22 client-shell behavior in `../DECISIONS.md` overrides historical ownership/renderer details below. The next Context/Review design is `../planning/next-level/08-ui-design.md`: detect real extension panes and replace their renderer, with no extension IPC or separate dock/tab authority. The current implemented workbench remains the baseline; these plans add future behavior.

## Decision

Build Cockpit as a **graphite operations workbench**: one compact control rail on the left and one terminal-first work area on the right. The interface should look native to a Linux developer's daily work, with flat dark surfaces, crisp one-pixel separators, restrained state color, and no decorative dashboard cards.

The first screen is a live Herdr session mirror. It answers three questions without navigation:

1. Which session and Space am I operating in?
2. Which agent needs attention next?
3. Which real terminal owns my keyboard now?

The hierarchy is fixed:

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

- Copy semantics before appearance: resource hierarchy, authoritative focus, attention priority, terminal ownership, shortcuts, and mutation behavior must remain recognizable and correct.
- Use the TUI as a regression oracle when Cockpit has no deliberate alternative. Matching its ordering and branch geometry is preferable to inventing a nearly equivalent convention.
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
| [Warp split panes](https://docs.warp.dev/terminal/windows/split-panes) | A pane has a draggable header, visible active marker, mouse focus, resize, movement between tabs, and keyboard navigation. | Give every pane a 26 px drag target and local action menu. Show explicit drop zones while dragging. Keep the terminal itself conventional, without Warp-style command blocks or rewritten prompts. |
| [VS Code interface](https://code.visualstudio.com/docs/editing/getting-started/userinterface) | VS Code separates the primary sidebar, tabbed editor groups, and movable split regions with compact borders. It supports tree context menus and drag/drop. | Use a resizable primary sidebar and tab strip, but omit the Activity Bar, secondary sidebar, global status bar, and bottom panel. Cockpit has one primary mode and does not need those extra layers. |
| [Zed Project Panel](https://zed.dev/docs/project-panel) | Zed's tree supports keyboard traversal, sticky ancestors, inline rename, context menus, filtering, and clear drag/drop behavior in a compact panel. | Give Spaces a true tree interaction model, sticky section header, inline rename, and precise insertion indicators. Do not turn each Space into a card. |
| [Zed getting started](https://zed.dev/docs/getting-started#panel-layout) | Zed's Agentic layout places agent controls next to the project tree rather than hiding them in a modal. | Keep Agents persistent below Spaces. Cockpit's queue is a glanceable operational list, not a chat panel or popup. |
| [Docker Desktop dashboard](https://docs.docker.com/desktop/use-desktop/) | Docker Desktop combines durable resource navigation, quick search, and persistent integrated terminals. | Borrow durable navigation and resource-local status. Reject its large dashboard rows, illustrations, and top-level product sections because Cockpit should open directly on work. |
| [lazygit primary screenshots](https://github.com/jesseduffield/lazygit#readme) | lazygit fits several operational lists and a detail area into one view through terse labels, strong selection, and almost no ornament. | Favor row density and immediate actions. Retain full words and accessible labels where a TUI can rely on learned single-letter commands. |

## First-screen composition

Design against a 1440 x 900 CSS-pixel reference window. The useful minimum is 1024 x 640. Below the minimum, preserve terminal dimensions by allowing the Spaces tree to collapse to a 48 px icon rail; do not make the entire app horizontally scroll.

### Frame

- Fill the window. No centered container, page gutter, hero, title masthead, or footer.
- Use a two-column grid: a 272 px sidebar and a fluid main area.
- The sidebar resizes from 224 to 360 px. Its default is 19 percent of a 1440 px window, rounded to 272 px.
- The divider is a 1 px rule with a 5 px invisible hit target on both sides. Double-click resets to 272 px.
- Use square corners inside the workbench. A 4 px radius is reserved for menus, tooltips, buttons, and inline notices, never for primary regions.

### Sidebar proportions

The sidebar has three vertical regions:

1. Session selector, fixed at 40 px.
2. Spaces, flexible, with a minimum of 180 px.
3. Agents, initially 34 percent of the remaining sidebar height, clamped to 176 through 320 px.

A 1 px draggable splitter separates Spaces and Agents. Store only the presentation ratio locally. The entities and ordering remain Herdr-owned.

#### Session selector

The selector is the topmost control because changing it replaces all state below it.

- Full-width 32 px button within 4 px vertical and 8 px horizontal insets.
- Leading 14 px server icon, selected session name, optional disconnected/stale suffix, then chevron.
- Truncate the middle of long session names only when the identifier contains path-like separators. Otherwise truncate the end and show the full identifier in a tooltip.
- Opening it shows a 280 px searchable popover. Each row contains session name, attachment state, and Space count if supplied by Herdr. The current session has a checkmark.
- A switch begins only after selection. Keep the old screen visible under a subdued scrim labeled `Switching to <name>...` until the new authoritative snapshot arrives. On failure, remove the scrim, keep the old session visible, and put the error in the popover and connection strip. Never present a half-old, half-new tree.

#### Spaces tree

- Section header: 28 px high, label `SPACES`, visible count at the right, and a `+` action that appears on header hover or keyboard focus.
- Tree rows: 28 px high. Use 8 px left/right padding and 16 px indentation per level.
- A 12 px disclosure chevron owns expand/collapse. Clicking the rest of the row requests Herdr focus.
- Row order: disclosure, 14 px Space icon, label, flexible gap, urgent-state glyph, optional child/agent count.
- The selected Space uses `surface-selected` across the full row plus a 2 px accent inset on the left. Hover alone never adds the accent line.
- Ancestor paths may use `text-secondary`; never dim the selected item's parents below readable contrast.
- Keep the `SPACES` header sticky while the tree scrolls.
- Inline rename replaces only the label region. Enter commits; Escape cancels. A mutation error stays directly below the row in a compact two-line notice.
- Right-click opens actions supported by the active Herdr schema. Unsupported actions are absent, not disabled promises.

Drag behavior must be unambiguous:

- Start after 4 px pointer travel, not on ordinary click.
- While dragging, the source row drops to 55 percent opacity.
- A 2 px horizontal accent line means reorder before/after. A tinted row plus a one-pixel outline means reparent into.
- Auto-expand a collapsed candidate after 650 ms. Auto-scroll within 24 px of the viewport edge.
- On drop, leave the tree in its authoritative order until Herdr confirms. Show a small spinner at the source. On rejection, remove the preview and show the error under the source row.

#### Agents queue

Agents is an attention queue, not a directory and not a chat transcript.

- Section header: 28 px, label `AGENTS`, total count, and collapse chevron.
- Preserve Herdr ordering. Do not locally sort by name, recency, or Cockpit selection.
- Row height is 40 px. First line contains a state glyph, agent label or executable, and short state word. Second line contains Space/tab context on the left and freshness such as `12s` on the right.
- State is never a color-only dot. Use icon plus word: `! Blocked`, `↻ Working`, `✓ Done`, `· Idle`. Use a spinner only for a pending operation, not as the permanent Working icon.
- Clicking a row requests focus of its owning pane and attachment. The highlight moves only when Herdr acknowledges focus.
- Blocked and Done rows may use a 2 px state bar at the left. Working and Idle rely on their icon and label so the queue does not flicker like a monitoring wall.
- Stale state appends `stale` and replaces the freshness value with the last confirmed time. Do not guess a current state.

### Main work area

#### Tab strip

- Height 36 px. It spans the main area above the pane layout.
- Tabs are 36 px high, 112 px preferred width, 72 px minimum, and 220 px maximum.
- Each tab shows a 14 px type icon, label, and close action on hover/focus. A small state mark may appear after the label if a child pane needs attention.
- The active tab has `surface-raised`, primary text, and a 2 px bottom accent. Inactive tabs use no pill or border radius.
- Keep Context as a normal first-class tab labeled `Context`. It belongs to the selected Space and sits after Herdr terminal tabs unless Herdr supplies a different authoritative order.
- When tabs overflow, retain the active tab in view and use horizontal wheel/trackpad scrolling plus an overflow menu. Do not shrink text below 12 px.
- A trailing `+` and split control expose Herdr-supported create/layout actions. Schema-gated actions disappear.

#### Pane layout and chrome

Render the exact Herdr layout. Do not rebalance it for aesthetic symmetry.

- Adjacent panes share one 1 px separator. The resize hit target extends 4 px on each side.
- Every pane has a 26 px header above its content. The header is the drag target and prevents terminal selection from starting a pane move.
- Header order: terminal/process icon, pane label or short ID, process/agent state, flexible gap, attachment status, local overflow menu.
- The focused pane has a 2 px `focus-strong` outline drawn inside its bounds. Its header text becomes primary. Unfocused pane headers use neutral borders, never reduced terminal opacity.
- If a pane becomes too narrow for its header metadata, keep label and status icon; move the rest into the overflow menu.
- Double-clicking the header toggles pane maximize through Herdr if supported. Escape or the visible restore action exits maximize.
- On pane drag, show four directional drop zones inside the hovered pane and a tab-strip target above. The chosen zone uses a 2 px accent outline and 12 percent accent fill. Do not move DOM panes until Herdr confirms the mutation.
- During resize, update the visual divider continuously, throttle Herdr resize requests, and reconcile to the confirmed layout when the drag ends.

#### Terminal treatment

The terminal is content, not a decorative widget.

- xterm.js fills all pane space below the header. Use 8 px horizontal and 6 px vertical internal padding so glyphs do not touch separators.
- Default terminal face: `IBM Plex Mono`, then `Noto Sans Mono`, then `monospace`; 13 px, 1.35 line height, normal weight. Allow platform font smoothing. Do not apply letter spacing.
- Preserve ANSI colors, cursor shape, alternate screen, selection, links, and terminal program mouse handling. Cockpit chrome must not reinterpret prompts or divide output into blocks.
- The terminal background is `terminal-bg`, slightly darker than the workbench. The focused terminal is identified by pane chrome, not a different background.
- Use xterm's cursor for input focus. Add no second fake caret.
- Selection uses `terminal-selection`; inactive selections remain visible but subdued.
- Scrollbars are 8 px overlay tracks and appear on hover, scroll, or keyboard focus. Scrollback comes from Herdr's terminal semantics, not a browser-owned parallel history.
- Do not animate terminal output, fade old lines, blur unfocused panes, or overlay agent prose on output.

#### Context tab

Context is a read-only work surface, not a document editor.

- Use a 232 px companion-file tree and a fluid viewer. The internal splitter resizes from 180 to 360 px.
- Tree rows follow the Spaces density but use file-type icons. Unsafe, binary, executable, and oversized files get a refusal icon and explicit reason.
- A 36 px local toolbar contains breadcrumb, read-only badge, search, refresh, and external-open when allowed.
- Markdown measure is capped at 88 characters, aligned 32 px from the viewer's left edge rather than centered in a broad empty canvas. Body type is 14 px/22 px. Code blocks use the terminal mono family at 13 px/19 px.
- Frontmatter starts collapsed behind `Metadata`, with canonical identifier and freshness summary visible in the header.
- Search opens a 32 px field in the toolbar and sends the narrow core-mediated search operation. Results appear as a compact list with file, line, and one bounded excerpt. Do not imply arbitrary shell access.

## Token-level visual specification

### Typography

Bundle fonts so native and browser builds match:

| Token | Value | Use |
| --- | --- | --- |
| `font-ui` | `"IBM Plex Sans", "Noto Sans", sans-serif` | All workbench chrome |
| `font-mono` | `"IBM Plex Mono", "Noto Sans Mono", monospace` | Terminal, IDs, paths, timestamps, code |
| `text-xs` | 11 px / 16 px, 500 | Section labels, compact metadata |
| `text-sm` | 12 px / 16 px, 400 | Secondary row text, tab metadata |
| `text-ui` | 13 px / 18 px, 400 | Rows, controls, notices |
| `text-ui-strong` | 13 px / 18 px, 600 | Selected labels and headings |
| `text-context` | 14 px / 22 px, 400 | Markdown body |
| `text-terminal` | 13 px / 1.35, 400 | xterm.js default |

Use uppercase only for the two sidebar section labels. Do not uppercase tabs, sessions, states, or errors.

### Spacing and geometry

Use a 4 px base unit.

| Token | Value |
| --- | --- |
| `space-1` | 4 px |
| `space-2` | 8 px |
| `space-3` | 12 px |
| `space-4` | 16 px |
| `space-6` | 24 px |
| `control-compact` | 28 px |
| `control-default` | 32 px |
| `tab-height` | 36 px |
| `agent-row` | 40 px |
| `pane-header` | 26 px |
| `radius-control` | 4 px |
| `border-thin` | 1 px |
| `focus-ring` | 2 px |

No 8, 12, or 16 px rounded cards. No shadows inside the workbench. Menus may use one soft shadow, `0 8px 24px rgba(0,0,0,.32)`.

### Dark palette

Dark is the first direction because real terminal programs and Herdr's primary screenshot are dark-first. These are semantic tokens, not component-local literals.

| Token | Value | Purpose |
| --- | --- | --- |
| `app-bg` | `#0B0E13` | Window root |
| `sidebar-bg` | `#11151C` | Sidebar |
| `surface` | `#151A22` | Pane headers, controls |
| `surface-raised` | `#1B222D` | Active tab, popovers |
| `surface-hover` | `#202936` | Hovered row/control |
| `surface-selected` | `#243144` | Selected Space/Agent |
| `border` | `#2A3340` | Ordinary separators |
| `border-strong` | `#3B4758` | Hovered divider/drop boundary |
| `text-primary` | `#E6EAF0` | Main text |
| `text-secondary` | `#A7B0BE` | Metadata |
| `text-muted` | `#737E8E` | Placeholder/disabled text |
| `accent` | `#70A7FF` | Active tab, selection, links |
| `focus-strong` | `#8DB8FF` | Keyboard/pane focus |
| `blocked` | `#FF6B78` | Blocked/error state |
| `working` | `#E7B14A` | Working state |
| `done` | `#69A7FF` | Done/unseen state |
| `idle` | `#57C78B` | Idle/seen state |
| `warning` | `#E7B14A` | Warning |
| `terminal-bg` | `#0C1016` | xterm background |
| `terminal-fg` | `#D8DEE8` | xterm default foreground |
| `terminal-selection` | `#315A8F99` | xterm selection |
| `scrim` | `#07090DB8` | Session-switch overlay |

State fills use no more than 12 percent opacity. Text and glyphs carry the solid color. `text-muted` is not valid for actionable controls.

Recommended ANSI defaults:

```text
black #1A1F29   red #E86872      green #63BD83   yellow #D8A657
blue  #6E9FDF   magenta #B08AD4  cyan  #5FB8BC   white  #C9D1DC
bright black #596273   bright red #FF7B86   bright green #75D395
bright yellow #EDBC6A bright blue #82B3F4 bright magenta #C29BE7
bright cyan #72CDD0   bright white #F1F4F8
```

Do not remap application-supplied true color. These values only fill the default 16-color palette.

### Icons

Use one outline icon set with a 1.5 px stroke, preferably Lucide, at 14 px in rows and 16 px in standalone buttons. Use filled geometry only for selection markers and status glyphs. Do not mix icon families, use product logos for generic resources, or put every icon in a circle.

Every icon-only control needs a tooltip after 500 ms, an accessible name, and a visible hover/focus area of at least 28 x 28 px. Keep destructive actions out of persistent row chrome; place them in context menus with confirmation where required.

## Interaction states

### Focus and keyboard ownership

Focus needs two visible levels:

- `focus-visible` on ordinary controls uses a 2 px `focus-strong` inset ring with 1 px offset from the component edge.
- Terminal focus uses the focused pane outline plus the real xterm cursor.

A mouse click may set pointer focus without leaving a bright ring on every row, but the active Space, tab, and pane remain visibly selected. The Herdr magic escape key has priority over Cockpit shortcuts. Ordinary keys go to focused xterm. Cockpit must not intercept common terminal chords merely because a matching GUI action exists.

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
- Agents empty: one 40 px row, `No agents need attention`. Do not celebrate inbox zero.
- Context empty: `No context files found for this Space`, then the companion path if safe to display and Refresh.

No illustrations, confetti, mascots, or oversized headings.

### Stale and disconnected

`Stale` means last-known data is visible but not current. `Disconnected` means the event/terminal connection is unavailable. Keep the distinction.

- Stale resource: amber outlined `STALE` badge in its header or row, last confirmed time, and `Resync` action. Content stays visible.
- Global disconnect: a 28 px strip directly below the session selector and across the main tab strip line. Text is `Disconnected from <session>. Last update <time>.` Actions are `Reconnect` and `Details`.
- Disconnected terminal: preserve the last rendered frame at full opacity, stop showing a live cursor, and overlay a bottom 30 px notice, `Terminal stream disconnected`, with `Retry attach` and `Resync`.
- Never close a pane, clear output, or change agent state to Idle because transport failed.

### Inline errors

Errors attach to the resource or action that failed.

- Row mutation error: 32 to 48 px inset notice immediately below the row. Include a short action sentence, stable error code in monospace when available, and `Retry` or `Dismiss`.
- Pane attach/input error: bottom overlay inside that pane. Keep the pane and last terminal frame visible.
- Context file error: viewer notice with file identity, refusal/failure reason, and allowed next action such as Refresh or Open externally.
- Session/schema error: persistent main-area notice because it affects the whole selected session. List the installed and required schema versions when known.
- A toast may echo an error only when the affected resource is offscreen. The inline copy remains the source of explanation and recovery.

Error copy says what failed and what remains safe. Example: `Rename was not applied. Herdr kept the Space as api-review.` Avoid `Something went wrong`.

## Herdr behavior constraints that the visual design must respect

These constraints come from the confirmed architecture in [CONTEXT.md](../CONTEXT.md#53-state-and-interaction), [DECISIONS.md](../DECISIONS.md#terminal-attachment), and Herdr's [direct attach documentation](https://github.com/SuperCodeAgents/herdr-terminal#direct-agent-attach).

1. Herdr owns sessions, Spaces, tabs, pane layout, PTYs, processes, focus, agent state, ordering, and terminal scrollback. Cockpit renders and requests; it does not manufacture a parallel truth.
2. Selecting a Space, tab, pane, or agent sends a Herdr focus operation. Selection chrome follows the acknowledged response or event. A pending click may show progress but not confirmed selection.
3. xterm.js renders Herdr's server-owned terminal. It does not start a replacement PTY. Attach receives current rendered state and then live ANSI frames where supported.
4. Only visible panes in the selected tab keep renderers/subscriptions. A hidden tab detaches its renderer while its Herdr process continues. UI copy must never equate hidden, detached, or disconnected with stopped.
5. One writable client owns terminal input and resize. The initially focused pane and explicit local selection/click may request takeover. After external focus or ownership loss, Cockpit observes without reclaiming until another local action. Never accept typing into a pane that has not confirmed ownership.
6. Herdr's magic escape key wins over GUI shortcuts. Focus styling must make terminal keyboard ownership obvious.
7. A sequence gap, reconnect, or stale cache triggers resnapshot and resubscription. Preserve last-known content with a stale marker until replacement, rather than animating local guesses into place.
8. Attach failure leaves the resource visible with retry and resync. It must not silently close or clear the Herdr process.

## Compact text wireframe

```text
┌──────────────────────────────┬──────────────────────────────────────────────────────────────────────┐
│ [server] default        [⌄]  │ [term] agent  [term] tests  [doc] Context                     [+][▦] │ 36
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
│ AGENTS                  4 [⌃]│  focused pane has 2 px blue inset outline; shared dividers are 1 px   │
│ ! Claude             Blocked│                                                                      │
│   api-review · pane 1    12s │                                                                      │
│ ↻ Codex             Working │                                                                      │
│   api-review · pane 2     3s │                                                                      │
│ ✓ tests                 Done │                                                                      │
│   side-project          48s │                                                                      │
│ · shell                 Idle │                                                                      │
└──────────────────────────────┴──────────────────────────────────────────────────────────────────────┘
        272 px default                                      fluid main area
```

The sidebar has its own Spaces/Agents splitter. The pane lines shown above are Herdr layout, not a prescribed two-row default.

## Explicit anti-patterns

- No landing-page hero, welcome marketing panel, gradient glow, glass blur, oversized logo, usage chart, or KPI card.
- No card per Space, agent, tab, or terminal. Rows and regions carry more information per pixel.
- No Activity Bar clone. Cockpit's first milestone has one primary work mode.
- No separate inbox popup or Cockpit-managed acknowledgement state.
- No agent avatars or chat bubbles in the attention queue.
- No status encoded by color alone and no unlabeled rainbow-dot matrix.
- No optimistic reordering that temporarily disagrees with Herdr.
- No browser-style close button that implies a disconnected terminal process was killed.
- No custom terminal prompt, command blocks, output summaries, translucent terminal, or terminal text restyling.
- No hidden pane headers. A 26 px header is the reliable focus, drag, ownership, and error anchor.
- No toast-only failure and no modal for recoverable resource errors.
- No permanent global status bar until the product has global information that cannot live beside its resource.
- No settings cog in the first milestone. Configuration is external by current decision.
- No unsupported disabled controls. Render actions only when the active Herdr schema supports them.
- No motion for live-state churn. Agent transitions change glyph, word, and timestamp without sliding rows around.

## First-screen acceptance checklist

### Structure

- [ ] At 1440 x 900, session selector, Spaces, Agents, tabs, pane headers, and terminal content are all visible without page scrolling.
- [ ] Sidebar defaults to 272 px and resizes within 224 to 360 px.
- [ ] Spaces gets the flexible upper region; Agents gets a resizable 176 to 320 px lower region.
- [ ] Main area renders the selected Space's authoritative tab order and exact pane layout.
- [ ] Context appears as a first-class read-only tab associated with the selected Space.
- [ ] At 1024 x 640, no control overlaps terminal text and the app does not gain a whole-page horizontal scrollbar.

### Identity and hierarchy

- [ ] The selected session is always visible at the top left.
- [ ] Selected Space, active tab, and focused pane use distinct treatments and can be identified simultaneously.
- [ ] UI labels say Spaces, Agents, tabs, and panes. They do not leak API `workspace` wording.
- [ ] Long names truncate with full identifiers available by tooltip or accessible description.

### Terminal correctness

- [ ] The pane contains a real xterm.js renderer attached to Herdr, with no mocked prompt or rewritten command blocks.
- [ ] Focused xterm receives ordinary input only after writable ownership is confirmed.
- [ ] External ownership loss preserves observation without a reclaim loop; an explicit local action can request control again.
- [ ] Herdr's magic escape key has priority over GUI shortcuts.
- [ ] Hidden tabs detach renderers without any stopped-process visual.
- [ ] Fit and Canvas initialize before attachment, initial dimensions match pane bounds, and box-drawing glyphs remain continuous at zoom.
- [ ] Disconnect preserves last rendered output, removes the live-cursor implication, and offers Retry attach and Resync.

### Spaces and Agents

- [ ] Space rows are 28 px, keyboard navigable, expandable, context-menu capable, and support inline rename.
- [ ] Drag distinguishes reorder from reparent with different previews and waits for Herdr confirmation.
- [ ] Agents remain in Herdr attention order.
- [ ] Every agent state has an icon, state word, and freshness. Color is supplemental.
- [ ] Clicking an agent requests focus of its owning pane and updates selection only after acknowledgement.

### Visual system

- [ ] Components use the semantic color tokens in this report rather than ad hoc hex values.
- [ ] UI uses IBM Plex Sans and terminal/IDs use IBM Plex Mono with the defined fallbacks.
- [ ] Primary region separators are one pixel; focused pane outline is two pixels.
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
- [ ] Icon-only actions have 28 x 28 px hit areas, accessible names, and delayed tooltips.
- [ ] Text and meaningful icons retain usable contrast against every specified state background.
- [ ] Reduced-motion mode replaces rotating progress with a static progress glyph.
- [ ] The focused terminal, not merely the selected tab, is unmistakably the keyboard destination.
