# Cockpit-owned tab layout: agreed contract

Status: full design and implementation planning paused for interactive refinement. Implementation remains a separate second step. The user has approved the demo's drag/drop, resizing, visual direction and pane control icons as implementation references; this does not establish backend integration.

## Approved implementation reference

- Interactive reference: [`mocks/tab-layout/demo.html`](mocks/tab-layout/demo.html), committed as `6aa0331` (standalone pane layout prototype).
- User feedback: “the drag & drop and resize behavior feels really good”; “it looks very good as well”; “i also like the controll icons.”
- The eventual design, implementation plan and product implementation MUST explicitly reference this demo rather than independently redesigning those approved interactions.
- Preserve header-based dragging, centre-drop swaps, edge-drop placement, destination previews and live divider resizing. Use the demo's pane chrome, selected-state treatment and control icon appearance as the visual reference.
- Compare the implemented surface against the runnable demo for mouse behavior and appearance. Product integration may adapt existing tokens and accessibility behavior without silently replacing the approved interaction model or icon treatment.
- Approval is limited to the stated interaction and visual feedback. Mock content, simulation controls, missing focus preservation and absence of backend integration are not approved product behavior.
- The demo is standalone, with no Herdr or real browser sessions. Full planning remains paused until the user requests it to resume.

## User decisions

1. Herdr owns Spaces, tabs, real terminal pane existence and membership. Cockpit owns placement within each tab and can add local viewer leaves unknown to Herdr.
2. Library is global, outside Space/tab layouts.
3. Browser, Review and Files all become tab-local Cockpit panes. New viewers do not launch addon TUIs. No terminal/UI renderer toggle for these viewers.
4. One browser, one Review and one Files viewer per tab. Opening an existing Files/Review viewer focuses it and switches to the requested source; existing durable drafts retain source identity.
5. Ignore all Herdr positioning hints, including initial geometry. Do not persist layouts yet. Preserve normal in-memory arrangement/viewer state through tab switches during a run; existing durable comments/drafts remain durable.
6. First-load terminal arrangement is a balanced grid starting with two terminals side by side. Use authoritative terminal member order, not Herdr rectangles.
7. Open a new viewer by splitting the selected leaf; right/down placement supported. Existing viewer reuse does not create another split.
8. Externally created terminals insert at the full-height right edge. Newcomer width is 1/(current visible layout leaves + 1), preserving the old layout's internal proportions. Clarify visible means unzoomed layout membership, not only currently painted zoom leaf, in the design.
9. Include draggable dividers, drag-to-centre swap, drag-to-edge repositioning with split-tree restructuring, and zoom/restore. Dragging uses pane headers, not content selections.
10. Real Herdr terminals retain supported moves between tabs/Spaces. Cockpit browser/Review/Files panes cannot move between tabs/Spaces; those actions must not be offered. Within-tab movement works for every leaf.
11. Always follow actual external Herdr focus changes: select its real terminal and owning tab/Space even if a viewer was active. Unchanged focus in routine snapshots must not continually steal selection back from a viewer. Keep semantic focus, attachment control and DOM focus distinct.
12. New terminal/split terminal while a viewer is selected uses that tab's last-selected real terminal as Herdr runtime source, but inserts beside the selected viewer. Define deterministic source fallback if no local real selection exists or source was closed. Associate creations by trustworthy identity, not by current focused pane.
13. User-established behavior: Cockpit control-attaches terminals within a tab; no observer mode. Cockpit dimensions drive shared PTY sizes. Awkward Herdr TUI dimensions during concurrent use are accepted. Do not re-probe to confirm this observation. Hidden/zoomed attachment lifecycle must be designed explicitly.
14. At least one real Herdr pane is required per tab. Confirmed loss of its last real member, whether through close or move, closes all virtual panes. Disconnect/stale state is not membership loss. Normal pane closure preserves durable drafts/comments.
15. Independent managed browser session per Herdr tab. Closing browser pane (including last-real-pane closure) stops that tab's managed session. Reopening starts at configured default URL, not last URL. Closing one browser must not stop another tab's browser. No browser cross-tab move.
16. Existing Reviewr/file-viewer addon panes become ordinary terminal panes. Remove automatic graphical replacement/toggles; do not automatically convert or stop existing addon processes.

## Design defaults that do not require more user questions

Use existing tokens, pane chrome and shortcut conventions. Keep ordinary creation split ratios 50:50 unless repository conventions justify a different value. Preserve unrelated tree ratios on moves. Maintain in-memory state keyed by verified session/server and stable tab identity. Do not create fake Herdr memberships. Last-terminal closure is destructive only to live viewer lifecycle, not durable comments.

## Known evidence and handoff

- `CONTEXT.md:194-238`: terminal attach and current graphical extension replacement/browser ownership.
- `DECISIONS.md:15-19`: current attach resize and tab transition sequencing; existing decisions are not constraints on proposed ownership change.
- `CODE_GUIDE.md`: layout/focus paths and core/host/native ownership.
- `agent://ViewerOwnership/report`: current addon launch/process binding, root authorization and browser Space-sidecar evidence.
- `local://tab-layout-hints.md`: read-only evidence of layout DTO/mutation contracts, no runtime capability claims. Its old recommendations are superseded where this contract differs (ignore initial hints; always follow external focus; no viewer moves; no saved layout).

## Required outputs

- `01-design.md`: exact interaction/state/focus/lifecycle design, examples/mock, acceptance matrix, proposed DECISIONS changes (text only).
- `02-implementation-plan.md`: repository-grounded dependencies, interfaces/data shapes, caller migration and obsolete paths removal, runtime acceptance and technical prerequisites.
- Example mock under `mocks/tab-layout/`; example state/transition fixtures if useful, explicitly design artifacts not product code.
- No builds/lints/tests/formatters mid-flight. No product code edits, configuration edits, changes to active decision/architecture docs, or commits claiming implementation.
